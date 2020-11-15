use bytes::Bytes;
use futures::{prelude::*, try_join};
use h2::{
    client::{self, ResponseFuture, SendRequest},
    server::{self, SendResponse},
    Reason, RecvStream, SendStream,
};
use http::{HeaderMap, HeaderValue, Request, Response};
use log::error;
use snafu::ResultExt;
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use crate::connection::{error::*, scripting, ConnectionDetails, Protocol, Streams};
use crate::session::events::*;
use crate::session::{RequestPart, Status};
use crate::ConnectionOptions;

pub async fn handle<TClient, TServer>(
    mut details: ConnectionDetails,
    options: Arc<ConnectionOptions>,
    client_addr: SocketAddr,
    streams: Streams<TClient, TServer>,
    ui: Sender<SessionEvent>,
) -> Result<()>
where
    TClient: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    TServer: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Streams { client, server } = streams;
    details.protocol_stack.push(Protocol::Http2);

    // This is a debugging proxy so we don't need to be supporting hundreds of concurrent
    // requests. We can opt for a bit larger window size to avoid slowing down the connection.
    let c2p_connection = server::Builder::new()
        .initial_window_size(1_000_000)
        .handshake::<TClient, Bytes>(client)
        .await
        .context(H2Error {})
        .context(EndpointError {
            endpoint: EndpointType::Client,
            scenario: "client handshake",
        })?;

    // Handshake the incoming client connection at the proxy.
    let (p2s_send_request, p2s_connection) = client::handshake(server)
        .await
        .context(H2Error {})
        .context(EndpointError {
            endpoint: EndpointType::Server,
            scenario: "server handshake",
        })?;

    // The connection futures are responsible for driving the network communication.
    // Spawn them into a new task to take care of that.
    tokio::spawn({
        let uuid = details.uuid;
        async move {
            match p2s_connection.await {
                Ok(..) => {}
                Err(e) => error!("Server connection failed for connection {}; {}", uuid, e),
            }
        }
    });

    ui.send(SessionEvent::NewConnection(NewConnectionEvent {
        uuid: details.uuid,
        protocol_stack: details.protocol_stack,
        client_addr,
        timestamp: SystemTime::now(),
    }))
    .unwrap();

    let connection_result = handle_connection::<TClient>(
        c2p_connection,
        p2s_send_request,
        details.uuid,
        details.opaque_redirect,
        options,
        ui.clone(),
    );
    let connection_result = connection_result.await;

    // Once the ´while client_connection.accept()` loop ends, the connection will close (or
    // alternatively an error happened and we'll terminate it). The final status value depends
    // on whether there was an error or not.
    ui.send(SessionEvent::ConnectionDone(ConnectionDoneEvent {
        uuid: details.uuid,
        status: match connection_result {
            Ok(_) => Status::Succeeded,
            Err(_) => Status::Failed,
        },
        timestamp: SystemTime::now(),
    }))
    .unwrap();
    connection_result
}

pub async fn handle_connection<TClient>(
    mut c2p_connection: server::Connection<TClient, Bytes>,
    p2s_send_request: SendRequest<bytes::Bytes>,
    connection_uuid: Uuid,
    authority: Option<String>,
    options: Arc<ConnectionOptions>,
    ui: Sender<SessionEvent>,
) -> Result<()>
where
    TClient: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Wait for the handshake to finish.
    let mut p2s_send_request =
        p2s_send_request
            .ready()
            .await
            .context(H2Error {})
            .context(EndpointError {
                endpoint: EndpointType::Server,
                scenario: "starting stream",
            })?;

    // The c2p_connection will produce individual HTTP request that we'll accept.
    // These requests will be handled in parallel by spawning them into their own
    // tasks.
    while let Some(request) = c2p_connection.accept().await {
        let (c2p_request, p2c_response) = request.context(H2Error {}).context(EndpointError {
            endpoint: EndpointType::Client,
            scenario: "processing request",
        })?;
        log::debug!("Request: {:?}", c2p_request);

        let request_future = resolve_request(
            connection_uuid,
            authority.clone(),
            options.clone(),
            c2p_request,
            p2c_response,
            &mut p2s_send_request,
            ui.clone(),
        );
        let request = request_future.await?;

        let ui = ui.clone();
        tokio::spawn(async move {
            let ui = ui;
            match request.execute(ui).await {
                Ok(_) => {}
                Err(e) => error!("Request error for request {}; {}", connection_uuid, e),
            }
        });
    }

    Ok(())
}

