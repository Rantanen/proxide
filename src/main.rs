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
) -> Result<(), Box<dyn Error>>
{
    let server_stream = TcpStream::connect("127.0.0.1:7766").await?;

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

    let (abort_tx, mut abort_rx) = oneshot::channel::<()>();
    let (ui_tx, ui_rx) = std::sync::mpsc::channel();

    let proto = {
        let mut proto_file = String::new();
        let mut f = std::fs::File::open(std::env::args().into_iter().nth(1).unwrap().as_str())?;
        f.read_to_string(&mut proto_file)?;
        proto::parse(&proto_file)?
    };

    let _ = std::thread::spawn({
        let ui_tx = ui_tx.clone();
        move || ui::main(abort_tx, ui_tx, ui_rx, proto)
    });

    let mut listener_ipv4 = TcpListener::bind("0.0.0.0:8888").await.unwrap();
    let mut listener_ipv6 = TcpListener::bind("[::1]:8888").await.unwrap();

    loop {
        tokio::select! {
            _ = &mut abort_rx => {
                break Ok(());
            },
            result = listener_ipv4.accept() => new_connection(ui_tx.clone(), result),
            result = listener_ipv6.accept() => new_connection(ui_tx.clone(), result),
        }
    }
}

fn new_connection(
    tx: Sender<ui_state::UiEvent>,
    result: Result<(TcpStream, SocketAddr), std::io::Error>,
)
{
    if let Ok((socket, src_addr)) = result {
        debug!("New connection from {:?}", src_addr);
        tokio::spawn(async move {
            match handle_socket(tx, socket, src_addr).await {
                Ok(..) => {}
                Err(e) => debug!("Error {:?}", e),
            }
        });
    }
}
