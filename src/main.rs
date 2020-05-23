use clap::{App, AppSettings, Arg, SubCommand};
use log::error;
use snafu::{ResultExt, Snafu};
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

mod connection;
mod decoders;
mod error;
mod session;
mod ui;

use connection::ProxyConnection;
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

    #[snafu(display("Could not read file '{}': {}", file, source))]
    FileReadError
    {
        file: String,
        source: Box<dyn std::error::Error + Send>,
    },
}

fn main() -> Result<(), Error>
{
    simplelog::WriteLogger::init(
        simplelog::LevelFilter::Trace,
        simplelog::ConfigBuilder::new()
            .add_filter_allow("proxide".to_string())
            .build(),
        std::fs::File::create("trace.log").unwrap(),
    )
    .unwrap();

    // Set up the monitor and view commands separately.
    //
    // Both of these commands should support the decoder options so we'll want to further process
    // them before constructing the clap App.
    let monitor_cmd = SubCommand::with_name("monitor")
        .about("Set up Proxide to monitor network traffic")
        .arg(
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
                .value_name("port")
                .required(true)
                .help("Specify target port")
                .takes_value(true),
        );

    let view_cmd = SubCommand::with_name("view")
        .about("View existing session")
        .arg(
            Arg::with_name("file")
                .value_name("file")
                .required(true)
                .index(1)
                .help("Specify the file to load"),
        );

    let mut app = App::new("Proxide - HTTP2 debugging proxy")
        .version(env!("CARGO_PKG_VERSION"))
        .author("Mikko Rantanen <rantanen@jubjubnest.net>")
        .setting(AppSettings::SubcommandRequiredElseHelp);

    // Add the decoder args to the subcommands before adding the subcommands to the app.
    for cmd in vec![monitor_cmd, view_cmd]
        .into_iter()
        .map(|cmd| decoders::grpc::setup_args(cmd))
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
            let listen_port = sub_m.value_of("listen").unwrap().to_string();
            let target_server = sub_m.value_of("target").unwrap().to_string();
            network_thread = Some(std::thread::spawn(move || {
                tokio_main(&listen_port, &target_server, abort_rx, ui_tx)
            }));
            (Session::default(), sub_m)
        }
        ("view", Some(sub_m)) => {
            let filename = sub_m.value_of("file").unwrap();
            let file = std::fs::File::open(filename)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>)
                .context(FileReadError {
                    file: filename.to_string(),
                })?;
            let session = rmp_serde::from_read(file)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send>)
                .context(FileReadError {
                    file: filename.to_string(),
                })?;
            (session, sub_m)
        }
        (_, _) => panic!("Sub command not handled!"),
    };

    let mut decoders = vec![];
    decoders.push(decoders::raw::initialize(&matches).context(DecoderError {})?);
    decoders.push(decoders::grpc::initialize(&matches).context(DecoderError {})?);
    let decoders = decoders.into_iter().filter_map(|o| o).collect();

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

#[tokio::main]
async fn tokio_main(
    listen_port: &str,
    target_server: &str,
    mut abort_rx: oneshot::Receiver<()>,
    ui_tx: Sender<session::events::SessionEvent>,
) -> Result<(), Error>
{
    // We'll want to liten for both IPv4 and IPv6. These days 'localhost' will first resolve to the
    // IPv6 address if that is available. If we did not bind to it, all the connections would first
    // need to timeout there before the Ipv4 would be attempted as a fallback.
    let mut listener_ipv4 = TcpListener::bind(format!("0.0.0.0:{}", listen_port))
        .await
        .unwrap();
    let mut listener_ipv6 = TcpListener::bind(format!("[::1]:{}", listen_port))
        .await
        .unwrap();

    // Loop until the abort signal is received.
    loop {
        tokio::select! {
            _ = &mut abort_rx => {
                break Ok(());
            },
            result = listener_ipv4.accept() => new_connection(ui_tx.clone(), result, &target_server),
            result = listener_ipv6.accept() => new_connection(ui_tx.clone(), result, &target_server),
        }
    }
}

fn new_connection(
    tx: Sender<session::events::SessionEvent>,
    result: Result<(TcpStream, SocketAddr), std::io::Error>,
    target_server: &str,
)
{
    // Process the new connection by spawning a new tokio task. This allows the original task to
    // process more connections.
    let target_server = target_server.to_string();
    if let Ok((socket, src_addr)) = result {
        tokio::spawn(async move {
            match handle_socket(tx, socket, src_addr, &target_server).await {
                Ok(..) => {}
                Err(e) => error!("Connection error\n{}", e),
            }
        });
    }
}

async fn handle_socket(
    tx: Sender<session::events::SessionEvent>,
    client_stream: TcpStream,
    src_addr: SocketAddr,
    target_server: &str,
) -> Result<(), Box<dyn std::error::Error>>
{
    let server_stream = TcpStream::connect(format!("{}", target_server)).await?;

    let tx_clone = tx.clone();
    let mut connection =
        ProxyConnection::new(client_stream, server_stream, src_addr, tx_clone).await?;
    connection.run(tx).await?;

    Ok(())
}
