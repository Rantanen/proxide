use clap::{arg, Parser};
use rust_grpc_private::DiagnosticsRequest;
use rust_grpc_private::{SendMessageRequest, WaitForFirstMessageRequest};
use std::{thread, time};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinSet;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
#[cfg(target_os = "windows")]
use windows;

mod rust_grpc_private
{
    tonic::include_proto!("rust_grpc");
}

/// Simple program to greet a person.
#[derive(Parser, Debug, Clone)]
#[command(author, version)]
pub struct Args
{
    /// Name of the person to greet.
    #[arg(short, long, default_value = "http://[::1]:50051")]
    pub address: String,

    /// Period / delay between the messages sent to the server.
    #[arg(short, long, value_parser = parse_period)]
    pub period: std::time::Duration,

    /// The number of asynchronous tasks used to send the messages in parallel.
    #[arg(short, long, default_value_t = 1)]
    pub tasks: u16,
}

/// A gRPC message generator that periodically sends messages to the target server.
pub struct GrpcGenerator
{
    generator: Option<thread::JoinHandle<()>>,
    stop: UnboundedSender<()>,
}

impl GrpcGenerator
{
    /// Stars a new gRPC generator.
    pub async fn start(args: Args) -> Result<Self, Box<dyn std::error::Error>>
    {
        // Start the generator in a separate tokio runtime to ensure its tasks won't interfere with the tests.
        let address_clone = args.address.to_string();
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
                    address: args.address,
                    period: args.period,
                    tasks: args.tasks,
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
            let mut client =
                rust_grpc_private::test_service_client::TestServiceClient::connect(address_clone)
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
    pub fn stop(&mut self) -> Result<(), Box<dyn std::error::Error>>
    {
        let _ = self.stop.send(()); // Fails when called repeatedly as the channel gets dropped.
        if let Some(generator) = self.generator.take() {
            if generator.join().is_err() {
                return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                    "Waiting for the generator to stop failed.",
                ));
            }
        }
        Ok(())
    }
}

impl Drop for GrpcGenerator
{
    fn drop(&mut self)
    {
        self.stop().expect("Dropping the generator failed. ");
    }
}

/// Spawns a new asynchronous message generation tasks.
fn spawn(args: Args) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    tokio::spawn(async move {
        generate_messages(args)
            .await
            .expect("Spawning gRPC client failed.")
    });
    Ok(())
}

/// Starts sending messages to the server,
async fn generate_messages(args: Args) -> Result<(), Box<dyn std::error::Error>>
{
    // Start the requested number of tasks.
    // Each task is given a unique client as the generator did not scale properly when the channel was shared
    // between the clients. The number of requests sent to the peaked at around <6 tasks. (Very rough approximation.)
    // TODO: Investigate further.
    let mut tasks: JoinSet<Result<(), Box<dyn std::error::Error + Send + Sync>>> = JoinSet::new();
    for _t in 0..args.tasks {
        let client = rust_grpc_private::test_service_client::TestServiceClient::connect(
            args.address.to_string(),
        )
        .await?;
        tasks.spawn(async move {
            generate_messages_task(client, args.period).await?;
            Ok(())
        });
    }
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(_) => {}
            Err(error) if error.is_cancelled() => {}
            Err(error) => return Err(Box::new(error)),
        }
    }

    Ok(())
}

/// An asynchronous function which sends messages to the server.
async fn generate_messages_task(
    mut client: rust_grpc_private::test_service_client::TestServiceClient<Channel>,
    period: std::time::Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    loop {
        let mut request = tonic::Request::new(SendMessageRequest {});
        request.metadata_mut().append(
            "proxide-client-process-id",
            MetadataValue::from(std::process::id()),
        );
        request.metadata_mut().append(
            "proxide-client-thread-id",
            MetadataValue::from(get_current_native_thread_id()),
        );
        tokio::select! {
            chosen = client.send_message(request) => { chosen?; },
            _chosen = tokio::signal::ctrl_c() => { break; }
        }

        #[allow(clippy::bool_comparison)]
        if period.is_zero() == false {
            tokio::time::sleep(period).await;
        }
    }
    Ok(())
}

#[tokio::main]
#[allow(dead_code)]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    let args = Args::parse();
    spawn(args.clone())?;
    tokio::select! {
        result = report_statistics( args ) => result?,
        result = tokio::signal::ctrl_c() => result?,
    }
    Ok(())
}

/// Reads period from the command line and converts it into duration.
fn parse_period(arg: &str) -> Result<std::time::Duration, std::num::ParseIntError>
{
    let seconds = arg.parse()?;
    Ok(std::time::Duration::from_millis(seconds))
}

/// Reports server statistics to the console.
async fn report_statistics(args: Args) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    let mut client =
        rust_grpc_private::test_service_client::TestServiceClient::connect(args.address).await?;

    loop {
        let response = client.get_diagnostics(DiagnosticsRequest {}).await?;
        let diagnostics = response.get_ref();
        let server_uptime = time::Duration::try_from(diagnostics.uptime.clone().unwrap())?;
        let call_rate = diagnostics.send_message_calls as u128 / server_uptime.as_millis();
        println!(
            "Call rate: {} calls / ms, processes: {} with {} threads",
            call_rate,
            diagnostics.clients.len(),
            diagnostics
                .clients
                .iter()
                .map(|c| c.threads.len() as u64)
                .sum::<u64>()
        );

        tokio::time::sleep(time::Duration::from_secs(2)).await;
    }
}

/// Gets the current native thread id.
pub fn get_current_native_thread_id() -> i64
{
    #[cfg(not(target_os = "windows"))]
    return os_id::thread::get_raw_id() as i64;

    #[cfg(target_os = "windows")]
    unsafe {
        return windows::Win32::System::Threading::GetCurrentThreadId() as i64;
    }
}
