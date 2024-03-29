use atomic_counter::AtomicCounter;
use clap::Parser;
use rust_grpc_private::test_service_server::{TestService, TestServiceServer};
use rust_grpc_private::{
    ClientProcess, DiagnosticsRequest, DiagnosticsResponse, PingRequest, PingResponse,
    SendMessageRequest, SendMessageResponse, WaitForFirstMessageRequest,
    WaitForFirstMessageResponse,
};
use std::net::SocketAddr;
use std::str::FromStr;
use std::thread;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tokio::sync::watch::{Receiver, Sender};
use tonic::metadata::{Ascii, MetadataValue};
use tonic::transport::Server;
use tonic::{Request, Response, Status};

mod rust_grpc_private
{
    tonic::include_proto!("rust_grpc");
}

/// A simple gRPC Server for receiving messages
#[derive(Parser, Debug)]
#[command(author, version)]
struct Args
{
    /// Network address to listen.
    #[arg(short, long, default_value = "[::1]:50051")]
    pub address: String,
}

/// A gRPC server ready to accept messages
pub struct GrpcServer
{
    address: SocketAddr,
    server: Option<thread::JoinHandle<()>>,
    stop: UnboundedSender<()>,
}

impl GrpcServer
{
    /// Starts a new gRPC server.
    pub async fn start() -> Result<Self, Box<dyn std::error::Error>>
    {
        // Start the server in a separate tokio runtime to ensure its tasks won't interfere with the tests.
        let address: SocketAddr = format!(
            "[::1]:{}",
            portpicker::pick_unused_port().expect("No TCP ports available.")
        )
        .parse()?;
        let address_clone = address;
        let (server_listening_send, mut server_listening_recv) =
            tokio::sync::mpsc::unbounded_channel();
        let (stop_requested_send, mut stop_requested_recv) = tokio::sync::mpsc::unbounded_channel();
        let server = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .thread_name("grpc-server")
                .enable_all()
                .build()
                .expect("Failed to start tokio runtime for grpc-server.");
            rt.block_on(async move {
                LocalTestService::spawn(address_clone).expect("gRPC Server failed.");
                server_listening_send
                    .send(())
                    .expect("Sending server ready failed.");
                tokio::select! {
                    _chosen = stop_requested_recv.recv() => {},
                    _chosen = tokio::signal::ctrl_c() => {},
                }
            });
        });
        let _ = server_listening_recv.recv().await;

        // Ensure the server is ready.
        let server = GrpcServer {
            address,
            server: Some(server),
            stop: stop_requested_send,
        };
        server.wait_for_server_to_listen().await?;
        Ok(server)
    }

    /// Gets the server's address.
    pub fn address(&self) -> &SocketAddr
    {
        &self.address
    }

    /// Gets the HTTP address of the server.
    pub fn http(&self) -> String
    {
        format!("http://{}", &self.address)
    }

    /// Stops the gRPC server.
    pub fn stop(&mut self) -> Result<(), Box<dyn std::error::Error>>
    {
        let _ = self.stop.send(()); // Fails when called repeatedly as the channel gets dropped.
        if let Some(server) = self.server.take() {
            if server.join().is_err() {
                return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                    "Waiting for the server to stop failed.",
                ));
            }
        }
        Ok(())
    }

    /// Pings the server and ensures it is listening.
    async fn wait_for_server_to_listen(&self) -> Result<(), Box<dyn std::error::Error>>
    {
        // Try to establish connection to the server.
        const MAX_ATTEMPTS: u32 = 100;
        for attempt in 1.. {
            let mut client =
                match rust_grpc_private::test_service_client::TestServiceClient::connect(
                    self.http(),
                )
                .await
                {
                    Ok(client) => client,
                    Err(_) if attempt < MAX_ATTEMPTS => {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        continue;
                    }
                    Err(error) => return Err(Box::new(error)),
                };
            match client.ping(PingRequest {}).await {
                Ok(_) => {
                    break; // A message was sent to the server.
                }
                Err(_) if attempt < MAX_ATTEMPTS => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    continue;
                }
                Err(error) => {
                    return Err(Box::new(error));
                }
            };
        }
        Ok(())
    }
}

impl Drop for GrpcServer
{
    fn drop(&mut self)
    {
        self.stop().expect("Dropping GrpcServer failed.");
    }
}

/// A message sink for the message generator.
///
/// Collects statistics about the calls it receives.
struct LocalTestService
{
    /// A watcher for acknowledging that the first "SendMessage" call has been received by the server.
    message_received_notify: Sender<bool>,

    /// A watcher for checking whether the server has received "SendMessage" call,
    message_received_check: Receiver<bool>,

    /// Timestamp when the service was created.
    started: Instant,

    /// The number of send_message calls the service has received.
    send_message_calls_served: atomic_counter::RelaxedCounter,

