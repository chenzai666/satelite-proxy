//! System HTTP(S) proxy control.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod stub;
#[cfg(target_os = "windows")]
mod windows;

use crate::error::AppResult;

#[derive(Debug, Clone)]
pub struct SystemProxySnapshot {
    /// Platform-specific opaque restore token (e.g. service name + previous flags).
    ///
    /// Part of the cross-platform snapshot contract; written by some backends
    /// and read by others, so on a given platform it may look unused.
    #[allow(dead_code)]
    pub detail: String,
}

pub trait SystemProxy: Send + Sync {
    fn enable(&self, host: &str, port: u16) -> AppResult<SystemProxySnapshot>;
    fn disable(&self, snapshot: Option<&SystemProxySnapshot>) -> AppResult<()>;
}

pub fn create_system_proxy() -> Box<dyn SystemProxy> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacSystemProxy::default())
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsSystemProxy)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Box::new(stub::StubSystemProxy)
    }
}