async fn resolve_request(
    connection_uuid: Uuid,
    authority: Option<String>,
    options: Arc<ConnectionOptions>,
    c2p_request: Request<RecvStream>,
    p2c_response: SendResponse<Bytes>,
    p2s_send_request: &mut SendRequest<Bytes>,
    ui: Sender<SessionEvent>,
) -> Result<ProxyRequest>
{
    let (c2p_head, c2p_stream) = c2p_request.into_parts();
    let request_uuid = Uuid::new_v4();
    ui.send(SessionEvent::NewRequest(NewRequestEvent {
        connection_uuid,
        uuid: request_uuid,
        uri: c2p_head.uri.clone(),
        method: c2p_head.method.clone(),
        headers: c2p_head.headers.clone(),
        timestamp: SystemTime::now(),
    }))
    .unwrap();

    let handler = match &options.script_host {
        None => scripting::Handler::Forward,
        Some(script_host) => scripting::http2::on_request(script_host.clone(), &c2p_head)
            .await
            .context(ScriptError {
                script: "on_request",
            })?,
    };

    let data = RequestData {
        connection_uuid,
        request_uuid,
    };
    let kind = match handler {
        scripting::Handler::Intercept(f) => {
            ProxyRequestKind::Intercept(InterceptProxyRequest::new(
                &data,
                authority,
                c2p_head,
                c2p_stream,
                p2c_response,
                p2s_send_request,
                f,
            )?)
        }
        scripting::Handler::StaticResponse(response, bytes) => {
            let (response, trailers) = response.split()?;
            let response = response.into_parts().0;
            ProxyRequestKind::Static(StaticProxyRequest::new(
                c2p_stream,
                p2c_response,
                response,
                bytes.map(|b| Bytes::from(b.into_vec())),
                trailers,
            )?)
        }
        scripting::Handler::Forward => ProxyRequestKind::Forward(ForwardProxyRequest::new(
            &data,
            authority,
            c2p_head,
            c2p_stream,
            p2c_response,
            p2s_send_request,
        )?),
    };

    Ok(ProxyRequest { data, kind })
}

struct ProxyRequest
{
    data: RequestData,
    kind: ProxyRequestKind,
}

#[derive(Clone, Debug)]
pub struct RequestData
{
    pub request_uuid: Uuid,
    pub connection_uuid: Uuid,
}

enum ProxyRequestKind
{
    Forward(ForwardProxyRequest),
    Static(StaticProxyRequest),
    Intercept(InterceptProxyRequest),
}

impl ProxyRequest
{
    pub async fn execute(self, ui: Sender<SessionEvent>) -> Result<()>
    {
        let execute_ui = ui.clone();
        let request_uuid = self.data.request_uuid;
        let r = match self.kind {
            ProxyRequestKind::Forward(r) => r.execute(self.data, execute_ui).await,
            ProxyRequestKind::Static(r) => r.execute(self.data, execute_ui).await,
            ProxyRequestKind::Intercept(r) => r.execute(self.data, execute_ui).await,
        };

        ui.send(SessionEvent::RequestDone(RequestDoneEvent {
            uuid: request_uuid,
            status: match is_fatal_error(&r) {
                true => Status::Failed,
                false => Status::Succeeded,
            },
            timestamp: SystemTime::now(),
        }))
        .unwrap();

        r
    }
}

struct ForwardProxyRequest
{
    c2p_stream: RecvStream,
    p2c_response: SendResponse<Bytes>,
    p2s_stream: SendStream<Bytes>,
    s2p_response: ResponseFuture,
}

