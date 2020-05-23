use std::borrow::Cow;
use tui::backend::Backend;
use tui::style::{Modifier, Style};
use tui::widgets::{Row, Table, TableState};

use super::prelude::*;
use crate::session::IndexedVec;

pub struct TableView<T>
{
    title: Cow<'static, str>,

    tui_state: TableState,
    user_selected: Option<usize>,

    columns: Vec<Column<T>>,
}

struct Column<T>
{
    title: &'static str,
    constraint: Constraint,
    map: fn(&T) -> String,
}

impl<T> TableView<T>
{
    pub fn new<TTitle: Into<Cow<'static, str>>>(title: TTitle) -> Self
    {
        Self {
            title: title.into(),
            tui_state: Default::default(),
            user_selected: Default::default(),
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

    pub fn on_input<B: Backend>(
        &mut self,
        content: &IndexedVec<T>,
        e: CTEvent,
        _size: Rect,
    ) -> HandleResult<B>
    {
        match e {
            CTEvent::Key(key) => match key.code {
                KeyCode::Char('k') | KeyCode::Up => self.user_select(
                    content,
                    self.user_selected
                        .or_else(|| self.tui_state.selected())
                        .map(|i| i.saturating_sub(1)),
                ),
                KeyCode::Char('j') | KeyCode::Down => self.user_select(
                    content,
                    self.user_selected
                        .or_else(|| self.tui_state.selected())
                        .map(|i| i + 1),
                ),
                KeyCode::Esc => self.user_select(content, None),
                _ => return HandleResult::Ignore,
            },
            _ => return HandleResult::Ignore,
        };
        HandleResult::Update
    }

    pub fn user_select(&mut self, content: &IndexedVec<T>, idx: Option<usize>)
    {
        match idx {
            None => {
                self.user_selected = None;
                if content.items.is_empty() {
                    self.tui_state.select(None);
                } else {
                    self.tui_state.select(Some(content.items.len() - 1));
                }
            }
            Some(mut idx) => {
                if idx >= content.items.len() {
                    idx = content.items.len() - 1;
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
            Some(idx) if idx >= content.items.len() => Some(content.items.len() - 1),
            None if content.items.is_empty() => None,
            None => Some(content.items.len() - 1),
            some => some,
        };

        self.tui_state.select(selection);
    }

    pub fn selected<'a>(&self, content: &'a IndexedVec<T>) -> Option<&'a T>
    {
        self.tui_state
            .selected()
            .and_then(|idx| content.items.get(idx))
    }

    pub fn draw_requests<B: Backend>(
        &mut self,
        content: &[T],
        f: &mut Frame<B>,
        chunk: Rect,
        is_active: bool,
    )
    {
        let block = create_block(&self.title, is_active);

        // Get a borrow of columns to avoid having to use `self` within the closure below.
        let columns = &self.columns;

        let widths = columns.iter().map(|c| c.constraint).collect::<Vec<_>>();
        let table = Table::new(
            columns.iter().map(|c| c.title),
            content
                .iter()
                .map(|item| Row::Data(columns.iter().map(move |c| (c.map)(item)))),
        )
        .block(block)
        .widths(&widths)
        .highlight_symbol("> ")
        .highlight_style(Style::default().modifier(Modifier::BOLD));

        f.render_stateful_widget(table, chunk, &mut self.tui_state)
    }
}
