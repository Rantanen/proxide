use runestick::{Bytes, Function, Module};

use super::http2::ClientResponse;

#[derive(Debug, runestick::Any)]
pub enum Handler
{
    Forward,
    StaticResponse(ClientResponse, Option<Bytes>),
    Intercept(Function),
}

impl Handler
{
    fn register(module: &mut runestick::Module)
    {
        module.ty::<Self>().expect("Failed to register Handler");
        module
            .function(&["Handler", "Forward"], Self::new_forward)
            .expect("Failed to register Handler::Forward");
        module
            .function(&["Handler", "StaticResponse"], Self::new_static_response)
            .expect("Failed to register Handler::StaticResponse");
        module
            .function(&["Handler", "Intercept"], Self::new_intercept)
            .expect("Failed to register Handler::Intercept");
    }

    fn new_forward() -> Handler
    {
        log::info!("Handler::new_forward");
        Handler::Forward
    }

    fn new_static_response(response: ClientResponse, content: Option<Bytes>) -> Handler
    {
        log::info!("Handler::new_static_response");
        Handler::StaticResponse(response, content)
    }

    fn new_intercept(f: Function) -> Handler
    {
        log::info!("Handler::new_intercept");
        Handler::Intercept(f)
    }
}

pub fn register(module: &mut Module)
{
    Handler::register(module);
}
