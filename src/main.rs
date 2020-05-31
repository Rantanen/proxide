#![allow(clippy::match_bool)]

use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
use crossterm::{cursor::MoveToPreviousLine, ExecutableCommand};
use log::error;
use snafu::{ResultExt, Snafu};
use std::fs::File;
use std::io::stdout;
use std::io::Read;
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

mod connection;
mod decoders;
mod error;
mod search;
mod session;
mod ui;

use connection::{run, CADetails, ConnectionOptions};
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

    #[snafu(display("Invalid configuration: {}", reason))]
    ConfigurationError
    {
        reason: &'static str
    },
}

fn main() -> Result<(), Error>
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

    // Set up the monitor and view commands separately.
    //
    // Both of these commands should support the decoder options so we'll want to further process
    // them before constructing the clap App.
    let monitor_cmd =
        SubCommand::with_name("monitor").about("Monitor network traffic using the Proxide UI");
    let monitor_cmd = add_connection_options(monitor_cmd);

    let view_cmd = SubCommand::with_name("view")
        .about("View traffic from a session or capture file")
        .arg(
            Arg::with_name("file")
                .value_name("file")
                .required(true)
                .index(1)
                .help("Specify the file to load"),
        );

    let capture_cmd = SubCommand::with_name("capture")
        .about("Capture network traffic into a file for later analysis")
        .arg(
            Arg::with_name("file")
                .short("o")
                .value_name("file")
                .required(true)
                .index(1)
                .help("Specify the output file"),
        );
    let capture_cmd = add_connection_options(capture_cmd);

    let mut app = App::new("Proxide - HTTP2 debugging proxy")
        .version(env!("CARGO_PKG_VERSION"))
        .author("Mikko Rantanen <rantanen@jubjubnest.net>")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommand(capture_cmd);

    // Add the decoder args to the subcommands before adding the subcommands to the app.
    for cmd in vec![monitor_cmd, view_cmd]
        .into_iter()
        .map(decoders::grpc::setup_args)
    {
        app = app.subcommand(cmd);
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
    let matches = app.get_matches();
    let (session, matches) = match matches.subcommand() {
        ("monitor", Some(sub_m)) => {
            // Monitor sets up the network tack.
            let options = ConnectionOptions::resolve(&sub_m)?;
            network_thread = Some(std::thread::spawn(move || {
                tokio_main(options, abort_rx, ui_tx)
            }));
            (Session::default(), sub_m)
        }
        ("capture", Some(sub_m)) => {
            let filename = sub_m.value_of("file").unwrap();

            // Monitor sets up the network tack.
            let options = ConnectionOptions::resolve(&sub_m)?;
            std::thread::spawn(move || tokio_main(options, abort_rx, ui_tx));
            println!("... Waiting for connections.");
            return session::serialization::capture_to_file(ui_rx, abort_tx, &filename, |status| {
                let _ = stdout().execute(MoveToPreviousLine(1));
                println!(
                    "Received {} requests in {} connections. Total of {} bytes of data.",
                    status.requests, status.connections, status.data
                );
            })
            .context(SerializationError {});
        }
        ("view", Some(sub_m)) => {
            let filename = sub_m.value_of("file").unwrap();
            let session =
                session::serialization::read_file(&filename).context(SerializationError {})?;
            (session, sub_m)
        }
        (_, _) => panic!("Sub command not handled!"),
    };

    let mut decoders = vec![];
    decoders.push(decoders::raw::initialize(&matches).context(DecoderError {})?);
    decoders.push(decoders::grpc::initialize(&matches).context(DecoderError {})?);
    let decoders = decoders::Decoders::new(decoders.into_iter().filter_map(|o| o));

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

fn add_connection_options<'a, 'b>(cmd: App<'a, 'b>) -> App<'a, 'b>
{
    cmd.arg(
        Arg::with_name("listen")
            .short("l")
            .value_name("port")
            .required(true)
            .help("Specify listening port")
            .takes_value(true),
    )
    .arg(
        Arg::with_name("target")
            .short("t")
            .value_name("host:port")
            .required(true)
            .help("Specify target host and port")
            .takes_value(true),
    )
    .arg(
        Arg::with_name("ca-certificate")
            .long("ca-certificate")
            .value_name("path")
            .required(false)
            .help("Specify the CA certificate used to sign the generated TLS certificates")
            .takes_value(true),
    )
    .arg(
        Arg::with_name("ca-key")
            .long("ca-key")
            .value_name("path")
            .required(false)
            .help("Specify the CA private key used to sign the generated TLS certificates")
            .takes_value(true),
    )
}

impl ConnectionOptions
{
    fn resolve(args: &ArgMatches) -> Result<Arc<Self>, Error>
    {
        let cert_details = match (args.value_of("ca-certificate"), args.value_of("ca-key")) {
            (None, None) => None,
            (Some(cert), Some(key)) => Some((cert, key)),
            _ => {
                return Err(Error::ConfigurationError {
                    reason: "ca-certificate and ca-key options must be used together",
                })
            }
        };

        let ca_details = match cert_details {
            None => None,
            Some((cert, key)) => {
                let mut cert_data = String::new();
                let mut key_data = String::new();
                File::open(&cert)
                    .and_then(|mut file| file.read_to_string(&mut cert_data))
                    .map_err(|_| Error::ConfigurationError {
                        reason: "Could not read CA certificate",
                    })?;
                File::open(&key)
                    .and_then(|mut file| file.read_to_string(&mut key_data))
                    .map_err(|_| Error::ConfigurationError {
                        reason: "Could not read CA key",
                    })?;
                Some(CADetails {
                    certificate: cert_data,
                    key: key_data,
                })
            }
        };

        Ok(Arc::new(Self {
            listen_port: args.value_of("listen").unwrap().to_string(),
            target_server: args.value_of("target").unwrap().to_string(),
            ca: ca_details,
        }))
    }
}

#[tokio::main]
async fn tokio_main(
    options: Arc<ConnectionOptions>,
    mut abort_rx: oneshot::Receiver<()>,
    ui_tx: Sender<session::events::SessionEvent>,
) -> Result<(), Error>
{
    // We'll want to liten for both IPv4 and IPv6. These days 'localhost' will first resolve to the
    // IPv6 address if that is available. If we did not bind to it, all the connections would first
    // need to timeout there before the Ipv4 would be attempted as a fallback.
    let mut listener_ipv4 = TcpListener::bind(format!("0.0.0.0:{}", &options.listen_port))
        .await
        .unwrap();
    let mut listener_ipv6 = TcpListener::bind(format!("[::1]:{}", &options.listen_port))
        .await
        .unwrap();

    // Loop until the abort signal is received.
    loop {
        tokio::select! {
            _ = &mut abort_rx => {
                log::info!("tokio_main done");
                break Ok(());
            },
            result = listener_ipv4.accept() => new_connection(ui_tx.clone(), result, options.clone()),
            result = listener_ipv6.accept() => new_connection(ui_tx.clone(), result, options.clone()),
        }
    }
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
        let options = options.clone();
        tokio::spawn(async move {
            match run(socket, src_addr, options, tx).await {
                Ok(..) => {}
                Err(e) => error!("Connection error\n{}", e),
            }
        });
    }
}
