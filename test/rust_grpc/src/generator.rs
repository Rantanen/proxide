use clap::Parser;
use rust_grpc_private::{SendMessageRequest, WaitForFirstMessageRequest};
use std::thread;
use tokio::sync::mpsc::UnboundedSender;

mod rust_grpc_private {
    tonic::include_proto!("rust_grpc");
}

/// Simple program to greet a person.
#[derive(Parser, Debug)]
#[command(author, version)]
pub struct Args {
    /// Name of the person to greet.
    #[arg(short, long, default_value = "http://[::1]:50051")]
    pub address: String,

    /// Period / delay between the messages sent to the server.
    #[arg(short, long, value_parser = parse_period)]
    pub period: std::time::Duration,
}

/// A gRPC message generator that periodically sends messages to the target server.
pub struct GrpcGenerator {
    generator: Option<thread::JoinHandle<()>>,
    stop: UnboundedSender<()>,
}

impl GrpcGenerator {
    /// Stars a new gRPC generator.
    pub async fn start(address: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // Start the generator in a separate tokio runtime to ensure its tasks won't interfere with the tests.
        let address_clone = address.to_string();
        let (generator_started_send, mut generator_started_recv) =
            tokio::sync::mpsc::unbounded_channel();
        let (stop_requested_send, mut stop_requested_recv) = tokio::sync::mpsc::unbounded_channel();
        let generator = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .thread_name("grpc-generator")
                .enable_all()
                .build()
                .expect("Starting runtime for message generator failed.");
            rt.block_on(async move {
                spawn(Args {
                    address: address_clone,
                    period: std::time::Duration::from_millis(100),
                })
                .expect("Starting generator failed.");
                generator_started_send
                    .send(())
                    .expect("Sending generator ready failed.");
                tokio::select! {
                    _chosen = stop_requested_recv.recv() => {},
                    _chosen = tokio::signal::ctrl_c() => {},
                }
            });
        });
        let _ = generator_started_recv.recv().await;

        // Wait for the first message to reach the server.
        // This improve the robustness of the tests utilizing the generator as the environment is guaranteed to work after this.
        {
            let mut client = rust_grpc_private::test_service_client::TestServiceClient::connect(
                address.to_string(),
            )
            .await?;
            let _ = client
                .wait_for_first_message(WaitForFirstMessageRequest {})
                .await;
        }

        Ok(GrpcGenerator {
            generator: Some(generator),
            stop: stop_requested_send,
        })
    }

    /// Stops the gRPC message generation.
    pub fn stop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.stop.send(()); // Fails when called repeatedly as the channel gets dropped.
        if let Some(generator) = self.generator.take() {
            if let Err(_) = generator.join() {
                return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                    "Waiting for the generator to stop failed.",
                ));
            }
        }
        Ok(())
    }
}

impl Drop for GrpcGenerator {
    fn drop(&mut self) {
        self.stop().expect("Dropping the generator failed. ");
    }
}

/// Spawns a new asynchronous message generation tasks.
fn spawn(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    tokio::spawn(async move {
        generate_messages(args)
            .await
            .expect("Spawning gRPC client failed.")
    });
    Ok(())
}

/// Starts sending messages to the server,
async fn generate_messages(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut client =
        rust_grpc_private::test_service_client::TestServiceClient::connect(args.address).await?;

    loop {
        let request = tonic::Request::new(SendMessageRequest {});

        tokio::select! {
            chosen = client.send_message(request) => { chosen?; },
            _chosen = tokio::signal::ctrl_c() => { break; }
        }

        if args.period.is_zero() == false {
            tokio::time::sleep(args.period).await;
        }
    }

    Ok(())
}

#[tokio::main]
#[allow(dead_code)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    spawn(Args::parse())?;
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// Reads period from the command line and converts it into duration.
fn parse_period(arg: &str) -> Result<std::time::Duration, std::num::ParseIntError> {
    let seconds = arg.parse()?;
    Ok(std::time::Duration::from_millis(seconds))
}
