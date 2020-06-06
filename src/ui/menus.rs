use crossterm::event::KeyCode;
use tui::widgets::{List, ListState, Paragraph, Text};

use super::controls::filters::{ConnectionFilter, FilterState, FilterType, PathFilter};
use super::views::prelude::*;
use crate::session::EncodedRequest;
use crate::ui::chords::{ChordResult, ChordState};
use crate::ui::toast;

pub struct RequestFilterMenu
{
    chord: Option<ChordState>,
    list_state: ListState,
    locked_selection: Option<(FilterType, String)>,
}

enum Dir
{
    Previous,
    Next,
}

impl RequestFilterMenu
{
    pub fn new() -> RequestFilterMenu
    {
        Self {
            chord: Default::default(),
            list_state: Default::default(),
            locked_selection: None,
        }
    }

    pub fn on_input<B: Backend>(
        &mut self,
        filter: &mut FilterState<EncodedRequest>,
        _ctx: &UiContext,
        request: Option<&EncodedRequest>,
        e: CTEvent,
    ) -> Option<HandleResult<B>>
    {
        if let Some(chord) = &mut self.chord {
            match chord.handle(e) {
                ChordResult::State(s) => {
                    match s {
                        "ss" => (),
                        "sf" => (),
                        other => {
                            toast::show_error(format!("Unknown chord '{}'", other));
                        }
                    }
                    self.chord = None;
                    return Some(HandleResult::Update);
                }
                ChordResult::Cancel => {
                    self.chord = None;
                    return Some(HandleResult::Update);
                }
                ChordResult::Ignore => (),
            }
        }

        if let CTEvent::Key(key) = e {
            match key.code {
                KeyCode::Esc => {
                    return Some(HandleResult::ExitView);
                }
                KeyCode::Char('X') => filter.clear_filters(),
                KeyCode::Char('F') => filter.use_filter = !filter.use_filter,
                KeyCode::Char('s') => self.chord = Some(ChordState::new('s')),
                KeyCode::Char('c') => return self.on_connection_filter(filter, request),
                KeyCode::Char('p') => return self.on_path_filter(filter, request),

                KeyCode::Char('k') | KeyCode::Up => self.move_selection(filter, Dir::Previous),
                KeyCode::Char('j') | KeyCode::Down => self.move_selection(filter, Dir::Next),
                _ => return None,
            }
        }

        Some(HandleResult::Update)
    }

    fn move_selection(&mut self, filter: &mut FilterState<EncodedRequest>, dir: Dir)
    {
        // If the selection was locked, find the lock.
        let locked_idx = match &self.locked_selection {
            None => None,
            Some((ty, item)) => match filter.filters.get(&ty) {
                None => None,
                Some(current_filter) => {
                    let offset = current_filter
                        .values()
                        .enumerate()
                        .find(|(_, v)| v.get_key() == *item)
                        .map(|(i, _)| i)
                        .unwrap_or(0);

                    let mut skipped = 0;
                    for (k, v) in filter.filters.iter() {
                        if k == ty {
                            break;
                        }
                        skipped += 1 + v.len();
                    }

                    Some(skipped + offset)
                }
            },
        };

        let total = filter.filters.values().flat_map(|v| v.values()).count()
            + filter.filters.values().count();
        log::trace!("RequestFilterMenu::move_selection; total={}", total);
        if total == 0 {
            self.list_state.select(None);
            return;
        }

        let selection = locked_idx.unwrap_or_else(|| match dir {
            Dir::Previous => match self.list_state.selected() {
                Some(s) if s > 0 => s - 1,
                _ => 0,
            },
            Dir::Next => match self.list_state.selected() {
                Some(s) => s + 1,
                None => usize::MAX,
            },
        });

        self.list_state.select(Some(selection.min(total - 1)))
    }

    /*
    fn get_selection(
        &mut self,
        filter: &mut FilterState<EncodedRequest>,
    ) -> Option<(FilterType, usize)>
    {
        let selection =
    }
    */

    pub fn draw<B: Backend>(
        &mut self,
        filter: &FilterState<EncodedRequest>,
        ctx: &UiContext,
        request: Option<&EncodedRequest>,
        f: &mut Frame<B>,
        chunk: Rect,
        is_active: bool,
    )
    {
        let block = create_block("Request [F]ilters/[H]ighlights");
        f.render_widget(block, chunk);

        let sub_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints([Constraint::Length(10), Constraint::Percentage(100)].as_ref())
            .split(block.inner(chunk));

        let mut keys_text = vec![Text::raw("\n")];
        if let Some(request) = request {
            if let Some(conn) = ctx
                .data
                .connections
                .get_by_uuid(request.request_data.connection_uuid)
            {
                keys_text.push(Text::raw(format!(
                    "[c]: Toggle filter by connection: {}\n",
                    conn.client_addr
                )));
            }
            keys_text.push(Text::raw(format!(
                "[p]: Toggle filter by path: {}\n",
                request.request_data.uri.path()
            )));
        }
        keys_text.extend(vec![
            Text::raw("[s?]: Toggle filter by status\n"),
            Text::raw(" - [ss]: Status Success\n"),
            Text::raw(" - [sf]: Status Fail\n"),
            Text::raw("\n"),
            Text::raw("[F]: Toggle visibility of filtered items\n"),
            Text::raw("[X]: Remove all filters\n"),
            Text::raw("[d]: Toggle disable selected filter(s)\n"),
            Text::raw("[D]: Delete selected filter(s)\n"),
        ]);

        let keys_paragraph = Paragraph::new(keys_text.iter());
        f.render_widget(keys_paragraph, sub_chunks[0]);

        let mut filter_items = vec![];
        for (ty, items) in &filter.filters {
            filter_items.push(Text::raw(format!("{} filters:", ty)));
            for i in items.values() {
                filter_items.push(Text::raw(format!(" - {}", i.to_string(ctx))));
            }
        }

        // Ensure something is selected if there are items to select.
        if !filter_items.is_empty() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0))
        }

        let filter_list = List::new(filter_items.into_iter())
            .block(create_control_block("Current filters", is_active))
            .highlight_symbol("> ");
        f.render_stateful_widget(filter_list, sub_chunks[1], &mut self.list_state)
    }

    fn on_connection_filter<B: Backend>(
        &self,
        filter: &mut FilterState<EncodedRequest>,
        request: Option<&EncodedRequest>,
    ) -> Option<HandleResult<B>>
    {
        request.map(|req| {
            let connection = req.request_data.connection_uuid;
            filter.add_filter(Box::new(ConnectionFilter { connection }));
            HandleResult::Update
        })
    }

    fn on_path_filter<B: Backend>(
        &self,
        filter: &mut FilterState<EncodedRequest>,
        request: Option<&EncodedRequest>,
    ) -> Option<HandleResult<B>>
    {
        request.map(|req| {
            filter.add_filter(Box::new(PathFilter {
                path: req.request_data.uri.path().to_string(),
            }));
            HandleResult::Update
        })
    }
}
