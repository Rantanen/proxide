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
    pub filters: FilterMap<T>,
    // These are waiting for highlight/search implementation.
    /*
    highlighted_items: HashSet<usize>,
    pub highlights: BTreeMap<FilterType, BTreeMap<String, Box<dyn ItemFilter<T>>>>,

    search_results: HashSet<usize>,
    pub search_pattern: String,
    */
}

pub struct FilterMap<T>
{
    map: BTreeMap<FilterType, FilterGroupState<T>>,
}

pub struct FilterGroupState<T>
{
    pub filter_type: FilterType,
    pub enabled: bool,
    filters: BTreeMap<String, SingleFilterState<T>>,
}

pub struct SingleFilterState<T>
{
    pub enabled: bool,
    filter: Box<dyn ItemFilter<T>>,
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

    pub fn iter<'a>(
        &'a mut self,
        items: &'a [T],
        selected_filter: Option<(FilterType, &'a str)>,
    ) -> impl Iterator<Item = (&'a T, bool, bool)> + 'a
    {
        self.filter_new(items);
        self.iter_no_new(items, selected_filter)
    }

    fn iter_no_new<'a>(
        &'a self,
        items: &'a [T],
        selected_filter: Option<(FilterType, &'a str)>,
    ) -> impl Iterator<Item = (&'a T, bool, bool)> + 'a
    {
        let items_iter: Box<dyn Iterator<Item = _>> = if self.use_filter {
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
        };

        items_iter.map(move |(item, filtered)| {
            let sf = match selected_filter {
                None => return (item, filtered, false),
                Some(sf) => sf,
            };

            let highlight = self
                .filters
                .get(sf.0)
                .and_then(|group| group.get(sf.1))
                .map(|f| f.filter(&item))
                .unwrap_or(false);
            (item, filtered, highlight)
        })
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

    pub fn has_filter(&self, filter: &dyn ItemFilter<T>) -> bool
    {
        self.filters
            .get(filter.filter_type())
            .and_then(|g| g.get(filter.key().as_ref()))
            .map(|_| true)
            .unwrap_or(false)
    }

    pub fn add_filter(&mut self, filter: Box<dyn ItemFilter<T>>) -> (FilterType, String)
    {
        let pair = (filter.filter_type(), filter.key().to_string());
        self.filters
            .add(filter.filter_type(), filter.key().to_string(), filter);
        self.refilter();
        pair
    }

    pub fn remove_filter(&mut self, filter_type: FilterType, key: &str)
    {
        self.filters.remove_filter(filter_type, key);
        self.refilter();
    }

    pub fn remove_filter_group(&mut self, filter_type: FilterType)
    {
        self.filters.remove_group(filter_type);
        self.refilter();
    }

    pub fn toggle_filter(&mut self, filter_type: FilterType, key: &str)
    {
        self.filters.toggle_filter(filter_type, key);
        self.refilter();
    }

    pub fn toggle_filter_group(&mut self, filter_type: FilterType)
    {
        self.filters.toggle_group(filter_type);
        self.refilter();
    }

    pub fn clear_filters(&mut self)
    {
        self.filters.clear();
        self.refilter();
    }

    pub fn toggle(&mut self)
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
            let matches_filter = self.filters.iter().all(|group| group.filter(item));
            if matches_filter {
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

impl<V> Default for FilterMap<V>
{
    fn default() -> Self
    {
        Self {
            map: BTreeMap::default(),
        }
    }
}

impl<T> FilterGroupState<T>
{
    fn new(filter_type: FilterType) -> Self
    {
        Self {
            filter_type,
            enabled: true,
            filters: BTreeMap::default(),
        }
    }

    fn filter(&self, item: &T) -> bool
    {
        match self.enabled {
            false => true,
            true => self
                .filters
                .values()
                .any(|f| f.enabled && f.filter.filter(item)),
        }
    }
}

pub trait ItemFilter<T>
{
    fn filter_type(&self) -> FilterType;
    fn key(&self) -> Cow<str>;
    fn filter(&self, item: &T) -> bool;
    fn to_string(&self, ctx: &UiContext) -> String;
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Debug)]
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

    fn key(&self) -> Cow<str>
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

    fn key(&self) -> Cow<str>
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

    fn key(&self) -> Cow<str>
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

impl<T> FilterMap<T>
{
    pub fn first(&self) -> Option<&FilterGroupState<T>>
    {
        self.map.values().next()
    }

    pub fn add(&mut self, ft: FilterType, key: String, filter: Box<dyn ItemFilter<T>>)
    {
        self.map
            .entry(ft)
            .or_insert_with(|| FilterGroupState::new(ft))
            .filters
            .insert(
                key,
                SingleFilterState {
                    enabled: true,
                    filter,
                },
            );
    }

    pub fn clear(&mut self)
    {
        self.map.clear();
    }

    pub fn get(&self, ft: FilterType) -> Option<&FilterGroupState<T>>
    {
        self.map.get(&ft)
    }

    pub fn prev_group(&self, current: FilterType) -> Option<&FilterGroupState<T>>
    {
        for cursor in self.map.values().rev() {
            if current > cursor.filter_type {
                return Some(cursor);
            }
        }
        None
    }

    pub fn next_group(&self, current: FilterType) -> Option<&FilterGroupState<T>>
    {
        for cursor in self.map.values() {
            if current < cursor.filter_type {
                return Some(cursor);
            }
        }
        None
    }

    pub fn iter(&self) -> impl Iterator<Item = &FilterGroupState<T>>
    {
        self.map.values()
    }

    pub fn remove_filter(&mut self, ft: FilterType, key: &str)
    {
        let group = match self.map.get_mut(&ft) {
            None => return,
            Some(g) => g,
        };

        group.filters.remove(key);
        if group.filters.is_empty() {
            self.map.remove(&ft);
        }
    }

    pub fn remove_group(&mut self, ft: FilterType)
    {
        self.map.remove(&ft);
    }

    pub fn toggle_filter(&mut self, ft: FilterType, key: &str)
    {
        let group = match self.map.get_mut(&ft) {
            None => return,
            Some(g) => g,
        };

        let filter = match group.filters.get_mut(key) {
            None => return,
            Some(f) => f,
        };

        match filter.enabled {
            false => {
                filter.enabled = true;
                group.enabled = true;
            }
            true => {
                filter.enabled = false;
                group.enabled = group.filters.values().any(|f| f.enabled)
            }
        }
    }

    pub fn toggle_group(&mut self, ft: FilterType)
    {
        let group = match self.map.get_mut(&ft) {
            None => return,
            Some(g) => g,
        };

        group.enabled = !group.enabled;
        for v in group.filters.values_mut() {
            v.enabled = group.enabled;
        }
    }
}

impl<T> FilterGroupState<T>
{
    pub fn prev_filter(&self, current: &str) -> Option<&str>
    {
        for cursor in self.filters.keys().rev() {
            if current > cursor.as_str() {
                return Some(cursor.as_str());
            }
        }
        None
    }

    pub fn next_filter(&self, current: &str) -> Option<&str>
    {
        for cursor in self.filters.keys() {
            if current < cursor.as_str() {
                return Some(cursor.as_str());
            }
        }
        None
    }

    pub fn first_key(&self) -> Option<&str>
    {
        self.filters.keys().next().map(|s| s.as_str())
    }

    pub fn last_key(&self) -> Option<&str>
    {
        self.filters.keys().rev().next().map(|s| s.as_str())
    }

    pub fn get(&self, f: &str) -> Option<&SingleFilterState<T>>
    {
        self.filters.get(f)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SingleFilterState<T>>
    {
        self.filters.values()
    }
}

impl<T> SingleFilterState<T>
{
    pub fn to_string(&self, ctx: &UiContext) -> String
    {
        self.filter.to_string(ctx)
    }

    pub fn key(&self) -> Cow<str>
    {
        self.filter.key()
    }

    pub fn filter(&self, t: &T) -> bool
    {
        self.filter.filter(t)
    }
}