impl ForwardProxyRequest
{
    pub fn new(
        request_data: &RequestData,
        authority: Option<String>,
        c2p_head: http::request::Parts,
        c2p_stream: RecvStream,
        p2c_response: SendResponse<Bytes>,
        p2s_send_request: &mut client::SendRequest<Bytes>,
    ) -> Result<ForwardProxyRequest>
    {
        // Check if we'll need to overwrite the authority.
        let p2s_request = setup_request(
            &request_data.request_uuid,
            &request_data.connection_uuid,
            c2p_head,
            authority,
        )?;

        // Set up a server request.
        let (s2p_response, p2s_stream) = p2s_send_request
            .send_request(p2s_request, c2p_stream.is_end_stream())
            .context(H2Error {})
            .context(EndpointError {
                endpoint: EndpointType::Server,
                scenario: "sending request",
            })?;

        Ok(ForwardProxyRequest {
            c2p_stream,
            p2c_response,
            p2s_stream,
            s2p_response,
        })
    }

    pub async fn execute(self, request_data: RequestData, ui: Sender<SessionEvent>) -> Result<()>
    {
        // Acquire futures that are responsible for streaming the request and the response. These
        // are set up in their own futures to allow parallel request/response streaming to occur.

        // Set up streaming the request to the server.
        //
        // The client request might have ended already if the client didn't need to stream a
        // request body. We'll set up the future here anyway just to keep things consistent and
        // easier to manage without having to special case the is_end_stream somewhere else.
        let request_future = request_future(
            self.c2p_stream,
            ui.clone(),
            self.p2s_stream,
            request_data.request_uuid,
        );

        // Set up streaming the response to the client.
        //
        // This is done in its own async block, since it's the pipe_stream async call that we'll
        // want to happen in parallel, but there's a good chance the server won't send the
        // response before the request stream has proceeded at least some. (Most likely the server
        // will require that stream to proceed in full, unless the call is some sort of a streaming
        // call.
        let mut p2c_response = self.p2c_response;
        let s2p_response = self.s2p_response;
        let connection_uuid = request_data.connection_uuid;
        let ui_temp = ui.clone();
        let request_uuid = request_data.request_uuid;
        let response_future = async move {
            let ui = ui_temp;
            let response = s2p_response
                .await
                .context(H2Error {})
                .context(EndpointError {
                    endpoint: EndpointType::Server,
                    scenario: "waiting for response",
                })?;

            let (response_head, response_body) = response.into_parts();
            ui.send(SessionEvent::NewResponse(NewResponseEvent {
                uuid: request_uuid,
                connection_uuid,
                timestamp: SystemTime::now(),
                headers: response_head.headers.clone(),
            }))
            .unwrap();

            let response = Response::from_parts(response_head, ());

            log::debug!(
                "{}: Sending response to client: {:?}",
                request_uuid,
                response
            );
            let mut p2c_stream = p2c_response
                .send_response(response, response_body.is_end_stream())
                .context(H2Error {})
                .context(EndpointError {
                    endpoint: EndpointType::Client,
                    scenario: "sending response",
                })?;

            // The server might have sent all the details in the headers, at which point there is
            // no body present. Check for this scenario here.
            if response_body.is_end_stream() {
                Ok(None)
            } else {
                log::info!("{}: Server stream starting", request_uuid);
                let trailers = pipe_stream(
                    response_body,
                    &mut p2c_stream,
                    ui,
                    request_uuid,
                    RequestPart::Response,
                )
                .await?;
                log::info!("{}: Server stream ended", request_uuid);

                if let Some(trailers) = trailers.clone() {
                    log::info!("{}: Trailers: {:?}", request_uuid, trailers);
                    p2c_stream
                        .send_trailers(trailers)
                        .context(H2Error {})
                        .context(EndpointError {
                            endpoint: EndpointType::Server,
                            scenario: "sending trailers",
                        })?;
                }

                Ok(trailers)
            }
        }
        .then({
            let ui = ui.clone();
            move |r| notify_message_done(ui, request_uuid, r, RequestPart::Response)
        });

        // Now handle both futures in parallel.
        let r = try_join!(request_future, response_future);
        r.map(|_| ())
    }
}

struct StaticProxyRequest
{
    c2p_stream: RecvStream,
    p2c_response: SendResponse<Bytes>,
    response_head: http::response::Parts,
    response_content: Option<Bytes>,
    response_trailers: HeaderMap<HeaderValue>,
}

