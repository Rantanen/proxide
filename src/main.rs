#![allow(clippy::match_bool)]
#![allow(clippy::match_like_matches_macro)]

use clap::ArgMatches;
use crossterm::{
    cursor,
    terminal::{Clear, ClearType},
    ExecutableCommand,
};
use log::error;
use snafu::{ResultExt, Snafu};
use std::fs::File;
use std::io::stdout;
use std::io::Read;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

mod command_line;
mod config;
mod connection;
mod decoders;
mod error;
mod json;
mod search;
mod session;
mod ui;

use connection::run;
use session::Session;

#[derive(Debug, Snafu)]
pub enum Error
{
    #[snafu(display("{}", source))]
    UiError
    {
        source: ui::Error
    },

    #[snafu(display("{}", source))]
    DecoderError
    {
        source: decoders::Error
    },

    #[snafu(display("{}", source))]
    SerializationError
    {
        source: session::serialization::SerializationError,
    },

    #[snafu(display("{}", msg))]
    ArgumentError
    {
        msg: String
    },

    #[snafu(display("{}", msg))]
    RuntimeError
    {
        msg: String
    },
}

pub struct ConnectionOptions
{
    pub allow_remote: bool,
    pub listen_port: String,
    pub target_server: Option<String>,
    pub proxy: Option<Vec<ProxyFilter>>,
    pub ca: Option<CADetails>,
}

pub struct CADetails
{
    pub certificate: String,
    pub key: String,
}

pub struct ProxyFilter
{
    pub host_filter: wildmatch::WildMatch,
    pub port_filter: Option<std::num::NonZeroU16>,
}

fn main()
{
    match proxide_main() {
        Ok(_) => (),
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1)
        }
    }
}

