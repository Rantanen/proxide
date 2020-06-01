#![allow(clippy::match_bool)]

use clap::ArgMatches;
use crossterm::{cursor::MoveToPreviousLine, ExecutableCommand};
use log::error;
use snafu::{ResultExt, Snafu};
use std::fs::File;
use std::io::stdout;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

mod command_line;
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

    let app = command_line::setup_app(&[decoders::grpc::setup_args]);

    // Parse the command line argument and handle the simple arguments that don't require Proxide
    // to set up the complex bits. Anything handled here should `return` out of the function to
    // prevent the more complex bits from being performed.
    let matches = app.get_matches();
    match matches.subcommand() {
        ("config", Some(matches)) => return do_config(matches),
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

impl ConnectionOptions
{
    fn resolve(args: &ArgMatches) -> Result<Arc<Self>, Error>
    {
        let cert_details = match (args.value_of("ca-certificate"), args.value_of("ca-key")) {
            (None, None) => None,
            (Some(cert), Some(key)) => Some((cert, key)),
            _ => unreachable!("Clap let ca-certificate or ca-key through without the other"),
        };

        let ca_details = match cert_details {
            None => None,
            Some((cert, key)) => {
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

fn do_config(matches: &ArgMatches) -> Result<(), Error>
{
    const CERT_COMMON_NAME: &str = "UNSAFE Proxide Root Certificate";

    match matches.subcommand() {
        ("ca", Some(matches)) => {
            // Handle revoke first.
            if matches.is_present("revoke") {
                std::process::Command::new("certutil")
                    .arg("-delstore")
                    .arg("-user")
                    .arg("Root")
                    .arg(CERT_COMMON_NAME)
                    .spawn()
                    .and_then(|mut process| process.wait())
                    .map_err(|e| Error::RuntimeError {
                        msg: format!("Failed to revoke the certificates with certutil: {}", e),
                    })?;
            }

            // If 'revoke' was the only command, we'll interrupt here.
            if !(matches.is_present("create") || matches.is_present("trust")) {
                return Ok(());
            }

            let cert_file = matches
                .value_of("ca-certificate")
                .unwrap_or_else(|| "proxide_ca.crt");
            let key_file = matches
                .value_of("ca-key")
                .unwrap_or_else(|| "proxide_ca.key");

            if matches.is_present("create") {
                // If the user didn't specify --force we'll need to ensure we are not overwriting
                // any existing files during create.
                if !matches.is_present("force") {
                    for file in &[cert_file, key_file] {
                        if Path::new(file).is_file() {
                            return Err(Error::ArgumentError {
                                msg: format!(
                                    "File '{}' already exists. Use --force to overwrite it.",
                                    file
                                ),
                            });
                        }
                    }
                }

                // Set up the rcgen certificate parameters for the new certificate.
                //
                // Note that at least on Windows the common name is used to later find and revoke
                // the certificate so it shouldn't be changed without a good reason. If it's
                // changed here, it would be best if new versions of Proxide still supported the
                // old names in the 'revoke' command.
                let mut ca_params = rcgen::CertificateParams::new(vec![]);
                ca_params.distinguished_name = rcgen::DistinguishedName::new();
                ca_params
                    .distinguished_name
                    .push(rcgen::DnType::OrganizationName, "UNSAFE");
                ca_params
                    .distinguished_name
                    .push(rcgen::DnType::CommonName, "UNSAFE Proxide Root Certificate"); // See the comment above.
                let ca_cert = rcgen::Certificate::from_params(ca_params).unwrap();

                File::create(cert_file)
                    .map_err(|_| Error::ArgumentError {
                        msg: format!(
                            "Could not open the certificate file '{}' for writing",
                            cert_file
                        ),
                    })?
                    .write_all(ca_cert.serialize_pem().unwrap().as_bytes())
                    .map_err(|_| Error::ArgumentError {
                        msg: format!("Could not write certificate to '{}'", cert_file),
                    })?;
                File::create(key_file)
                    .map_err(|_| Error::ArgumentError {
                        msg: format!(
                            "Could not open the private key file '{}' for writing",
                            key_file
                        ),
                    })?
                    .write_all(ca_cert.serialize_private_key_pem().as_bytes())
                    .map_err(|_| Error::ArgumentError {
                        msg: format!("Could not write private key to '{}'", key_file),
                    })?;
            }

            // Technically if all the user wanted to do was '--create' we wouldn't really need to
            // do this check, but it doesn't really hurt either, unless you count the extra disk
            // access (which I don't!).
            for file in &[cert_file, key_file] {
                if !Path::new(file).is_file() {
                    return Err(Error::ArgumentError {
                        msg: format!("Could not open '{}', use --create if you need to create a new CA certificate", file),
                    });
                }
            }

            // Trust the certificate if the user asked for that.
            if matches.is_present("trust") {
                std::process::Command::new("certutil")
                    .arg("-addstore")
                    .arg("-user")
                    .arg("Root")
                    .arg(cert_file)
                    .spawn()
                    .and_then(|mut process| process.wait())
                    .map_err(|e| Error::RuntimeError {
                        msg: format!("Failed to import the certificate with certutil: {}", e),
                    })?;
            }
        }
        (cmd, _) => unreachable!("Unknown command: {}", cmd),
    }

    Ok(())
}
