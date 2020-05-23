use clap::{App, Arg};
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

#[tokio::main(core_threads = 4)]
pub async fn main() -> Result<(), Error>
{
    simplelog::WriteLogger::init(
        simplelog::LevelFilter::Trace,
        simplelog::ConfigBuilder::new()
            .add_filter_allow("proxide".to_string())
            .build(),
        std::fs::File::create("trace.log").unwrap(),
    )
    .unwrap();

    let app = App::new("Proxide - HTTP2 debugging proxy")
        .version(env!("CARGO_PKG_VERSION"))
        .author("Mikko Rantanen <rantanen@jubjubnest.net>")
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
    let app = decoders::grpc::setup_args(app);

    let matches = app.get_matches();

    let (abort_tx, mut abort_rx) = oneshot::channel::<()>();
    let (ui_tx, ui_rx) = std::sync::mpsc::channel();

    let listen_port = matches.value_of("listen").unwrap();
    let mut listener_ipv4 = TcpListener::bind(format!("0.0.0.0:{}", listen_port))
        .await
        .unwrap();
    let mut listener_ipv6 = TcpListener::bind(format!("[::1]:{}", listen_port))
        .await
        .unwrap();

    let target_port = matches.value_of("target").unwrap().to_string();

    let h: std::thread::JoinHandle<Result<(), Error>> = std::thread::spawn({
        move || {
            let mut decoders = vec![];
            decoders.push(decoders::raw::initialize(&matches).context(DecoderError {})?);
            decoders.push(decoders::grpc::initialize(&matches).context(DecoderError {})?);
            let decoders = decoders.into_iter().filter_map(|o| o).collect();

            Ok(ui::main(abort_tx, ui_rx, decoders).context(UiError {})?)
        }
    });

    loop {
        tokio::select! {
            _ = &mut abort_rx => {
                let r = h.join().unwrap();
                break r;
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
