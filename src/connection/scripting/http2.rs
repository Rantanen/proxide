use bytes::Bytes;
use futures::Future;
use h2::{client::ResponseFuture, server::SendResponse, RecvStream, SendStream};
use http::{
    header::{HeaderMap, HeaderName, HeaderValue},
    method::Method,
    uri::Uri,
    version::Version,
};
use runestick::Module;
use snafu::ResultExt;
use std::sync::mpsc::Sender;

use super::{handlers::Handler, Error, ScriptHost, VmError};
use crate::connection;
use crate::connection::error::*;
use crate::connection::http2::RequestData;
use crate::session::events::SessionEvent;
use crate::session::events::*;

#[derive(Debug, runestick::Any)]
pub struct ClientRequest
{
    method: Method,
    uri: Uri,
    version: Version,
    headers: HeaderMap<HeaderValue>,
}

#[derive(Debug, runestick::Any)]
pub struct ClientResponse
{
    status: u16,
    headers: HeaderMap<HeaderValue>,
    trailers: HeaderMap<HeaderValue>,
}

#[derive(Debug, runestick::Any)]
pub struct ScriptHeaders
{
    inner: HeaderMap<HeaderValue>,
}

#[derive(Debug, runestick::Any)]
pub struct ScriptRecvStream
{
    pub inner: RecvStream,
    pub endpoint: EndpointType,
    pub ui: Sender<SessionEvent>,
}

#[derive(Debug, runestick::Any)]
pub struct ScriptSendResponse
{
    pub data: RequestData,
    pub inner: SendResponse<Bytes>,
    pub endpoint: EndpointType,
    pub ui: Sender<SessionEvent>,
}

#[derive(Debug, runestick::Any)]
pub struct ScriptSendStream
{
    pub data: RequestData,
    pub inner: SendStream<Bytes>,
    pub endpoint: EndpointType,
    pub ui: Sender<SessionEvent>,
}

#[derive(Debug, runestick::Any)]
pub struct ScriptResponseFuture
{
    pub inner: ResponseFuture,
    pub endpoint: EndpointType,
    pub ui: Sender<SessionEvent>,
}

impl ClientRequest
{
    fn register(module: &mut runestick::Module)
    {
        module
            .ty::<Self>()
            .expect("Failed to register ClientRequest");
        module
            .inst_fn("method", ClientRequest::get_method)
            .expect("Failed to register ClientRequest::get_method");
        module
            .inst_fn("headers", ClientRequest::get_header)
            .expect("Failed to register ClientRequest::get_header");
    }

    fn get_method(&self) -> String
    {
        self.method.as_str().to_string()
    }

    fn get_header(&self, name: &str) -> Option<String>
    {
        self.headers
            .get(name)
            .and_then(|h| h.to_str().map(|s| s.to_string()).ok())
    }
}

impl ScriptRecvStream
{
    fn register(module: &mut runestick::Module)
    {
        module
            .ty::<Self>()
            .expect("Failed to register ClientRequest");
        module
            .async_inst_fn("data", Self::data)
            .expect("Failed to register ScriptRecvStream::data");
        module
            .async_inst_fn("trailers", Self::trailers)
            .expect("Failed to register ScriptRecvStream::trailers");
        module
            .inst_fn("is_end_stream", Self::is_end_stream)
            .expect("Failed to register ScriptRecvStream::is_end_stream");
    }

    async fn data(&mut self) -> Option<Result<runestick::Bytes, connection::Error>>
    {
        log::trace!("ScriptRecvStream::data");
        let result = match self.inner.data().await {
            None => return None,
            Some(result) => result,
        };

        let bytes = match result {
            Ok(bytes) => bytes,
            Err(e) => {
                return Some(
                    Err(EndpointErrorKind::H2Error { source: e }).context(EndpointError {
                        endpoint: EndpointType::Client,
                        scenario: "ScriptRecvStream::data",
                    }),
                )
            }
        };

        Some(Ok(runestick::Bytes::from_vec(Vec::from(&*bytes))))
    }

    fn is_end_stream(&self) -> bool
    {
        self.inner.is_end_stream()
    }

    async fn trailers(&mut self) -> Option<ScriptHeaders>
    {
        self.inner
            .trailers()
            .await
            .ok()
            .flatten()
            .map(|h| ScriptHeaders { inner: h })
    }
}

impl ScriptSendStream
{
    fn register(module: &mut runestick::Module)
    {
        module
            .ty::<Self>()
            .expect("Failed to register ScriptSendStream");
        module
            .inst_fn("send_data", Self::send_data)
            .expect("Failed to register ScriptSendStream::send_data");
        module
            .inst_fn("send_trailers", Self::send_trailers)
            .expect("Failed to register ScriptSendStream::send_trailers");
    }

    fn send_data(&mut self, data: runestick::Bytes, is_end_stream: bool)
    {
        log::trace!("ScriptSendStream::send_data: {:?}", data);
        let bytes = bytes::Bytes::copy_from_slice(&*data);

        // Send a notification to the UI.
        let part = match self.endpoint {
            EndpointType::Server => crate::connection::RequestPart::Request,
            EndpointType::Client => crate::connection::RequestPart::Response,
        };
        self.ui
            .send(SessionEvent::MessageData(MessageDataEvent {
                uuid: self.data.request_uuid,
                data: bytes.clone(),
                part,
            }))
            .unwrap();

        self.inner
            .send_data(bytes::Bytes::copy_from_slice(&*data), is_end_stream)
            .unwrap();
    }

    fn send_trailers(&mut self, trailers: ScriptHeaders)
    {
        self.inner.send_trailers(trailers.inner).unwrap();
    }
}