fn proxide_main() -> Result<(), Error>
{
    #[cfg(debug_assertions)]
    {
        simplelog::WriteLogger::init(
            simplelog::LevelFilter::Trace,
            simplelog::ConfigBuilder::new()
                .add_filter_allow("proxide".to_string())
                .build(),
            std::fs::File::create("trace.log").unwrap(),
        )
        .unwrap();
    }

    let commit = option_env!("GITHUB_SHA")
        .map(|c| &c[..7])
        .unwrap_or("dev build");
    let version = format!("{} ({})", env!("CARGO_PKG_VERSION"), commit);
    let app = command_line::setup_app(&version);

    // Parse the command line argument and handle the simple arguments that don't require Proxide
    // to set up the complex bits. Anything handled here should `return` out of the function to
    // prevent the more complex bits from being performed.
    let matches = app.get_matches();
    match matches.subcommand() {
        Some(("config", matches)) => return config::run(matches),
        Some(("view", matches)) if matches.is_present("json") => return json::view(matches),
        _ => (), // Ignore other subcommands for now.
    }

    // We'll have the channels present all the time to simplify setup.  The parameters are free to
    // use them if they want.
    let (abort_tx, abort_rx) = oneshot::channel::<()>();
    let (ui_tx, ui_rx) = std::sync::mpsc::channel();

    // We have the slot for the network thread available always so we can
    // check at the end whether we should join on it.
    let mut network_thread = None;

    // Process the subcommands.
    //
    // The subcommands are responsible for figuring out how the initial session is constructed as
    // well as for giving us back the argument matches so we can initialize the decoders with them.
    let (session, matches) = match matches.subcommand() {
        Some(("monitor", sub_m)) => {
            // Monitor sets up the network tack.
            let options = ConnectionOptions::resolve(sub_m)?;
            network_thread = Some(std::thread::spawn(move || {
                tokio_main(options, abort_rx, ui_tx)
            }));
            (Session::default(), sub_m)
        }
        Some(("capture", sub_m)) => {
            let filename = sub_m.value_of("file").map(String::from).unwrap_or_else(|| {
                format!(
                    "capture-{}.bin",
                    chrono::Local::now().format("%Y-%m-%d_%H%M%S")
                )
            });
            let format = match sub_m.is_present("json") {
                true => session::serialization::OutputFormat::Json,
                false => session::serialization::OutputFormat::MessagePack,
            };

            let stdout_data = filename == "-";
            // If the user is writing the output data to stdout, we don't want to clobber that with
            // status updates.
            let status_cb: fn(&session::serialization::CaptureStatus) = match stdout_data {
                true => |_| (),
                false => |status| {
                    let _ = stdout().execute(cursor::Hide);
                    let _ = stdout().execute(cursor::MoveToPreviousLine(3));

                    print!(
                        "Connections: {} ({} active)",
                        status.connections, status.active_connections
                    );
                    let _ = stdout().execute(Clear(ClearType::UntilNewLine));
                    println!();

                    print!(
                        "Requests:    {} ({} active)",
                        status.requests, status.active_requests
                    );
                    let _ = stdout().execute(Clear(ClearType::UntilNewLine));
                    println!();

                    print!("Total of {} bytes of data.", status.data);
                    let _ = stdout().execute(Clear(ClearType::UntilNewLine));
                    println!();

                    let _ = stdout().execute(cursor::Show);
                },
            };

            // Monitor sets up the network tack.
            let options = ConnectionOptions::resolve(sub_m)?;
            std::thread::spawn(move || tokio_main(options, abort_rx, ui_tx));
            if !stdout_data {
                println!("Capturing to file: {}...", filename);
                println!("\n... Waiting for connections.\n\n");
            }
            return session::serialization::capture_to_file(
                ui_rx, abort_tx, &filename, format, status_cb,
            )
            .context(SerializationError {});
        }
        Some(("view", sub_m)) => {
            let filename = sub_m.value_of("file").unwrap();
            let session =
                session::serialization::read_file(&filename).context(SerializationError {})?;
            (session, sub_m)
        }
        _ => panic!("Sub command not handled!"),
    };

    let decoders = decoders::get_decoders(matches).context(DecoderError {})?;

    // Run the UI on the current thread.
    //
    // This function returns once the user has indicated they want to quit the app in the UI.
    ui::main(session, decoders, ui_rx).context(UiError {})?;

    // Abort the network thread.
    abort_tx.send(()).unwrap();
    if let Some(join_handle) = network_thread {
        join_handle.join().unwrap()?
    }

    Ok(())
}

impl ConnectionOptions
{
    fn resolve(args: &ArgMatches) -> Result<Arc<Self>, Error>
    {
        let ca_details = Self::read_cert(args)?;

        let target_server = args.value_of("target").map(ToString::to_string);
        let mut proxy = match args.value_of("proxy") {
            Some(p) => Some(ProxyFilter::parse(p)?),
            None => None,
        };

        if target_server.is_none() && proxy.is_none() {
            proxy = Some(vec![]);
        }

        Ok(Arc::new(Self {
            allow_remote: args.is_present("allow-remote"),
            listen_port: args.value_of("listen").unwrap().to_string(),
            ca: ca_details,
            target_server,
            proxy,
        }))
    }

    fn read_cert(args: &ArgMatches) -> Result<Option<CADetails>, Error>
    {
        let cert = args.value_of("ca-certificate").unwrap_or("proxide_ca.crt");
        let key = args.value_of("ca-key").unwrap_or("proxide_ca.key");

        // Handle the case where the user didn't explicilty require the CA
        // certificates and the default ones don't exist.
        if (!Path::new(cert).is_file() || !Path::new(key).is_file())
            && (args.occurrences_of("ca-certificate") == 0 && args.occurrences_of("ca-key") == 0)
        {
            return Ok(None);
        }

        let mut cert_data = String::new();
        let mut key_data = String::new();
        File::open(cert)
            .and_then(|mut file| file.read_to_string(&mut cert_data))
            .map_err(|_| Error::ArgumentError {
                msg: format!("Could not read CA certificate: '{}'", cert),
            })?;
        File::open(key)
            .and_then(|mut file| file.read_to_string(&mut key_data))
            .map_err(|_| Error::ArgumentError {
                msg: format!("Could not read CA private key: '{}'", key),
            })?;
        Ok(Some(CADetails {
            certificate: cert_data,
            key: key_data,
        }))
    }
}

