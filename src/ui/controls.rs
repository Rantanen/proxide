use crossterm::event::KeyModifiers;
use std::borrow::Cow;
use tui::backend::Backend;
use tui::style::{Modifier, Style};
use tui::widgets::{Row, Table, TableState};
use uuid::Uuid;

use super::prelude::*;
use crate::session::IndexedVec;

pub struct TableView<T>
{
    title: Cow<'static, str>,

    tui_state: TableState,
    user_selected: Option<usize>,
    locked: Option<Uuid>,

    group_filter: fn(&T, &T) -> bool,

    columns: Vec<Column<T>>,
}

struct Column<T>
{
    title: &'static str,
    constraint: Constraint,
    map: fn(&T) -> String,
}

enum Dir
{
    Previous,
    Next,
}
impl<T: crate::session::HasKey> TableView<T>
{
    pub fn new<TTitle: Into<Cow<'static, str>>>(title: TTitle) -> Self
    {
        Self {
            title: title.into(),
            tui_state: Default::default(),
            user_selected: Default::default(),
            locked: None,
            group_filter: |_, _| true,
            columns: Default::default(),
        }
    }

    pub fn with_column(
        mut self,
        title: &'static str,
        constraint: Constraint,
        map: fn(&T) -> String,
    ) -> Self
    {
        self.columns.push(Column {
            title,
            constraint,
            map,
        });
        self
    }

    pub fn with_group_filter(mut self, group_filter: fn(&T, &T) -> bool) -> Self
    {
        self.group_filter = group_filter;
        self
    }

    pub fn on_input<B: Backend>(
        &mut self,
        content: &IndexedVec<T>,
        e: CTEvent,
        _size: Rect,
    ) -> HandleResult<B>
    {
        match e {
            CTEvent::Key(key) => match key.code {
                KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up => {
                    self.user_move(content, key.modifiers == KeyModifiers::SHIFT, Dir::Previous)
                }
                KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down => {
                    self.user_move(content, key.modifiers == KeyModifiers::SHIFT, Dir::Next)
                }
                KeyCode::Esc => self.user_select(content, None),
                _ => return HandleResult::Ignore,
            },
            _ => return HandleResult::Ignore,
        };
        HandleResult::Update
    }

    fn user_move(&mut self, content: &IndexedVec<T>, by_group: bool, dir: Dir)
    {
        // If there's no content, there should be no reason to move.
        // We'd just end up panicing on the calculations.
        if content.is_empty_filtered() {
            return;
        }

        // Get the current selection.
        let mut idx = match self.tui_state.selected() {
            None => return self.user_select(content, Some(content.len_filtered() - 1)),
            Some(idx) => idx.min(content.len_filtered() - 1),
        };
        let current_item = &content[idx];

        // Loop until we'll find an item that matches the filter.
        loop {
            idx = match dir {
                Dir::Previous => match idx {
                    0 => return,
                    other => other.saturating_sub(1),
                },
                Dir::Next => match idx + 1 {
                    c if c >= content.len_filtered() => {
                        return;
                    }
                    c => c,
                },
            };

            let candidate_item = &content[idx];
            if !by_group || (self.group_filter)(current_item, candidate_item) {
                return self.user_select(content, Some(idx));
            }
        }
    }

    pub fn user_select(&mut self, content: &IndexedVec<T>, idx: Option<usize>)
    {
        self.unlock();
        match idx {
            None => {
                self.user_selected = None;
                if content.is_empty_filtered() {
                    self.tui_state.select(None);
                } else {
                    self.tui_state.select(Some(content.len_filtered() - 1));
                }
            }
            Some(mut idx) => {
                if idx >= content.len_filtered() {
                    idx = content.len_filtered() - 1;
                }
                self.user_selected = Some(idx);
                self.tui_state.select(self.user_selected);
            }
        }
    }

    pub fn auto_select(&mut self, content: &IndexedVec<T>, idx: Option<usize>)
    {
        // If the user has selected something, skip the auto select. The user select will override
        // this.
        if self.user_selected.is_some() {
            return;
        }

        let selection = match idx {
            Some(idx) if idx >= content.len_filtered() => Some(content.len_filtered() - 1),
            None if content.is_empty_filtered() => None,
            None => Some(content.len_filtered() - 1),
            some => some,
        };

        self.tui_state.select(selection);
    }

    pub fn selected<'a>(&self, content: &'a IndexedVec<T>) -> Option<&'a T>
    {
        self.tui_state.selected().and_then(|idx| content.get(idx))
    }

    fn ensure_current_selection<'a>(&mut self, content: &'a IndexedVec<T>) -> Option<&'a T>
    {
        let currently_selected = self.selected(content);
        let lock = match self.locked {
            Some(l) => l,
            None => return currently_selected,
        };

        // If the current selection matches the lock there's no need to do anything.
        // This is a slight optimization to avoid having to find the item every single time.
        if let Some(selected) = currently_selected {
            if selected.key() == lock {
                return Some(selected);
            }
        }

        let idx = content.get_index_by_uuid(lock);
        self.user_selected = idx;
        self.tui_state.select(idx);
        self.selected(content)
    }

    pub fn draw_requests<B: Backend>(
        &mut self,
        content: &IndexedVec<T>,
        f: &mut Frame<B>,
        chunk: Rect,
    )
    {
        let currently_selected = self.ensure_current_selection(content);
        let block = create_block(&self.title);

        // Get a borrow of columns to avoid having to use `self` within the closure below.
        let columns = &self.columns;
        let group_filter = &self.group_filter;

        let widths = columns.iter().map(|c| c.constraint).collect::<Vec<_>>();
        let table = Table::new(
            columns.iter().map(|c| c.title),
            content.iter().map(|item| {
                // This is a bit of a mess. :(
                //
                // We'll define the closure beforehand so it's the _same_ closure for both match
                // arms. Otherwise it would be a _different_ closure (even if it did the same
                // thing), thus resulting in "different types for match arms".
                let closure = move |c: &Column<T>| (c.map)(item);

                // Match the currently selected item. If the currently selected item exists, we'll
                // want to highlight all other itms that belong to the same group. Any other items
                // is rendered normally.
                match currently_selected {
                    Some(cs) if (group_filter)(cs, item) => Row::StyledData(
                        columns.iter().map(closure),
                        Style::default().modifier(Modifier::BOLD),
                    ),
                    _ => Row::Data(columns.iter().map(closure)),
                }
            }),
        )
        .block(block)
        .widths(&widths)
        .highlight_symbol("> ")
        .highlight_style(Style::default().modifier(Modifier::BOLD));

        f.render_stateful_widget(table, chunk, &mut self.tui_state)
    }

    pub fn unlock(&mut self)
    {
        self.locked = None;
    }

    pub fn lock(&mut self, content: &IndexedVec<T>)
    {
        if let Some(item) = self.selected(content) {
            self.locked = Some(item.key())
        }
    }
}
