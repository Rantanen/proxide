pub mod prelude;
use prelude::*;

mod main_view;
pub use main_view::MainView;
mod details_view;
pub use details_view::DetailsView;
mod message_view;
pub use message_view::MessageView;

pub trait View<B: Backend>
{
    fn draw(&mut self, ctx: &UiContext, f: &mut Frame<B>, chunk: Rect);
    fn on_input(&mut self, ctx: &UiContext, e: CTEvent, size: Rect) -> HandleResult<B>;
    fn on_change(&self, ctx: &UiContext, change: &SessionChange) -> bool;
    fn help_text(&self, state: &UiContext, size: Rect) -> String;
}