impl ProxyFilter
{
    fn parse(data: &str) -> Result<Vec<ProxyFilter>, Error>
    {
        data.split(',')
            .map(|part| {
                // Split into parts and process the host and port separately.
                let mut split = part.split(':');
                let host = split.next().ok_or_else(|| Error::ArgumentError {
                    msg: format!("Invalid proxy filter '{}'", part),
                })?;

                // The port is optional.
                let port = split
                    .next()
                    .map(|p| p.parse::<u16>())
                    .transpose()
                    .map_err(|_| Error::ArgumentError {
                        msg: format!("Invalid proxy filter '{}'", part),
                    })?
                    .and_then(std::num::NonZeroU16::new);

                // There should be no more data after the port.
                if split.next().is_some() {
                    return Err(Error::ArgumentError {
                        msg: format!("Invalid proxy filter '{}'", part),
                    });
                }

                Ok(ProxyFilter {
                    host_filter: wildmatch::WildMatch::new(host),
                    port_filter: port,
                })
            })
            .collect()
    }
}

#[tokio::main]
async fn tokio_main(
    options: Arc<ConnectionOptions>,
    abort_rx: oneshot::Receiver<()>,
    ui_tx: Sender<session::events::SessionEvent>,
) -> Result<(), Error>
{
    launch_proxide(options, abort_rx, ui_tx).await?;
    Ok(())
}

async fn launch_proxide(
    options: Arc<ConnectionOptions>,
    abort_rx: oneshot::Receiver<()>,
    ui_tx: Sender<session::events::SessionEvent>,
) -> Result<(), Error>
{
    // We'll want to listen for both IPv4 and IPv6. These days 'localhost' will first resolve to the
    // IPv6 address if that is available. If we did not bind to it, all the connections would first
    // need to timeout there before the Ipv4 would be attempted as a fallback.
    let addresses = match options.allow_remote {
        true => vec!["0.0.0.0", "[::]"],
        false => vec!["127.0.0.1", "[::1]"],
    };

    let mut sockets: Vec<_> = Vec::new();
    for addr in addresses {
        let addr = format!("{}:{}", addr, &options.listen_port);
        match TcpListener::bind(&addr).await {
            Err(_) => log::error!("Could not bind to {}", addr),
            Ok(s) => sockets.push(s),
        }
    }

    // Ensure we bound at least one of the sockets.
    if sockets.is_empty() {
        return Err(Error::RuntimeError {
            msg: "Could not bind to either IPv4 or IPv6 address".to_string(),
        });
    }

    for s in sockets {
        spawn_accept(s, options.clone(), ui_tx.clone())
    }

    // Wait for an abort event to quit the thread.
    //
    // Once the tokio_main exits, the main program will exit. The spawned tasks
    // won't keep the process alive (unless they block, which would be a bug!)
    let _ = abort_rx.await;
    log::info!("tokio_main done");
    Ok(())
}

fn spawn_accept(
    listener: TcpListener,
    options: Arc<ConnectionOptions>,
    ui_tx: Sender<session::events::SessionEvent>,
)
{
    tokio::spawn(async move {
        loop {
            let ui_tx = ui_tx.clone();
            new_connection(ui_tx, listener.accept().await, options.clone());
        }
    });
}

fn new_connection(
    tx: Sender<session::events::SessionEvent>,
    result: Result<(TcpStream, SocketAddr), std::io::Error>,
    options: Arc<ConnectionOptions>,
)
{
    // Process the new connection by spawning a new tokio task. This allows the original task to
    // process more connections.
    if let Ok((socket, src_addr)) = result {
        tokio::spawn(async move {
            match run(socket, src_addr, options, tx).await {
                Ok(..) => {}
                Err(e) => error!("Connection error\n{}", e),
            }
        });
    }
}

