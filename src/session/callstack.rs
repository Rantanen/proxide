use serde::{Deserialize, Serialize};

/// UI visualization types for callstack captures.

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Thread
{
    /// Identity of the thread.
    id: i64,

    /// Name of the thread.
    name: String,

    /// Captured stack frames of the thread.
    frames: Vec<Frame>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Frame
{
    symbols: Vec<Symbol>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Symbol
{
    name: String,
}

impl Thread
{
    pub fn id(&self) -> i64
    {
        self.id
    }

    pub fn name(&self) -> &str
    {
        &self.name
    }

    pub fn frames(&self) -> &[Frame]
    {
        &self.frames
    }
}

impl Frame
{
    pub fn symbols(&self) -> &[Symbol]
    {
        &self.symbols
    }
}

impl Symbol
{
    pub fn name(&self) -> &str
    {
        &self.name
    }
}

#[cfg(target_os = "linux")]
impl From<&rstack::Thread> for Thread
{
    fn from(value: &rstack::Thread) -> Self
    {
        Self {
            id: value.id() as i64,
            name: value.name().unwrap_or("<unknown>").to_string(),
            frames: value.frames().iter().map(Frame::from).collect(),
        }
    }
}

#[cfg(target_os = "linux")]
impl From<&rstack::Frame> for Frame
{
    fn from(value: &rstack::Frame) -> Self
    {
        Frame {
            symbols: value.symbol().iter().map(|s| Symbol::from(*s)).collect(),
        }
    }
}

#[cfg(target_os = "linux")]
impl From<&rstack::Symbol> for Symbol
{
    fn from(value: &rstack::Symbol) -> Self
    {
        Self {
            name: value.name().to_string(),
        }
    }
}

#[cfg(all(target_os = "linux", test))]
impl From<&rstack_self::Thread> for Thread
{
    fn from(value: &rstack_self::Thread) -> Self
    {
        Self {
            id: value.id() as i64,
            name: value.name().to_string(),
            frames: value.frames().iter().map(Frame::from).collect(),
        }
    }
}

#[cfg(all(target_os = "linux", test))]
impl From<&rstack_self::Frame> for Frame
{
    fn from(value: &rstack_self::Frame) -> Self
    {
        Self {
            symbols: value.symbols().iter().map(Symbol::from).collect(),
        }
    }
}
#[cfg(all(target_os = "linux", test))]
impl From<&rstack_self::Symbol> for Symbol
{
    fn from(value: &rstack_self::Symbol) -> Self
    {
        Symbol {
            name: value.name().expect("Name missing").to_string(),
        }
    }
}
