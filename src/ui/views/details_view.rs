use super::prelude::*;
use tui::widgets::Paragraph;
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct DetailsView
{
    request: Uuid,
}
impl<B: Backend> View<B> for DetailsView
{
    fn draw(&mut self, ctx: &UiContext, f: &mut Frame<B>, chunk: Rect)
    {
        self.draw_control(self.request, ctx, f, chunk, false)
    }

    fn on_input(&mut self, _session: &UiContext, _e: CTEvent, _size: Rect) -> HandleResult<B>
    {
        HandleResult::Ignore
    }

    fn on_change(&mut self, _ctx: &UiContext, state_changed: &SessionChange) -> bool
    {
        match state_changed {
            SessionChange::NewConnection { .. } | SessionChange::NewRequest { .. } => false,
            SessionChange::Request { request }
            | SessionChange::NewMessage { request, .. }
            | SessionChange::Message { request, .. } => *request == self.request,
        }
    }

    fn help_text(&self, _session: &UiContext, _size: Rect) -> String
    {
        String::new()
    }
}

impl DetailsView
{
    pub fn show(&mut self, request: Uuid)
    {
        self.request = request;
    }

    pub fn draw_control<B: Backend>(
        &mut self,
        request: Uuid,
        ctx: &UiContext,
        f: &mut Frame<B>,
        chunk: Rect,
        is_active: bool,
    )
    {
        let request = match ctx.data.requests.get_by_uuid(request) {
            Some(r) => r,
            None => return,
        };

        let block = create_block("[D]etails", is_active);
        f.render_widget(block, chunk);

        let details_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints([Constraint::Length(6), Constraint::Percentage(50)].as_ref())
            .split(block.inner(chunk));
        let mut c = details_chunks[1];
        c.x -= 1;
        c.width += 2;
        c.height += 1;
        let req_resp_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .margin(0)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(block.inner(c));

        let duration = match request.request_data.end_timestamp {
            None => "(Pending)".to_string(),
            Some(end) => format_duration(end - request.request_data.start_timestamp),
        };

        let text = vec![
            Text::raw("\n"),
            Text::raw(format!(" Method:     {}\n", request.request_data.method)),
            Text::raw(format!(" URI:        {}\n", request.request_data.uri)),
            Text::raw(format!(
                " Timestamp:  {}\n",
                request.request_data.start_timestamp.to_string()
            )),
            Text::raw(format!(
                " Status:     {}\n",
                request.request_data.status.to_string()
            )),
            Text::raw(format!(" Duration:   {}\n", duration)),
        ];
        let details = Paragraph::new(text.iter());
        f.render_widget(details, details_chunks[0]);

        request.request_msg.draw(
            &ctx.runtime.decoder_factories,
            &request.request_data,
            "Re[q]uest Data",
            f,
            req_resp_chunks[0],
            false,
            0,
        );
        request.response_msg.draw(
            &ctx.runtime.decoder_factories,
            &request.request_data,
            "Re[s]ponse Data",
            f,
            req_resp_chunks[1],
            false,
            0,
        );
    }
}
