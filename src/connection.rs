use bytes::Bytes;
use futures::join;
use h2::{
    client::{self, ResponseFuture},
    server::{self, SendResponse},
    RecvStream, SendStream,
};
use http::{HeaderMap, Request, Response};
use log::error;
use snafu::{ResultExt, Snafu};
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use tokio::net::TcpStream;
use uuid::Uuid;

use crate::ui_state::*;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error
{
    #[snafu(display("HTTP error occurred with the server in {}: {}", scenario, source))]
    ServerError
    {
        scenario: &'static str,
        source: h2::Error,
    },

    #[snafu(display("HTTP error occurred with the client in {}: {}", scenario, source))]
    ClientError
    {
        scenario: &'static str,
        source: h2::Error,
    },
}

pub type Result<S, E = Error> = std::result::Result<S, E>;

pub struct ProxyConnection
{
    uuid: Uuid,
    client_connection: server::Connection<TcpStream, Bytes>,
    server_stream: client::SendRequest<Bytes>,
}

impl ProxyConnection
{
    pub async fn new(
        client: TcpStream,
        server: TcpStream,
        src_addr: SocketAddr,
        ui: Sender<UiEvent>,
    ) -> Result<ProxyConnection>
    {
        let client_connection = server::handshake(client).await.context(ClientError {
            scenario: "client handshake",
        })?;
        let (server_stream, server_connection) =
            client::handshake(server).await.context(ServerError {
                scenario: "server handshake",
            })?;

        tokio::spawn(async move {
            match server_connection.await {
                Ok(..) => {}
                Err(e) => error!("Error: {:?}", e),
            }
        });

        let server_stream = server_stream.ready().await.context(ServerError {
            scenario: "starting stream",
        })?;

        let conn = ProxyConnection {
            uuid: Uuid::new_v4(),
            client_connection,
            server_stream,
        };

        ui.send(UiEvent::NewConnection(NewConnectionEvent {
            uuid: conn.uuid,
            client_addr: src_addr,
        }))
        .unwrap();
        Ok(conn)
    }

    pub async fn run(&mut self, ui: Sender<UiEvent>) -> Result<()>
    {
        let r = {
            let ui = ui.clone();
            let client_connection = &mut self.client_connection;
            let server_stream = &mut self.server_stream;
            let uuid = self.uuid;
            async move {
                while let Some(request) = client_connection.accept().await {
                    // Process the client request.
                    let (client_request, client_response) = request.context(ClientError {
                        scenario: "processing request",
                    })?;

                    let request = ProxyRequest::new(
                        uuid,
                        client_request,
                        client_response,
                        server_stream,
                        &ui,
                    )?;

                    let ui = ui.clone();
                    tokio::spawn(async move {
                        let ui = ui;
                        match request.execute(ui).await {
                            Ok(_) => {}
                            Err(e) => error!("{}", e),
                        }
                    });
                }

                Ok(())
            }
        }
        .await;

        ui.send(UiEvent::ConnectionClosed {
            uuid: self.uuid,
            status: match r {
                Ok(_) => crate::ui_state::Status::Succeeded,
                Err(_) => crate::ui_state::Status::Failed,
            },
        })
        .unwrap();
        r
    }
}

pub struct ProxyRequest
{
    uuid: Uuid,
    client_request: RecvStream,
    client_response: SendResponse<Bytes>,
    server_request: SendStream<Bytes>,
    server_response: ResponseFuture,
}

impl ProxyRequest
{
    pub fn new(
        uuid_parent: Uuid,
        client_request: Request<RecvStream>,
        client_response: SendResponse<Bytes>,
        server_stream: &mut client::SendRequest<Bytes>,
        ui: &Sender<UiEvent>,
    ) -> Result<ProxyRequest>
    {
        let uuid = Uuid::new_v4();
        let (client_head, client_request) = client_request.into_parts();

        ui.send(UiEvent::NewRequest(NewRequestEvent {
            connection_uuid: uuid_parent,
            uuid: uuid,
            uri: client_head.uri.clone(),
            method: client_head.method.clone(),
            headers: client_head.headers.clone(),
        }))
        .unwrap();

        let server_request = Request::from_parts(client_head, ());

        // Set up a server request.
        let (server_response, server_request) = server_stream
            .send_request(server_request, false)
            .context(ServerError {
            scenario: "sending request",
        })?;

        Ok(ProxyRequest {
            uuid,
            client_request,
            client_response,
            server_request,
            server_response,
        })
    }

