use snafu::ResultExt;
use std::path::Path;
use std::sync::Arc;

use super::{ContextError, Error};

#[derive(Clone)]
pub struct ScriptHost
{
    pub unit: Arc<runestick::Unit>,
    pub context: Arc<runestick::Context>,
}

impl ScriptHost
{
    pub fn new<'a>(script: &'a str) -> Result<Self, Error>
    {
        let mut sources = rune::Sources::new();
        sources.insert(
            runestick::Source::from_path(Path::new(script)).map_err(|_| Error::FileLoadError {
                file: script.to_string(),
            })?,
        );

        log::info!("Sources: {:?}", sources);

        let mut context = rune_modules::default_context().context(ContextError {})?;
        let mut module = runestick::Module::new(&["proxide"]);
        super::handlers::register(&mut module);
        super::http2::register(&mut module);
        context.install(&module).context(ContextError {})?;

        let options = rune::Options::default();
        let mut errors = rune::Errors::new();
        let mut warnings = rune::Warnings::new();
        let unit = match rune::load_sources(
            &context,
            &options,
            &mut sources,
            &mut errors,
            &mut warnings,
        ) {
            Ok(unit) => unit,
            Err(rune::LoadSourcesError) => return Err(Error::SourceLoadError { warnings, errors }),
        };

        // No error, we'll still want to log the warnings.
        if !warnings.is_empty() {
            log::warn!("Following warnings were encountered when loading the scripts:");
            for w in &warnings {
                log::warn!("{}", w);
            }
        }

        Ok(ScriptHost {
            unit: Arc::new(unit),
            context: Arc::new(context),
        })
    }
}
