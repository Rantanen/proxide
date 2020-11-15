use snafu::Snafu;

mod handlers;
pub mod host;
pub mod http2;

pub use handlers::Handler;

#[derive(Debug, Snafu)]
pub enum Error
{
    #[snafu(display("Could not load script '{}'", file))]
    FileLoadError
    {
        file: String
    },

    #[snafu(display("Could not load script\n{:#?}\n{:#?}", warnings, errors))]
    SourceLoadError
    {
        warnings: rune::Warnings,
        errors: rune::Errors,
    },

    ContextError
    {
        source: runestick::ContextError
    },

    VmError
    {
        source: runestick::VmError
    },

    ThreadingError
    {
        source: tokio::task::JoinError
    },
}

pub use host::ScriptHost;
