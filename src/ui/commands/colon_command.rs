use crate::ui::state::UiContext;
use crate::ui::toast;
use chrono::prelude::*;
use clap::{Arg, ArgMatches, Command};
use core::cell::RefCell;
use tui::backend::Backend;

use super::Executable;
use crate::session;
use crate::ui::state::HandleResult;

thread_local! {
    pub static CMD_API: RefCell<Command> = RefCell::new(create_app());
}

pub fn create_app() -> Command
{
    Command::new("CMD")
        .no_binary_name(true)
        .subcommand(Command::new("quit").alias("q"))
        .subcommand(Command::new("clear"))
        .subcommand(
            Command::new("export")
                .alias("w")
                .arg(Arg::new("file").index(1).value_name("file").required(false))
                .arg(
                    Arg::new("format")
                        .short('f')
                        .long("format")
                        .value_parser(["msgpack", "json"]),
                ),
        )
}

pub struct ColonCommand;
impl<B: Backend> Executable<B> for ColonCommand
{
    fn execute(&self, cmd: &str, ctx: &mut UiContext) -> Option<HandleResult<B>>
    {
        let words = match shell_words::split(cmd) {
            Ok(w) => w,
            Err(e) => {
                toast::show_error(format!("Failed to parse command:\n{}", e));
                return None;
            }
        };

        CMD_API.with(|api| {
            let mut cmd_borrow = api.borrow_mut();
            let matches = match cmd_borrow.try_get_matches_from_mut(words) {
                Ok(matches) => matches,
                Err(e) => {
                    toast::show_error(format!("Failed to parse command:\n{}", e));
                    return None;
                }
            };
            execute_matches(matches, ctx)
        })
    }
}

pub fn execute_matches<B: Backend>(s: ArgMatches, ctx: &mut UiContext) -> Option<HandleResult<B>>
{
    match s.subcommand() {
        Some(("quit", _)) => Some(HandleResult::Quit),
        Some(("clear", _)) => clear_session(ctx),
        Some(("export", m)) => export_session(ctx, m),
        Some((cmd, _)) => {
            toast::show_error(format!("Unknown command: {}", cmd));
            None
        }
        None => {
            toast::show_error("No command specified");
            None
        }
    }
}

pub fn clear_session<B: Backend>(ctx: &mut UiContext) -> Option<HandleResult<B>>
{
    ctx.data.requests = Default::default();
    ctx.data.connections = Default::default();
    Some(HandleResult::Update)
}

pub fn export_session<B: Backend>(ctx: &UiContext, matches: &ArgMatches)
    -> Option<HandleResult<B>>
{
    let filename = matches
        .get_one::<String>("file")
        .cloned()
        .unwrap_or_else(|| format!("session-{}.bin", Local::now().format("%Y-%m-%d_%H%M%S")));

    let format = match matches.get_one::<String>("format").map(String::as_str) {
        Some("json") => session::serialization::OutputFormat::Json,
        _ => session::serialization::OutputFormat::MessagePack,
    };

    match ctx.data.write_to_file(&filename, format) {
        Ok(_) => toast::show_message(format!("Exported session to '{}'", filename)),
        Err(e) => toast::show_error(e.to_string()),
    }

    None
}

#[cfg(test)]
mod test
{
    use super::*;

    fn parse(cmd: &str) -> Result<ArgMatches, clap::Error>
    {
        create_app().try_get_matches_from(shell_words::split(cmd).unwrap())
    }

    #[test]
    fn command_definition_is_valid()
    {
        create_app().debug_assert();
    }

    #[test]
    fn commands_and_aliases()
    {
        for (cmd, expected) in [
            ("quit", "quit"),
            ("q", "quit"),
            ("clear", "clear"),
            ("w", "export"),
        ] {
            assert_eq!(
                parse(cmd).unwrap().subcommand_name(),
                Some(expected),
                "{}",
                cmd
            );
        }
        assert!(parse("frobnicate").is_err());
    }

    #[test]
    fn export_options()
    {
        let m = parse("export 'my file.json' -f json").unwrap();
        let m = m.subcommand_matches("export").unwrap();
        assert_eq!(m.get_one::<String>("file").unwrap(), "my file.json");
        assert_eq!(m.get_one::<String>("format").unwrap(), "json");

        let m = parse("w").unwrap();
        let m = m.subcommand_matches("export").unwrap();
        assert_eq!(m.get_one::<String>("file"), None);
        assert_eq!(m.get_one::<String>("format"), None);

        assert!(parse("export -f xml").is_err());
    }
}
