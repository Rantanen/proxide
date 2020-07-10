use crate::ui::state::UiContext;
use crate::ui::toast;
use chrono::prelude::*;
use clap::{App, Arg, ArgMatches, SubCommand};
use core::cell::RefCell;
use tui::backend::Backend;

use super::Executable;
use crate::session;
use crate::ui::state::HandleResult;

thread_local! {
    pub static CMD_API: RefCell<App<'static, 'static>> = RefCell::new(create_app());
}

pub fn create_app() -> App<'static, 'static>
{
    App::new("CMD")
        .setting(clap::AppSettings::NoBinaryName)
        .subcommand(SubCommand::with_name("quit").alias("q"))
        .subcommand(SubCommand::with_name("clear"))
        .subcommand(
            SubCommand::with_name("export")
                .alias("w")
                .arg(
                    Arg::with_name("file")
                        .index(1)
                        .value_name("file")
                        .required(false),
                )
                .arg(
                    Arg::with_name("format")
                        .short("f")
                        .long("format")
                        .takes_value(true)
                        .possible_values(&["msgpack", "json"]),
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
            let matches = match cmd_borrow.get_matches_from_safe_borrow(words) {
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
        ("quit", _) => Some(HandleResult::Quit),
        ("clear", _) => clear_session(ctx),
        ("export", m) => export_session(ctx, m.unwrap()),
        (cmd, _) => {
            toast::show_error(format!("Unknown command: {}", cmd));
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
        .value_of("file")
        .map(|f| f.to_string())
        .unwrap_or_else(|| format!("session-{}.bin", Local::now().format("%Y-%m-%d_%H%M%S")));

    let format = match matches.value_of("format") {
        Some("json") => session::serialization::OutputFormat::Json,
        _ => session::serialization::OutputFormat::MessagePack,
    };

    match ctx.data.write_to_file(&filename, format) {
        Ok(_) => toast::show_message(format!("Exported session to '{}'", filename)),
        Err(e) => toast::show_error(e.to_string()),
    }

    None
}