impl StaticProxyRequest
{
    pub fn new(
        c2p_stream: RecvStream,
        p2c_response: SendResponse<Bytes>,
        response_head: http::response::Parts,
        response_content: Option<Bytes>,
        response_trailers: HeaderMap<HeaderValue>,
    ) -> Result<StaticProxyRequest>
    {
        Ok(StaticProxyRequest {
            c2p_stream,
            p2c_response,
            response_head,
            response_content,
            response_trailers,
        })
    }

    pub async fn execute(
        mut self,
        request_data: RequestData,
        ui: Sender<SessionEvent>,
    ) -> Result<()>
    {
        let response = Response::from_parts(self.response_head, ());
        let mut p2c_stream = self
            .p2c_response
            .send_response(response, false)
            .context(H2Error {})
            .context(EndpointError {
                endpoint: EndpointType::Client,
                scenario: "sending static response",
            })?;

        if let Some(bytes) = self.response_content {
            // Send a notification to the UI.
            ui.send(SessionEvent::MessageData(MessageDataEvent {
                uuid: request_data.request_uuid,
                data: bytes.clone(),
                part: RequestPart::Response,
            }))
            .unwrap();

            p2c_stream
                .send_data(bytes, false)
                .context(H2Error {})
                .context(EndpointError {
                    endpoint: EndpointType::Client,
                    scenario: "sending static response content",
                })?;
        }

        p2c_stream
            .send_trailers(self.response_trailers)
            .context(H2Error {})
            .context(EndpointError {
                endpoint: EndpointType::Client,
                scenario: "sending static trailers",
            })?;

        while let Some(data) = self.c2p_stream.data().await {
            let data = data.context(H2Error {}).context(EndpointError {
                endpoint: EndpointType::Client,
                scenario: "reading content",
            })?;
            ui.send(SessionEvent::MessageData(MessageDataEvent {
                uuid: request_data.request_uuid,
                data,
                part: RequestPart::Request,
            }))
            .unwrap();
        }

        Ok(())
    }
}

struct InterceptProxyRequest
{
    c2p_stream: RecvStream,
    p2c_response: SendResponse<Bytes>,
    p2s_stream: SendStream<Bytes>,
    s2p_response: ResponseFuture,
    handler: runestick::Function,
}

/// TODO: Check the sanity here.
///
/// ruststick::Function is !Send automagically due to pointer stuff.
/// Not sure if intended.
unsafe impl Send for InterceptProxyRequest {}

impl InterceptProxyRequest
{
    pub fn new(
        data: &RequestData,
        authority: Option<String>,
        c2p_head: http::request::Parts,
        c2p_stream: RecvStream,
        p2c_response: SendResponse<Bytes>,
        p2s_send_request: &mut client::SendRequest<Bytes>,
        handler: runestick::Function,
    ) -> Result<InterceptProxyRequest>
    {
        // Check if we'll need to overwrite the authority.
        let p2s_request = setup_request(
            &data.request_uuid,
            &data.connection_uuid,
            c2p_head,
            authority,
        )?;

        // Set up a server request.
        let (s2p_response, p2s_stream) = p2s_send_request
            .send_request(p2s_request, c2p_stream.is_end_stream())
            .context(H2Error {})
            .context(EndpointError {
                endpoint: EndpointType::Server,
                scenario: "sending request",
            })?;

        Ok(InterceptProxyRequest {
            c2p_stream,
            p2c_response,
            p2s_stream,
            s2p_response,
            handler,
        })
    }

    pub async fn execute(self, request_data: RequestData, ui: Sender<SessionEvent>) -> Result<()>
    {
        let future = {
            let handler = self.handler;
            crate::scripting::http2::on_intercept(
                request_data,
                handler,
                ui,
                self.c2p_stream,
                self.p2c_response,
                self.p2s_stream,
                self.s2p_response,
            )
        };
        future.await.context(ScriptError {
            script: "[Handler::Intercept]",
        })?;

        Ok(())
    }
}

