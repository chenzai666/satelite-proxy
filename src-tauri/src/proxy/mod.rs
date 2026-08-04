//! System HTTP(S) proxy control.

mod macos;
#[cfg(not(target_os = "macos"))]
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
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(stub::StubSystemProxy)
    }
}
