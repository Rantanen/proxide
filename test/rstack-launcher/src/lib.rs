use std::fmt::{Display, Formatter};
use std::process::Command;

#[cfg(target_os = "linux")]
include!(concat!(env!("OUT_DIR"), "/child.rs"));

/// An error in launching the child.
#[derive(Debug)]
pub enum Error
{
    /// The error originates from rstack_self.
    Rstack(rstack_self::Error),

    /// The specified thread was not available.
    ThreadNotFound,

    /// Unsuportted operating system.
    UnsupportedOperatingSystem,
}

/// The result type returned by methods in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Captures the callstack of a thread of the calling process.
pub fn capture_self(thread_id: i64) -> Result<rstack_self::Thread>
{
    #[cfg(target_os = "linux")]
    {
        let trace: rstack_self::Trace = launch_child()?;
        match trace
            .threads()
            .into_iter()
            .find(|&t| t.id() as i64 == thread_id)
        {
            Some(thread) => Ok(thread.clone()),
            None => Err(Error::ThreadNotFound),
        }
    }

    #[cfg(not(target_os = "linux"))]
    Err(Error::UnsupportedOperatingSystem)
}

impl From<rstack_self::Error> for Error
{
    fn from(value: rstack_self::Error) -> Self
    {
        Self::Rstack(value)
    }
}

impl Display for Error
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result
    {
        match self {
            Error::Rstack(error) => Ok(error.fmt(f)?),
            Error::ThreadNotFound => write!(f, "Specified thread unavailable."),
            Error::UnsupportedOperatingSystem => {
                write!(f, "Capture not supported on the current operatins system.")
            }
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod test
{
    use crate::capture_self;

    #[test]
    fn capturing_callstack_succeeds()
    {
        let thread_id = get_current_native_thread_id();
        let callstack = capture_self(thread_id).expect("Capturing self failed");
        assert_eq!(callstack.id() as i64, thread_id);
        assert_eq!(callstack.name(), "test::capturing");
    }

    /// Gets the current native thread id.
    fn get_current_native_thread_id() -> i64
    {
        #[cfg(not(target_os = "windows"))]
        return os_id::thread::get_raw_id() as i64;

        #[cfg(target_os = "windows")]
        unsafe {
            return windows::Win32::System::Threading::GetCurrentThreadId() as i64;
        }
    }
}
