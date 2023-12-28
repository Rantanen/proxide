pub use rust_grpc::{
    DiagnosticsRequest, DiagnosticsResponse, SendMessageRequest, SendMessageResponse,
};
use std::time;
use std::time::Duration;
use tonic::transport::channel::Channel;

mod generator;
mod server;

pub mod rust_grpc
{
    tonic::include_proto!("rust_grpc");
}

pub struct Args
{
    /// Period / delay between the messages sent to the server.
    pub period: std::time::Duration,

    /// The number of asynchronous tasks used to send the messages in parallel.
    pub tasks: u16,
}

/// Snapshot of statistics of a test run.
pub struct Statistics
{
    /// Uptime of the server associated with the tester.
    pub server_uptime: std::time::Duration,

    /// Number of "SendMessage" calls the tester has processed.
    pub send_message_calls_processed: u64,
}

pub struct GrpcTester
{
    server: server::GrpcServer,
    generator: generator::GrpcGenerator,
    client: rust_grpc::test_service_client::TestServiceClient<Channel>,
}

impl GrpcTester
{
    /// Gets gRPC client for communicating with the server associated with the tester.
    pub fn client(&self) -> rust_grpc::test_service_client::TestServiceClient<Channel>
    {
        self.client.clone()
    }

    pub async fn pipe() -> Result<GrpcTester, Box<dyn std::error::Error>>
    {
        Self::pipe_with_args(Args {
            tasks: 1,
            period: Duration::from_secs(1),
        })
        .await
    }

    /// Creates a new testes which internally pipes data from client to the server.
    pub async fn pipe_with_args(args: Args) -> Result<GrpcTester, Box<dyn std::error::Error>>
    {
        let server = server::GrpcServer::start().await?;
        let generator = generator::GrpcGenerator::start(generator::Args {
            address: server.http(),
            period: args.period,
            tasks: args.tasks,
        })
        .await?;
        let client =
            rust_grpc::test_service_client::TestServiceClient::connect(server.http()).await?;
        Ok(GrpcTester {
            server,
            generator,
            client,
        })
    }

    pub async fn get_statistics(&self) -> Result<Statistics, Box<dyn std::error::Error>>
    {
        let diagnostics = self
            .client
            .clone()
            .get_diagnostics(DiagnosticsRequest {})
            .await?;
        let diagnostics = diagnostics.get_ref();

        Ok(Statistics {
            server_uptime: time::Duration::try_from(diagnostics.uptime.clone().unwrap())?,
            send_message_calls_processed: diagnostics.send_message_calls,
        })
    }

    /// Stops the gRPC Server
    pub fn stop(&mut self) -> Result<(), Box<dyn std::error::Error>>
    {
        self.generator.stop()?;
        self.server.stop()?;

        Ok(())
    }
}

impl Drop for GrpcTester
{
    fn drop(&mut self)
    {
        self.stop().expect("Dropping the tester failed.");
    }
}

#[cfg(test)]
mod test
{
    use crate::{Args, GrpcTester};
    use std::time::Duration;

    #[tokio::test]
    async fn starting_and_stopping_tester_succeeds()
    {
        let mut tester = GrpcTester::pipe().await.expect("Starting tester failed.");
        tester.stop().expect("Stopping tester failed.");
    }

    #[tokio::test]
    async fn server_has_valid_uptime()
    {
        let mut tester = GrpcTester::pipe().await.expect("Starting tester failed.");

        let statistics = tester
            .get_statistics()
            .await
            .expect("Fetching tester statistics failed.");
        if statistics.server_uptime.is_zero() {
            panic!("Uptime of the server cannot be zero.")
        }

        tester.stop().expect("Stopping tester failed.");
    }

    #[tokio::test]
    async fn server_receives_messages()
    {
        // Ensure the generator sends messages constantly to minimize the test duration.
        let mut tester = GrpcTester::pipe_with_args(Args {
            tasks: 1,
            period: Duration::from_secs(0),
        })
        .await
        .expect("Starting tester failed.");

        // Ensure the server is reporting increase in the number of processed send_message calls.
        let statistics_base = tester
            .get_statistics()
            .await
            .expect("Fetching tester statistics failed.");
        for attempt in 0.. {
            let statistics = tester
                .get_statistics()
                .await
                .expect("Fetching tester statistics failed.");
            if statistics.server_uptime <= statistics_base.server_uptime {
                panic!("Server's uptime should be increasing.")
            }
            if statistics.send_message_calls_processed
                > statistics_base.send_message_calls_processed
            {
                break;
            }
            if attempt > 100 {
                panic!("Server did not report any increase in send_message calls.")
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tester.stop().expect("Stopping tester failed.");
    }
}
