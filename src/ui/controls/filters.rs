use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use uuid::Uuid;

use crate::search::SearchIndex;
use crate::session::EncodedRequest;
use crate::ui::state::UiContext;

pub struct FilterState<T>
{
    last_count: usize,

    pub use_filter: bool,
    filtered_items: Vec<usize>,
    filtered_items_set: HashSet<usize>,
    pub filters: BTreeMap<FilterType, BTreeMap<String, Box<dyn ItemFilter<T>>>>,
    // These are waiting for highlight/search implementation.
    /*
    highlighted_items: HashSet<usize>,
    pub highlights: BTreeMap<FilterType, BTreeMap<String, Box<dyn ItemFilter<T>>>>,

    search_results: HashSet<usize>,
    pub search_pattern: String,
    */
}

impl<T> FilterState<T>
{
    pub fn len_filtered(&mut self, items: &[T]) -> usize
    {
        if self.use_filter {
            self.filter_new(items);
            self.filtered_items.len()
        } else {
            items.len()
        }
    }

    pub fn is_empty_filtered(&mut self, items: &[T]) -> bool
    {
        if self.use_filter {
            self.filter_new(items);
            self.filtered_items.is_empty()
        } else {
            items.is_empty()
        }
    }

    pub fn get<'a>(&mut self, idx: usize, items: &'a [T]) -> Option<(&'a T, bool)>
    {
        if self.use_filter {
            self.filter_new(items);
            self.filtered_items
                .get(idx)
                .and_then(|idx| items.get(*idx))
                .map(|i| (i, true))
        } else {
            items
                .get(idx)
                .map(|item| (item, self.filtered_items_set.contains(&idx)))
        }
    }

    pub fn iter<'a>(&'a mut self, items: &'a [T]) -> Box<dyn Iterator<Item = (&'a T, bool)> + 'a>
    {
        self.filter_new(items);
        if self.use_filter {
            Box::new(
                self.filtered_items
                    .iter()
                    .map(move |idx| (&items[*idx], true)),
            )
        } else {
            Box::new(
                items
                    .iter()
                    .enumerate()
                    .map(move |(idx, item)| (item, self.filtered_items_set.contains(&idx))),
            )
        }
    }

    pub fn find_filtered_index(&mut self, idx: usize, items: &[T]) -> usize
    {
        self.filter_new(items);

        if self.use_filter {
            match self.filtered_items.binary_search(&idx) {
                Ok(idx) | Err(idx) => idx.min(self.filtered_items.len() - 1),
            }
        } else {
            idx
        }
    }

    pub fn add_filter(&mut self, filter: Box<dyn ItemFilter<T>>)
    {
        self.filters
            .entry(filter.filter_type())
            .or_insert_with(Default::default)
            .insert(filter.get_key().to_string(), filter);
        self.refilter();
    }

    pub fn remove_filter(&mut self, filter_type: FilterType, key: String)
    {
        if let Some(list) = self.filters.get_mut(&filter_type) {
            list.remove(&key);
        }
        self.refilter();
    }

    pub fn clear_filters(&mut self)
    {
        self.filters.clear();
        self.refilter();
    }

    pub fn toggle_filter(&mut self)
    {
        self.use_filter = !self.use_filter;
    }

    fn refilter(&mut self)
    {
        self.filtered_items.clear();
        self.filtered_items_set.clear();
        self.last_count = 0;
    }

    fn filter_new(&mut self, items: &[T])
    {
        for (i, _) in items.iter().enumerate().skip(self.last_count) {
            let item = &items[i];
            if self
                .filters
                .iter()
                .all(|(_, f)| f.iter().any(|(_, f)| f.filter(item)))
            {
                self.filtered_items.push(i);
                self.filtered_items_set.insert(i);
            }
        }
        self.last_count = items.len();
    }
}

impl<T> Default for FilterState<T>
{
    fn default() -> Self
    {
        Self {
            last_count: 0,
            use_filter: true,
            filtered_items: Default::default(),
            filtered_items_set: Default::default(),
            filters: Default::default(),
            /*
            highlighted_items: Default::default(),
            highlights: Default::default(),
            search_results: Default::default(),
            search_pattern: Default::default(),
            */
        }
    }
}

pub trait ItemFilter<T>
{
    fn filter_type(&self) -> FilterType;
    fn get_key(&self) -> Cow<str>;
    fn filter(&self, item: &T) -> bool;
    fn to_string(&self, ctx: &UiContext) -> String;
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub enum FilterType
{
    Connection,
    Path,
    Search,
}

impl FilterType
{
    pub fn as_str(&self) -> &'static str
    {
        match self {
            FilterType::Connection => "Connection",
            FilterType::Path => "Path",
            FilterType::Search => "Text",
        }
    }
}

impl std::fmt::Display for FilterType
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result
    {
        write!(f, "{}", self.as_str())
    }
}

pub struct SearchFilter
{
    pub pattern: String,
    pub index: Rc<RefCell<SearchIndex>>,
}

impl ItemFilter<EncodedRequest> for SearchFilter
{
    fn filter_type(&self) -> FilterType
    {
        FilterType::Search
    }

    fn get_key(&self) -> Cow<str>
    {
        Cow::from(&self.pattern)
    }

    fn filter(&self, item: &EncodedRequest) -> bool
    {
        self.index
            .borrow()
            .is_match(item.request_data.uuid, &self.pattern)
    }

    fn to_string(&self, _ctx: &UiContext) -> String
    {
        self.pattern.clone()
    }
}

pub struct ConnectionFilter
{
    pub connection: Uuid,
}

impl ItemFilter<EncodedRequest> for ConnectionFilter
{
    fn filter_type(&self) -> FilterType
    {
        FilterType::Connection
    }

    fn get_key(&self) -> Cow<str>
    {
        Cow::from(self.connection.to_string())
    }

    fn filter(&self, item: &EncodedRequest) -> bool
    {
        item.request_data.connection_uuid == self.connection
    }

    fn to_string(&self, ctx: &UiContext) -> String
    {
        match ctx.data.connections.get_by_uuid(self.connection) {
            None => format!("Unknown connection ({:?})", self.connection),
            Some(conn) => format!("{}", conn.client_addr),
        }
    }
}

pub struct PathFilter
{
    pub path: String,
}

impl ItemFilter<EncodedRequest> for PathFilter
{
    fn filter_type(&self) -> FilterType
    {
        FilterType::Path
    }

    fn get_key(&self) -> Cow<str>
    {
        Cow::from(&self.path)
    }

    fn filter(&self, item: &EncodedRequest) -> bool
    {
        item.request_data.uri.path() == self.path
    }

    fn to_string(&self, _ctx: &UiContext) -> String
    {
        self.path.clone()
    }
}
