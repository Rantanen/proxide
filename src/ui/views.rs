pub mod prelude;
use prelude::*;

mod main_view;
pub use main_view::MainView;
mod message_view;
pub use message_view::MessageView;

mod callstack_view;
pub use callstack_view::CallstackView;

pub trait View<B: Backend>
{
    fn draw(&mut self, ctx: &UiContext, f: &mut Frame<B>, chunk: Rect);
    fn on_input(&mut self, ctx: &UiContext, e: &CTEvent, size: Rect) -> Option<HandleResult<B>>;
    fn on_change(&mut self, ctx: &UiContext, change: &SessionChange) -> bool;
    fn help_text(&self, state: &UiContext, size: Rect) -> String;
    fn transparent(&self) -> bool
    {
        false
    }
}
