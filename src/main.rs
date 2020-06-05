#![allow(clippy::match_bool)]

use clap::ArgMatches;
use crossterm::{cursor::MoveToPreviousLine, ExecutableCommand};
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

    let app = command_line::setup_app();

    // Parse the command line argument and handle the simple arguments that don't require Proxide
    // to set up the complex bits. Anything handled here should `return` out of the function to
    // prevent the more complex bits from being performed.
    let matches = app.get_matches();
    match matches.subcommand() {
        ("config", Some(matches)) => return config::run(matches),
        ("view", Some(matches)) if matches.is_present("json") => return json::view(matches),
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
                    let _ = stdout().execute(MoveToPreviousLine(1));
                    println!(
                        "Received {} requests in {} connections. Total of {} bytes of data.",
                        status.requests, status.connections, status.data
                    );
                },
            };

            // Monitor sets up the network tack.
            let options = ConnectionOptions::resolve(&sub_m)?;
            std::thread::spawn(move || tokio_main(options, abort_rx, ui_tx));
            if !stdout_data {
                println!("... Waiting for connections.");
            }
            return session::serialization::capture_to_file(
                ui_rx, abort_tx, &filename, format, status_cb,
            )
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

    let decoders = decoders::get_decoders(&matches).context(DecoderError {})?;

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

        Ok(Arc::new(Self {
            listen_port: args.value_of("listen").unwrap().to_string(),
            target_server: args.value_of("target").unwrap().to_string(),
            ca: ca_details,
        }))
    }

    fn read_cert(args: &ArgMatches) -> Result<Option<CADetails>, Error>
    {
        let (cert, key) = match (args.value_of("ca-certificate"), args.value_of("ca-key")) {
            (None, None) => return Ok(None),
            (Some(cert), Some(key)) => (cert, key),
            _ => unreachable!("Clap let ca-certificate or ca-key through without the other"),
        };

        // Handle the case where the user didn't explicilty require the CA
        // certificates and the default ones don't exist.
        if (!Path::new(cert).is_file() || !Path::new(key).is_file())
            && (args.occurrences_of("ca-certificate") == 0 && args.occurrences_of("ca-key") == 0)
        {
            return Ok(None);
        }

        let mut cert_data = String::new();
        let mut key_data = String::new();
        File::open(&cert)
            .and_then(|mut file| file.read_to_string(&mut cert_data))
            .map_err(|_| Error::ArgumentError {
                msg: format!("Could not read CA certificate: '{}'", cert),
            })?;
        File::open(&key)
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

#[tokio::main]
async fn tokio_main(
    options: Arc<ConnectionOptions>,
    abort_rx: oneshot::Receiver<()>,
    ui_tx: Sender<session::events::SessionEvent>,
) -> Result<(), Error>
{
    // We'll want to listen for both IPv4 and IPv6. These days 'localhost' will first resolve to the
    // IPv6 address if that is available. If we did not bind to it, all the connections would first
    // need to timeout there before the Ipv4 would be attempted as a fallback.
    let listener_ipv4 = TcpListener::bind(format!("0.0.0.0:{}", &options.listen_port))
        .await
        .ok();
    let listener_ipv6 = TcpListener::bind(format!("[::1]:{}", &options.listen_port))
        .await
        .ok();

    // Ensure we bound at least one of the sockets.
    if listener_ipv4.is_none() && listener_ipv6.is_none() {
        return Err(Error::RuntimeError {
            msg: "Could not bind to either IPv4 or IPv6 address".to_string(),
        });
    }

    // Start the accept-tasks.
    match listener_ipv4 {
        None => ui::toast::show_error("Could not bind to IPv4"),
        Some(listener) => spawn_accept(listener, options.clone(), ui_tx.clone()),
    }
    match listener_ipv6 {
        None => ui::toast::show_error("Could not bind to IPv6"),
        Some(listener) => spawn_accept(listener, options.clone(), ui_tx.clone()),
    }

    // Wait for an abort event to quit the thread.
    //
    // Once the tokio_main exits, the main program will exit. The spawned tasks
    // won't keep the process alive.
    let _ = abort_rx.await;
    log::info!("tokio_main done");
    Ok(())
}

fn spawn_accept(
    mut listener: TcpListener,
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