    /// Collection of client processes and threads as reported with
    /// the proxide-client-process-id and proxide-client-thread-id headers
    clients: crossbeam_skiplist::SkipMap<u32, crossbeam_skiplist::SkipSet<u64>>,
}

impl LocalTestService
{
    /// Spawns the test service in a new asynchronous task.
    fn spawn(address: SocketAddr) -> Result<(), Box<dyn std::error::Error>>
    {
        tokio::spawn(async move {
            LocalTestService::run(address)
                .await
                .expect("Spawning gRPC server failed.")
        });
        Ok(())
    }

    /// Launches the the test service.
    async fn run(address: SocketAddr) -> Result<(), Box<dyn std::error::Error>>
    {
        let (tx, rx) = watch::channel(false);
        let service = LocalTestService {
            message_received_notify: tx,
            message_received_check: rx,
            started: Instant::now(),
            send_message_calls_served: atomic_counter::RelaxedCounter::new(0),
            clients: crossbeam_skiplist::SkipMap::new(),
        };
        #[cfg(not(test))]
        println!("Test server listening on {}", address);
        Server::builder()
            .concurrency_limit_per_connection(128)
            .add_service(TestServiceServer::new(service))
            .serve_with_shutdown(address, async {
                tokio::signal::ctrl_c()
                    .await
                    .expect("Waiting shutdown failed");
            })
            .await?;

        Ok(())
    }
}

impl Drop for LocalTestService
{
    fn drop(&mut self)
    {
        let _ = self.message_received_notify.send(true);
    }
}

#[tonic::async_trait]
impl TestService for LocalTestService
{
    async fn send_message(
        &self,
        request: Request<SendMessageRequest>,
    ) -> Result<Response<SendMessageResponse>, Status>
    {
        // Avoid unnecessary notifications to reduce CPU <-> CPU communication.
        self.message_received_notify
            .send_if_modified(|value: &mut bool| {
                #[allow(clippy::bool_comparison)]
                if *value == false {
                    *value = true;
                    true
                } else {
                    false
                }
            });
        self.send_message_calls_served.inc();

        // Collect the client info.
        let process_id = request.metadata().get("proxide-client-process-id");
        let thread_id = request.metadata().get("proxide-client-thread-id");
        if process_id.is_some() && thread_id.is_some() {
            let threads = self.clients.get_or_insert(
                number_from_client(process_id.unwrap())?,
                crossbeam_skiplist::SkipSet::new(),
            );
            threads
                .value()
                .insert(number_from_client(thread_id.unwrap())?);
        }
        Ok(Response::new(SendMessageResponse {}))
    }

    async fn get_diagnostics(
        &self,
        _request: Request<DiagnosticsRequest>,
    ) -> Result<Response<DiagnosticsResponse>, Status>
    {
        let duration = Instant::now().duration_since(self.started);
        let duration = match prost_types::Duration::try_from(duration) {
            Ok(d) => d,
            Err(_) => return Err(Status::internal("Calculating server uptime failed.")),
        };

        let clients = self
            .clients
            .iter()
            .map(|c| ClientProcess {
                id: *c.key(),
                threads: c.value().iter().map(|e| *e.value()).collect(),
            })
            .collect();

        Ok(Response::new(DiagnosticsResponse {
            uptime: Some(duration),
            send_message_calls: self.send_message_calls_served.get() as u64,
            clients,
        }))
    }

    /// Waits and blocks until the server has received at least one send_message call.
    async fn wait_for_first_message(
        &self,
        _request: Request<WaitForFirstMessageRequest>,
    ) -> Result<Response<WaitForFirstMessageResponse>, Status>
    {
        self.message_received_check
            .clone()
            .wait_for(|value| value == &true)
            .await
            .expect("Waiting for the first SendMessage call failed.");
        Ok(Response::new(WaitForFirstMessageResponse {}))
    }

    async fn ping(&self, _request: Request<PingRequest>) -> Result<Response<PingResponse>, Status>
    {
        Ok(Response::new(PingResponse {}))
    }
}

#[tokio::main]
#[allow(dead_code)]
async fn main() -> Result<(), Box<dyn std::error::Error>>
{
    let args = Args::parse();
    LocalTestService::spawn(args.address.parse()?)?;
    tokio::signal::ctrl_c().await?;
    Ok(())
}

fn number_from_client<N>(value: &MetadataValue<Ascii>) -> Result<N, Status>
where
    N: FromStr,
{
    let value = match value.to_str() {
        Ok(v) => v,
        Err(_) => return Err(Status::invalid_argument("Header was not a string.")),
    };
    match N::from_str(value) {
        Ok(numbert) => Ok(numbert),
        Err(_) => Err(Status::invalid_argument("Expected number")),
    }
}
