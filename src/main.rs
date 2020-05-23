use clap::{App, AppSettings, Arg, SubCommand};
use log::debug;
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

type Result<S, E = Error> = std::result::Result<S, E>;

async fn handle_socket(
    tx: Sender<session::events::SessionEvent>,
    client_stream: TcpStream,
    src_addr: SocketAddr,
    target_port: &str,
) -> Result<(), Box<dyn std::error::Error>>
{
    let server_stream = TcpStream::connect(format!("127.0.0.1:{}", target_port)).await?;

    let tx_clone = tx.clone();
    let mut connection =
        ProxyConnection::new(client_stream, server_stream, src_addr, tx_clone).await?;
    connection.run(tx).await?;

    Ok(())
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

    // Set up the monitor and view subcommand as a distinct command.
    //
    // This is the one that decoders can add their own parameters to.
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

    for cmd in vec![monitor_cmd, view_cmd]
        .into_iter()
        .map(|cmd| decoders::grpc::setup_args(cmd))
    {
        app = app.subcommand(cmd);
    }

    // We'll have the channels present all the time to simplify setup.
    // The parameters are free to use them if they want.
    let (abort_tx, abort_rx) = oneshot::channel::<()>();
    let (ui_tx, ui_rx) = std::sync::mpsc::channel();

    let mut network_thread = None;
    let matches = app.get_matches();
    let (session, matches) = match matches.subcommand() {
        ("monitor", Some(sub_m)) => {
            // Monitor sets up the network tack.
            let listen_port = matches.value_of("listen").unwrap().to_string();
            let target_port = sub_m.value_of("target").unwrap().to_string();
            network_thread = Some(std::thread::spawn(move || {
                tokio_main(&listen_port, &target_port, abort_rx, ui_tx)
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
    target_port: &str,
    mut abort_rx: oneshot::Receiver<()>,
    ui_tx: Sender<session::events::SessionEvent>,
) -> Result<(), Error>
{
    let mut listener_ipv4 = TcpListener::bind(format!("0.0.0.0:{}", listen_port))
        .await
        .unwrap();
    let mut listener_ipv6 = TcpListener::bind(format!("[::1]:{}", listen_port))
        .await
        .unwrap();

    loop {
        tokio::select! {
            _ = &mut abort_rx => {
                break Ok(());
            },
            result = listener_ipv4.accept() => new_connection(ui_tx.clone(), result, &target_port),
            result = listener_ipv6.accept() => new_connection(ui_tx.clone(), result, &target_port),
        }
    }
}

fn new_connection(
    tx: Sender<session::events::SessionEvent>,
    result: Result<(TcpStream, SocketAddr), std::io::Error>,
    target_port: &str,
)
{
    let target_port = target_port.to_string();
    if let Ok((socket, src_addr)) = result {
        tokio::spawn(async move {
            match handle_socket(tx, socket, src_addr, &target_port).await {
                Ok(..) => {}
                Err(e) => debug!("Error {:?}", e),
            }
        });
    }
}
