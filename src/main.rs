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
    let server_stream = TcpStream::connect("192.168.0.103:7766").await?;

    let tx_clone = tx.clone();
    let mut connection =
        ProxyConnection::new(client_stream, server_stream, src_addr, tx_clone).await?;
    connection.run(tx).await?;

    Ok(())
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn Error>>
{
    // env_logger::init();

    let (abort_tx, mut abort_rx) = oneshot::channel::<()>();
    let (ui_tx, ui_rx) = std::sync::mpsc::channel();

    let logger = Logger(std::sync::Mutex::new(ui_tx.clone()));
    log::set_boxed_logger(Box::new(logger))
        .map(|()| log::set_max_level(log::LevelFilter::Debug))
        .unwrap();

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

    let mut listener = TcpListener::bind("0.0.0.0:8888").await.unwrap();

    loop {
        tokio::select! {
            _ = &mut abort_rx => {
                break Ok(());
            },
            result = listener.accept() => {
                if let Ok((socket, src_addr)) = result {
                    debug!("New connection from {:?}", src_addr);
                    let tx = ui_tx.clone();
                    tokio::spawn(async move {
                        let tx = tx;
                        match handle_socket(tx, socket, src_addr).await {
                            Ok(..) => {}
                            Err(e) => debug!("Error {:?}", e),
                        }
                    });
                }
            },
        }
    }
}

struct Logger(std::sync::Mutex<Sender<ui_state::UiEvent>>);
impl log::Log for Logger
{
    fn enabled(&self, metadata: &log::Metadata) -> bool
    {
        true
    }

    fn log(&self, record: &log::Record)
    {
        if !record.target().starts_with("proxide") {
            return;
        }

        self.0
            .lock()
            .expect("Mutex poisoned")
            .send(ui_state::UiEvent::LogMessage(format!(
                "{}:{} {}: {}\n",
                record.file().unwrap_or_else(|| "<Unknown>"),
                record.line().unwrap_or_else(|| 0),
                record.metadata().level(),
                record.args()
            )))
            .unwrap();
    }

    fn flush(&self) {}
}