    pub async fn execute(self, ui: Sender<UiEvent>) -> Result<()>
    {
        ui.send(UiEvent::RequestStatus(RequestStatusEvent {
            uuid: self.uuid,
            status: crate::ui_state::Status::InProgress,
        }))
        .unwrap();

        // Acquire futures that are responsible for streaming the request and the response. These
        // are set up in their own futures to allow parallel request/response streaming to occur.

        // Set up streaming the request to the server.
        let uuid = self.uuid;
        let client_request = self.client_request;
        let mut server_request = self.server_request;
        let ui_temp = ui.clone();
        let request_future = async move {
            let ui = ui_temp;
            log::info!("{}: Client stream starting", uuid);
            let trailers = pipe_stream(
                client_request,
                &mut server_request,
                ui,
                |ui, bytes| {
                    ui.send(UiEvent::RequestData(RequestDataEvent {
                        uuid: uuid,
                        data: bytes.clone(),
                    }))
                    .unwrap();
                },
                false,
            )
            .await?;
            log::info!("{}: Client stream ended", uuid);

            if let Some(trailers) = trailers {
                server_request
                    .send_trailers(trailers)
                    .context(ServerError {
                        scenario: "sending trailers",
                    })?;
                log::info!("{}: Client trailers sent", uuid);
            }

            Ok(())
        };

        // Set up streaming the response to the client.
        //
        // This is done in its own async block, since it's the pipe_stream async call that we'll
        // want to happen in parallel, but there's a good chance the server won't send the
        // response before the request stream has proceeded at least some. (Most likely the server
        // will require that stream to proceed in full, unless the call is some sort of a streaming
        // call.
        let mut client_response = self.client_response;
        let server_response = self.server_response;
        let ui_temp = ui.clone();
        let response_future = async move {
            let ui = ui_temp;
            let response = server_response.await.context(ServerError {
                scenario: "waiting for response",
            })?;

            let (response_head, response_body) = response.into_parts();

            let response = Response::from_parts(response_head, ());

            let mut client_stream =
                client_response
                    .send_response(response, false)
                    .context(ClientError {
                        scenario: "sending response",
                    })?;

            log::info!("{}: Server stream starting", uuid);
            let trailers = pipe_stream(
                response_body,
                &mut client_stream,
                ui,
                |ui, bytes| {
                    ui.send(UiEvent::ResponseData(ResponseDataEvent {
                        uuid: uuid,
                        data: bytes.clone(),
                    }))
                    .unwrap();
                },
                true,
            )
            .await?;
            log::info!("{}: Server stream ended", uuid);

            if let Some(trailers) = trailers {
                client_stream.send_trailers(trailers).context(ServerError {
                    scenario: "sending trailers",
                })?;
                log::info!("{}: Server trailers sent", uuid);
            }

            Ok(())
        };

        // Now handle both futures in parallel.
        let (r1, r2) = join!(request_future, response_future);
        let r = r1.and(r2);
        ui.send(UiEvent::RequestStatus(RequestStatusEvent {
            uuid: self.uuid,
            status: match r.is_ok() {
                true => crate::ui_state::Status::Succeeded,
                false => crate::ui_state::Status::Failed,
            },
        }))
        .unwrap();
        r
    }
}

async fn pipe_stream<F: Fn(Sender<UiEvent>, &bytes::Bytes)>(
    mut source: RecvStream,
    target: &mut SendStream<Bytes>,
    ui: Sender<UiEvent>,
    f: F,
    server: bool,
) -> Result<Option<HeaderMap>>
{
    while let Some(data) = source.data().await {
        let b = data.context(ClientError {
            scenario: "reading content",
        })?;
        f(ui.clone(), &b);
        target
            .send_data(b, source.is_end_stream())
            .context(ServerError {
                scenario: "writing content",
            })?;
    }

    Ok(source.trailers().await.context(ClientError {
        scenario: "receiving trailers",
    })?)
}
