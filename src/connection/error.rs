use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum ConfigurationErrorKind
{
    DNSError
    {
        source: webpki::InvalidDNSNameError,
    },
    UriError
    {
        source: http::uri::InvalidUri,
    },
    UriPartsError
    {
        source: http::uri::InvalidUriParts,
    },
    NoSource {},
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum EndpointErrorKind
{
    IoError
    {
        source: std::io::Error
    },
    ConnectError
    {
        source: httparse::Error
    },
    H2Error
    {
        source: h2::Error
    },
    TLSError
    {
        source: rustls::TLSError
    },

    #[snafu(display("{}", reason))]
    ProxideError
    {
        reason: &'static str
    },
}

#[derive(Debug, Clone, Copy)]
pub enum EndpointType
{
    Client,
    Server,
}

#[derive(Debug, Snafu, runestick::Any)]
#[snafu(visibility(pub(crate)))]
pub enum Error
{
    #[snafu(display("Configuration error: {}", reason))]
    ConfigurationError
    {
        reason: &'static str,
        source: ConfigurationErrorKind,
    },

    #[snafu(display(
        "Error occurred with {:?} endpoint in {}: {}",
        endpoint,
        scenario,
        source
    ))]
    EndpointError
    {
        endpoint: EndpointType,
        scenario: &'static str,
        source: EndpointErrorKind,
    },

    HttpScriptError
    {
        source: http::Error
    },

    #[snafu(display("Error executing '{}' script: {}", script, source))]
    ScriptError
    {
        script: &'static str,
        source: crate::scripting::Error,
    },
}

pub type Result<S, E = Error> = std::result::Result<S, E>;
