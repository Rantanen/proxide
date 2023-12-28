pub use rust_grpc::{
    DiagnosticsRequest, DiagnosticsResponse, SendMessageRequest, SendMessageResponse,
};
use tonic::transport::channel::Channel;

mod generator;
mod server;

pub mod rust_grpc {
    tonic::include_proto!("rust_grpc");
}

pub struct GrpcTester {
    server: server::GrpcServer,
    generator: generator::GrpcGenerator,
    client: rust_grpc::test_service_client::TestServiceClient<Channel>,
}

impl GrpcTester {
    /// Gets gRPC client for communicating with the server associated with the tester.
    pub fn client(&self) -> rust_grpc::test_service_client::TestServiceClient<Channel> {
        self.client.clone()
    }

    /// Creates a new testes which internally pipes data from client to the server.
    pub async fn pipe() -> Result<GrpcTester, Box<dyn std::error::Error>> {
        let server = server::GrpcServer::start().await?;
        let http_address = server.http();
        let generator = generator::GrpcGenerator::start(&http_address).await?;
        let client =
            rust_grpc::test_service_client::TestServiceClient::connect(server.http()).await?;
        Ok(GrpcTester {
            server,
            generator,
            client,
        })
    }

    /// Stops the gRPC Server
    pub fn stop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.generator.stop()?;
        self.server.stop()?;

        Ok(())
    }
}

impl Drop for GrpcTester {
    fn drop(&mut self) {
        self.stop().expect("Dropping the tester failed.");
    }
}

#[cfg(test)]
mod test {
    use crate::GrpcTester;

    #[tokio::test]
    async fn starting_and_stopping_tester_succeeds() {
        let mut tester = GrpcTester::pipe().await.expect("Starting tester failed.");
        tester.stop().expect("Stopping tester failed.");
    }
}
