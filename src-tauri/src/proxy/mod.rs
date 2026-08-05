//! System HTTP(S) proxy control.

mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod stub;

use crate::error::AppResult;

#[derive(Debug, Clone)]
pub struct SystemProxySnapshot {
    /// Platform-specific opaque restore token (e.g. service name + previous flags).
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