async fn request_future(
    c2p_stream: RecvStream,
    ui: Sender<SessionEvent>,
    mut p2s_stream: SendStream<Bytes>,
    request_uuid: Uuid,
) -> Result<()>
{
    let trailers = if c2p_stream.is_end_stream() {
        Ok(None)
    } else {
        let ui = ui.clone();
        let trailers = pipe_stream(
            c2p_stream,
            &mut p2s_stream,
            ui,
            request_uuid,
            RequestPart::Request,
        )
        .await?;

        if let Some(trailers) = trailers.clone() {
            p2s_stream
                .send_trailers(trailers)
                .context(H2Error {})
                .context(EndpointError {
                    endpoint: EndpointType::Server,
                    scenario: "sending trailers",
                })?;
        }
        Ok(trailers)
    };

    let ui = ui.clone();
    notify_message_done(ui, request_uuid, trailers, RequestPart::Request).await
}

async fn pipe_stream(
    mut source: RecvStream,
    target: &mut SendStream<Bytes>,
    ui: Sender<SessionEvent>,
    request_uuid: Uuid,
    part: RequestPart,
) -> Result<Option<HeaderMap>>
{
    while let Some(data) = source.data().await {
        let b = match data {
            Ok(b) => b,
            Err(e) => {
                if let Some(reason) = e.reason() {
                    target.send_reset(reason);
                }

                return Err(e).context(H2Error {}).context(EndpointError {
                    endpoint: EndpointType::Client,
                    scenario: "reading content",
                });
            }
        };

        // Send a notification to the UI.
        ui.send(SessionEvent::MessageData(MessageDataEvent {
            uuid: request_uuid,
            data: b.clone(),
            part,
        }))
        .unwrap();

        let size = b.len();
        target
            .send_data(b, source.is_end_stream())
            .context(H2Error {})
            .context(EndpointError {
                endpoint: EndpointType::Server,
                scenario: "writing content",
            })?;
        source.flow_control().release_capacity(size).unwrap();
    }

    let t = source
        .trailers()
        .await
        .context(H2Error {})
        .context(EndpointError {
            endpoint: EndpointType::Client,
            scenario: "receiving trailers",
        })?;
    Ok(t)
}

async fn notify_message_done(
    ui: Sender<SessionEvent>,
    request_uuid: Uuid,
    r: Result<Option<HeaderMap>>,
    part: RequestPart,
) -> Result<()>
{
    match r {
        Ok(trailers) => ui
            .send(SessionEvent::MessageDone(MessageDoneEvent {
                uuid: request_uuid,
                part,
                status: Status::Succeeded,
                timestamp: SystemTime::now(),
                trailers,
            }))
            .unwrap(),
        Err(e) => {
            ui.send(SessionEvent::MessageDone(MessageDoneEvent {
                uuid: request_uuid,
                part,
                status: Status::Succeeded,
                timestamp: SystemTime::now(),
                trailers: None,
            }))
            .unwrap();
            return Err(e);
        }
    }
    Ok(())
}

fn is_fatal_error<S>(r: &Result<S, Error>) -> bool
{
    match r {
        Ok(_) => false,
        Err(e) => match e {
            Error::EndpointError {
                source: EndpointErrorKind::H2Error { source },
                ..
            } => match source.reason() {
                Some(Reason::NO_ERROR) => false,
                Some(Reason::CANCEL) => false,
                _ => true,
            },
            _ => true,
        },
    }
}

fn setup_request(
    request_uuid: &Uuid,
    connection_uuid: &Uuid,
    request: http::request::Parts,
    authority: Option<String>,
) -> Result<http::request::Request<()>>
{
    let mut request = request;
    if let Some(authority) = authority {
        log::debug!(
            "{}:{} - Replacing authority in URI {} with {}",
            connection_uuid,
            request_uuid,
            request.uri,
            authority
        );
        let mut uri_parts = request.uri.into_parts();
        uri_parts.authority = Some(
            http::uri::Authority::from_maybe_shared(authority)
                .context(UriError {})
                .context(ConfigurationError {
                    reason: "invalid target server",
                })?,
        );
        request.uri = http::uri::Uri::from_parts(uri_parts)
            .context(UriPartsError {})
            .context(ConfigurationError {
                reason: "invalid target server",
            })?;
    }

    Ok(Request::from_parts(request, ()))
}
