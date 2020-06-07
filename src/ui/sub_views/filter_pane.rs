use crossterm::event::KeyCode;
use tui::layout::{Constraint, Direction, Layout};
use tui::style::{Color, Style};
use tui::widgets::{List, ListState, Paragraph, Text};

use crate::ui::prelude::*;

use crate::session::EncodedRequest;
use crate::ui::chords::{ChordResult, ChordState};
use crate::ui::filters::{ConnectionFilter, FilterGroupState, FilterState, FilterType, PathFilter};

pub struct FilterPane
{
    chord: Option<ChordState>,
    selection: Option<(FilterType, Option<String>)>,
}

enum Dir
{
    Previous,
    Next,
}

impl FilterPane
{
    pub fn new() -> FilterPane
    {
        Self {
            chord: Default::default(),
            selection: None,
        }
    }

    pub fn on_global_input<B: Backend>(
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
                KeyCode::Char('s') => self.chord = Some(ChordState::new('s')),
                KeyCode::Char('c') => return self.on_connection_filter(filter, request),
                KeyCode::Char('p') => return self.on_path_filter(filter, request),
                _ => return None,
            }
        }

        Some(HandleResult::Update)
    }

    pub fn on_active_input<B: Backend>(
        &mut self,
        filter: &mut FilterState<EncodedRequest>,
        e: CTEvent,
    ) -> Option<HandleResult<B>>
    {
        if let CTEvent::Key(key) = e {
            match key.code {
                KeyCode::Char('k') | KeyCode::Up => self.move_selection(filter, Dir::Previous),
                KeyCode::Char('j') | KeyCode::Down => self.move_selection(filter, Dir::Next),
                KeyCode::Char('x') => match &self.selection {
                    Some((ft, Some(key))) => filter.remove_filter(*ft, key),
                    Some((ft, None)) => filter.remove_filter_group(*ft),
                    None => (),
                },
                KeyCode::Char('t') => match &self.selection {
                    Some((ft, Some(key))) => filter.toggle_filter(*ft, key),
                    Some((ft, None)) => filter.toggle_filter_group(*ft),
                    None => (),
                },
                _ => return None,
            }
        }

        Some(HandleResult::Update)
    }

    pub fn selected_filter(
        &mut self,
        filter: &FilterState<EncodedRequest>,
    ) -> Option<(FilterType, &str)>
    {
        self.ensure_selection(filter);
        match &self.selection {
            Some((ft, Some(key))) => Some((*ft, key.as_str())),
            _ => None,
        }
    }

    fn move_selection(&mut self, filter: &mut FilterState<EncodedRequest>, dir: Dir)
    {
        let group = self.ensure_selection(filter);
        let selection = match &self.selection {
            Some(s) => s,
            None => return,
        };
        let group = group.expect("Selection did exist but the group didn't");

        let (new_ft, new_group): (FilterType, Option<String>) = match &selection.1 {
            // We're currently have a filter selected instead of being in group selection.
            Some(s) => match dir {
                Dir::Next => {
                    // When moving to the next from a group item, there's a chance we'll end up
                    // in the next group.
                    //
                    // Then if we're moving to the next group, we'll need to make sure that group
                    // exists. If it doesn't, we'll just stay still.
                    match group.next_filter(&s) {
                        Some(f) => (selection.0, Some(f.to_string())),
                        None => match filter.filters.next_group(selection.0) {
                            Some(g) => (g.filter_type, None),
                            None => (selection.0, Some(s.to_string())),
                        },
                    }
                }
                Dir::Previous => {
                    // We had a selection in the current group and we are selecting previous.
                    // At worst we'll be ending up in the group header (prev_filter = None).
                    (selection.0, group.prev_filter(s).map(String::from))
                }
            },

            // We're currently in group selection.
            None => match dir {
                Dir::Next => (selection.0, group.first_key().map(String::from)),
                Dir::Previous => match filter.filters.prev_group(selection.0) {
                    Some(g) => (g.filter_type, g.last_key().map(String::from)),
                    None => (selection.0, None),
                },
            },
        };

        self.selection = Some((new_ft, new_group));
    }

    fn ensure_selection<'a>(
        &mut self,
        filter: &'a FilterState<EncodedRequest>,
    ) -> Option<&'a FilterGroupState<EncodedRequest>>
    {
        let selection = match &self.selection {
            Some(s) => s,
            None => {
                // If there's no previous selection, we can just ensure the first thing is selected
                // and be done.
                let first = filter.filters.first();
                self.selection = first.map(|group| (group.filter_type, None));
                return first;
            }
        };

        // Ensure the group is valid.
        let group = match filter.filters.get(selection.0) {
            Some(g) => g,
            None => {
                // Current group doesn't exist. Attempt to select the previosu group.
                let new_group = filter.filters.prev_group(selection.0);
                match new_group {
                    None => {
                        // Previous group didn't exist either. Do first group instead.
                        // If first group doesn't exist, that's a valid reason to yield None.
                        let first = filter.filters.first();
                        self.selection = first.map(|first| (first.filter_type, None));
                        return first;
                    }
                    Some(g) => {
                        // Previous group did exist, set that as the selection.
                        self.selection = Some((g.filter_type, None));
                        return Some(g);
                    }
                }
            }
        };

        // Ensure the filter within the group is valid.
        if let Some(f) = &selection.1 {
            if group.get(&f).is_none() {
                // Filter not found. Select the previous item that exits.
                //
                // It's okay if the previous item won't exist as this just results in a group
                // selection at that point.
                self.selection = Some((group.filter_type, group.prev_filter(&f).map(String::from)));
            }
        }

        Some(group)
    }

    pub fn draw<B: Backend>(
        &mut self,
        filter: &FilterState<EncodedRequest>,
        ctx: &UiContext,
        request: Option<&EncodedRequest>,
        is_active: bool,
        f: &mut Frame<B>,
        chunk: Rect,
    )
    {
        self.ensure_selection(filter);

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
            Text::raw("[t]: Toggle selected filter or filter group\n"),
            Text::raw("[x]: Remove selected filter or filter group\n"),
            Text::raw("[X]: Remove all filters\n"),
        ]);

        let keys_paragraph = Paragraph::new(keys_text.iter());
        f.render_widget(keys_paragraph, sub_chunks[0]);

        let mut filter_items = vec![];
        let mut state = ListState::default();
        for group in filter.filters.iter() {
            if self.selection == Some((group.filter_type, None)) {
                state.select(Some(filter_items.len()));
            }

            let mut style = Style::default();
            if !group.enabled {
                style = style.fg(Color::Gray)
            }
            filter_items.push(Text::styled(
                format!("{} filters:", group.filter_type),
                style,
            ));

            for single_filter in group.iter() {
                if let Some((ft, Some(filter))) = &self.selection {
                    if *ft == group.filter_type && filter.as_str() == single_filter.key() {
                        state.select(Some(filter_items.len()));
                    }
                }
                let mut style = Style::default();
                if !single_filter.enabled {
                    style = style.fg(Color::Gray)
                }
                filter_items.push(Text::styled(
                    format!(" - {}", single_filter.to_string(ctx)),
                    style,
                ));
            }
        }

        let filter_list = List::new(filter_items.into_iter())
            .block(create_control_block("Current filters", is_active))
            .highlight_symbol("> ");
        f.render_stateful_widget(filter_list, sub_chunks[1], &mut state)
    }

    fn on_connection_filter<B: Backend>(
        &mut self,
        filter: &mut FilterState<EncodedRequest>,
        request: Option<&EncodedRequest>,
    ) -> Option<HandleResult<B>>
    {
        request.map(|req| {
            let connection = req.request_data.connection_uuid;
            let key = filter.add_filter(Box::new(ConnectionFilter { connection }));
            self.selection = Some((key.0, Some(key.1)));
            HandleResult::Update
        })
    }

    fn on_path_filter<B: Backend>(
        &mut self,
        filter: &mut FilterState<EncodedRequest>,
        request: Option<&EncodedRequest>,
    ) -> Option<HandleResult<B>>
    {
        request.map(|req| {
            let key = filter.add_filter(Box::new(PathFilter {
                path: req.request_data.uri.path().to_string(),
            }));
            self.selection = Some((key.0, Some(key.1)));
            HandleResult::Update
        })
    }
}