impl ScriptSendResponse
{
    fn register(module: &mut runestick::Module)
    {
        module
            .ty::<Self>()
            .expect("Failed to register ScriptSendResponse");
        module
            .inst_fn("send_response", Self::send_response)
            .expect("Failed to register ScriptSendResponse::send_response");
    }

    fn send_response(
        self,
        response: ClientResponse,
        is_end_stream: bool,
    ) -> Result<ScriptSendStream, connection::Error>
    {
        let (response, _) = response.split()?;
        let mut inner = self.inner;
        let data = self.data;
        let endpoint = self.endpoint;
        let ui = self.ui;
        let inner = inner
            .send_response(response, is_end_stream)
            .context(H2Error {})
            .context(EndpointError {
                endpoint,
                scenario: "ScriptSendResponse::send_response",
            })?;

        Ok(ScriptSendStream {
            data,
            inner,
            endpoint,
            ui,
        })
    }
}

impl ScriptResponseFuture
{
    fn register(module: &mut runestick::Module)
    {
        module
            .ty::<Self>()
            .expect("Failed to register ScriptResponseFuture");
        module
            .async_inst_fn("get", Self::get)
            .expect("Failed to register ScriptResponseFuture::get");
    }

    async fn get(self) -> Result<(ClientResponse, ScriptRecvStream), ()>
    {
        let (parts, stream) = self.inner.await.map_err(|_| ())?.into_parts();

        Ok((
            ClientResponse {
                status: parts.status.into(),
                headers: parts.headers,
                trailers: HeaderMap::new(),
            },
            ScriptRecvStream {
                inner: stream,
                endpoint: self.endpoint,
                ui: self.ui,
            },
        ))
    }
}

impl ClientResponse
{
    fn register(module: &mut runestick::Module)
    {
        module
            .ty::<Self>()
            .expect("Failed to register ClientResponse");
        module
            .function(&["ClientResponse", "ok"], Self::ok)
            .expect("Failed to register ClientResponse::ok");
        module
            .inst_fn("set_header", Self::set_header)
            .expect("Failed to register ClientResponse::set_header");
        module
            .inst_fn("set_trailer", Self::set_trailer)
            .expect("Failed to register ClientResponse::set_trailer");
    }

    pub fn ok() -> Self
    {
        Self {
            status: 200,
            headers: Default::default(),
            trailers: Default::default(),
        }
    }

    pub fn set_header(&mut self, name: &str, value: &str) -> Result<(), runestick::Error>
    {
        self.headers.insert(
            HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(value)?,
        );

        Ok(())
    }

    pub fn set_trailer(&mut self, name: &str, value: &str) -> Result<(), runestick::Error>
    {
        self.trailers.insert(
            HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(value)?,
        );

        Ok(())
    }

    pub(crate) fn split(self) -> Result<(http::response::Response<()>, HeaderMap<HeaderValue>)>
    {
        let mut builder = http::response::Builder::new()
            .status(self.status)
            .version(http::version::Version::HTTP_2);

        let headers = builder.headers_mut().expect("Builder was invalid");
        *headers = self.headers;

        Ok((builder.body(()).context(HttpScriptError {})?, self.trailers))
    }
}

pub fn register(module: &mut Module)
{
    ClientRequest::register(module);
    ClientResponse::register(module);
    ScriptRecvStream::register(module);
    ScriptSendStream::register(module);
    ScriptResponseFuture::register(module);
    ScriptSendResponse::register(module);
}

pub async fn on_request(
    script_host: ScriptHost,
    client_head: &'_ http::request::Parts,
) -> Result<Handler, Error>
{
    let method = client_head.method.clone();
    let uri = client_head.uri.clone();
    let version = client_head.version.clone();
    let headers = client_head.headers.clone();

    use runestick::IntoTypeHash;
    let fn_hash = ["on_request"].into_type_hash();
    if script_host.unit.lookup(fn_hash).is_none() {
        // return Ok(Handler::Forward);
    }

    let request = ClientRequest {
        method,
        uri,
        version,
        headers,
    };
    on_request_local(script_host, request).await
}

pub fn on_request_local(
    script_host: ScriptHost,
    request: ClientRequest,
) -> impl Future<Output = Result<Handler, Error>> + Send
{
    async move {
        log::trace!("execute");
        let execution = {
            runestick::Vm::new(script_host.context, script_host.unit)
                .send_execute(&["on_request"], (request,))
                .context(VmError {})?
        };
        let return_value = execution.async_complete().await.context(VmError {})?;

        use runestick::FromValue;
        Handler::from_value(return_value).context(VmError {})
    }
}

pub fn on_intercept(
    data: RequestData,
    f: runestick::Function,
    ui: Sender<SessionEvent>,
    c2p_stream: RecvStream,
    p2c_response: SendResponse<Bytes>,
    p2s_stream: SendStream<Bytes>,
    s2p_response: ResponseFuture,
) -> impl Future<Output = Result<(), Error>> + Send
{
    let sync_fn = f.into_sync().unwrap();
    async move {
        log::trace!("Invoking Handle::Intercept callback");
        let future = sync_fn.async_send_call((
            ScriptRecvStream {
                endpoint: EndpointType::Client,
                inner: c2p_stream,
                ui: ui.clone(),
            },
            ScriptSendResponse {
                data: data.clone(),
                endpoint: EndpointType::Client,
                inner: p2c_response,
                ui: ui.clone(),
            },
            ScriptSendStream {
                data,
                endpoint: EndpointType::Server,
                inner: p2s_stream,
                ui: ui.clone(),
            },
            ScriptResponseFuture {
                endpoint: EndpointType::Server,
                inner: s2p_response,
                ui: ui.clone(),
            },
        ));

        future.await.context(VmError {})?;

        Ok(())
    }
}
