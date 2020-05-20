use clap::{App, Arg};
use log::debug;
use std::error::Error;
use std::io::Read;
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

mod connection;
mod decoders;
mod error;
mod proto;
mod ui;
mod ui_state;

use connection::ProxyConnection;

async fn handle_socket(
    tx: Sender<ui_state::UiEvent>,
    client_stream: TcpStream,
    src_addr: SocketAddr,
    target_port: &str,
) -> Result<(), Box<dyn Error>>
{
    let server_stream = TcpStream::connect(format!("127.0.0.1:{}", target_port)).await?;

    let tx_clone = tx.clone();
    let mut connection =
        ProxyConnection::new(client_stream, server_stream, src_addr, tx_clone).await?;
    connection.run(tx).await?;

    Ok(())
}

#[tokio::main(core_threads = 4)]
pub async fn main() -> Result<(), Box<dyn Error>>
{
    simplelog::WriteLogger::init(
        simplelog::LevelFilter::Trace,
        simplelog::ConfigBuilder::new()
            .add_filter_allow("proxide".to_string())
            .build(),
        std::fs::File::create("trace.log").unwrap(),
    )
    .unwrap();

    let matches = App::new("Proxide - HTTP2 debugging proxy")
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
        )
        .arg(
            Arg::with_name("proto")
                .short("p")
                .value_name("PROTO_FILE")
                .help("Specify .proto file for decoding Protobuf messages")
                .takes_value(true),
        )
        .get_matches();

    let (abort_tx, mut abort_rx) = oneshot::channel::<()>();
    let (ui_tx, ui_rx) = std::sync::mpsc::channel();

    let proto = match matches.value_of("proto") {
        Some(file_name) => {
            let mut proto_file = String::new();
            let mut f = std::fs::File::open(file_name)?;
            f.read_to_string(&mut proto_file)?;
            proto::parse(&proto_file)?
        }
        None => proto::empty(),
    };

    let _ = std::thread::spawn({
        let ui_tx = ui_tx.clone();
        move || ui::main(abort_tx, ui_tx, ui_rx, proto)
    });

    let listen_port = matches.value_of("listen").unwrap();
    let mut listener_ipv4 = TcpListener::bind(format!("0.0.0.0:{}", listen_port))
        .await
        .unwrap();
    let mut listener_ipv6 = TcpListener::bind(format!("[::1]:{}", listen_port))
        .await
        .unwrap();

    let target_port = matches.value_of("target").unwrap();
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
    tx: Sender<ui_state::UiEvent>,
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