#[cfg(test)]
mod test
{
    use grpc_tester::server::GrpcServer;
    use lazy_static::lazy_static;
    use log::SetLoggerError;
    use serial_test::serial;
    use std::io::{ErrorKind, Write};
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::broadcast::error::TryRecvError;
    use tokio::sync::broadcast::Receiver;
    use tokio::sync::mpsc::UnboundedReceiver;
    use tokio::sync::oneshot;

    use crate::session::events::SessionEvent;
    use crate::ConnectionOptions;

    lazy_static! {

        // Logging must be enabled to detect errors inside proxide.
        // Failure to monitor logs may cause the test to hang as errors that stop processing get silently ignored.
        // It is only possible to initialize one logger => the logger is shared between the tests =>
        // the tests are execute sequentially.
        //
        // Call get_error_monitor to access the monitor inside a test.
        // It drains the monitor from messages ensuring it has room for errors.
        static ref ERROR_MONITOR: Mutex<Receiver<String>> = create_error_monitor().expect( "Initializing error monitoring failed.");
    }

    #[tokio::test]
    #[serial]
    async fn proxide_captures_messages()
    {
        // Logging must be enabled to detect errors inside proxide.
        // Failure to monitor logs may cause the test to hang as errors that stop processing get silently ignored.
        let mut error_monitor = get_error_monitor().expect("Acquiring error monitor failed.");

        // Server
        let server = GrpcServer::start()
            .await
            .expect("Starting test server failed.");

        // Proxide
        let options = get_proxide_options(&server);
        let (abort_tx, abort_rx) = tokio::sync::oneshot::channel::<()>();
        let (ui_tx, ui_rx_std) = std::sync::mpsc::channel();
        let proxide_port = u16::from_str(&options.listen_port.to_string()).unwrap();
        let proxide = tokio::spawn(crate::launch_proxide(options, abort_rx, ui_tx));

        // Message generator and tester.
        let tester = grpc_tester::GrpcTester::with_proxide(
            server,
            proxide_port,
            grpc_tester::Args {
                period: std::time::Duration::from_secs(0),
                tasks: 1,
            },
        )
        .await
        .expect("Starting tester failed.");
        let mut message_rx = async_from_sync(ui_rx_std);
        tokio::select! {
            _result = message_rx.recv() => {},
            result = error_monitor.recv() => panic!( "{:?}", result ),
        }
        let mut server = tester.stop_generator().expect("Stopping generator failed.");
        abort_tx.send(()).expect("Stopping proxide failed.");
        proxide
            .await
            .expect("Waiting for proxide to stop failed.")
            .expect("Waiting for proxide to stop failed.");
        server.stop().expect("Stopping server failed");
    }

    #[tokio::test]
    #[serial]
    async fn error_monitor_catches_errors()
    {
        // Logging must be enabled to detect errors inside proxide.
        // Failure to monitor logs may cause the test to hang as errors that stop processing get silently ignored.
        let mut error_monitor = get_error_monitor().expect("Acquiring error monitor failed.");

        // Proxide
        let options = ConnectionOptions {
            ca: None,
            allow_remote: false,
            listen_port: portpicker::pick_unused_port()
                .expect("Getting free port for proxide failed.")
                .to_string(),
            target_server: Some("Invalid address".to_string()),
            proxy: None,
        };
        let (abort_tx, abort_rx) = oneshot::channel::<()>();
        let (ui_tx, _) = std::sync::mpsc::channel();
        let proxide_port = u16::from_str(&options.listen_port.to_string()).unwrap();
        let proxide = tokio::spawn(crate::launch_proxide(Arc::new(options), abort_rx, ui_tx));

        // Request proxide to connect to the dummy server. This triggers the expected failure.
        let mut generator =
            grpc_tester::generator::GrpcGenerator::start(grpc_tester::generator::Args {
                address: format!("http://[::1]:{}", proxide_port),
                period: Duration::from_secs(0),
                tasks: 1,
            })
            .await
            .expect("Starting the genrator failed.");

        // The invalid address given as the target server's address should trigger an error within the proxide proxy.
        tokio::select! {
            _result = tokio::time::sleep( Duration::from_secs(30)) => panic!("Error monitor did not receive errors."),
            _result = error_monitor.recv() => {},
        }
        abort_tx.send(()).expect("Stopping proxide failed.");
        generator.stop().expect("Stopping the generator failed.");
        proxide
            .await
            .expect("Waiting for proxide to stop failed.")
            .expect("Waiting for proxide to stop failed.");
    }

    /// Gets options for launching proxide.
    fn get_proxide_options(server: &GrpcServer) -> Arc<ConnectionOptions>
    {
        let options = ConnectionOptions {
            ca: None,
            allow_remote: false,
            listen_port: portpicker::pick_unused_port()
                .expect("Getting free port for proxide failed.")
                .to_string(),
            target_server: Some(server.address().to_string()),
            proxy: None,
        };
        Arc::new(options)
    }

    /// Converts a synchronous channel into an asynchronous channel with a helper thread.
    fn async_from_sync(
        sync_received: std::sync::mpsc::Receiver<SessionEvent>,
    ) -> UnboundedReceiver<SessionEvent>
    {
        let (async_tx, async_rx) = tokio::sync::mpsc::unbounded_channel();
        std::thread::spawn(
            move || -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
                loop {
                    async_tx.send(sync_received.recv()?)?;
                }
            },
        );
        async_rx
    }

    /// Gets an error monitor for a test.
    fn get_error_monitor() -> Result<Receiver<String>, Box<dyn std::error::Error>>
    {
        // Gets a clean monitor for the tests.
        // The monitor is drained to guarantee the error monitor channel is empty before the tests.
        // Re-subscribe would not work as it doesn't remove the existing messages from the channel.
        let mut monitor = ERROR_MONITOR.lock().unwrap();
        loop {
            match monitor.try_recv() {
                Ok(_) => {}
                Err(TryRecvError::Empty) => {
                    break;
                }
                Err(e) => return Err(Box::from(e)),
            }
        }
        Ok(monitor.resubscribe())
    }

    /// Initializes logging for the tests.
    fn create_error_monitor() -> Result<Mutex<Receiver<String>>, SetLoggerError>
    {
        let (log_tx, log_rx) = tokio::sync::broadcast::channel(256);
        simplelog::WriteLogger::init(
            simplelog::LevelFilter::Error,
            simplelog::ConfigBuilder::new().build(),
            ChannelLogger {
                target: log_tx,
                data: Vec::new(),
            },
        )?;
        Ok(Mutex::new(log_rx))
    }

    /// A logger that sends the log messages into a channel.
    struct ChannelLogger
    {
        /// Target channel for sending log messages.
        target: tokio::sync::broadcast::Sender<String>,

        /// Data buffer for the error messages.
        data: Vec<u8>,
    }

    impl Write for ChannelLogger
    {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize>
        {
            // Split the log at line breaks.
            if let Some(terminator) = buf.iter().position(|&b| b == 0 || b == 0x0a) {
                let (current, _) = buf.split_at(terminator);
                self.data.extend_from_slice(current);
                let log_entry = String::from_utf8_lossy(&self.data).to_string();
                self.data.clear();
                match self.target.send(log_entry) {
                    Ok(_) => Ok(current.len() + 1),
                    Err(_) => Err(std::io::Error::from(ErrorKind::BrokenPipe)),
                }
            } else {
                self.data.extend_from_slice(buf);
                Ok(buf.len())
            }
        }

        fn flush(&mut self) -> std::io::Result<()>
        {
            let log = String::from_utf8_lossy(self.data.as_slice()).to_string();
            self.data.clear();
            match self.target.send(log.to_string()) {
                Ok(_) => Ok(()),
                Err(_) => Err(std::io::Error::from(ErrorKind::BrokenPipe)),
            }
        }
    }
}
