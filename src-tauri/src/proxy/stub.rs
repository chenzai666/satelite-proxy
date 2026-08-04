use super::{SystemProxy, SystemProxySnapshot};
use crate::error::{AppError, AppResult};

pub struct StubSystemProxy;

impl SystemProxy for StubSystemProxy {
    fn enable(&self, _host: &str, _port: u16) -> AppResult<SystemProxySnapshot> {
        Err(AppError::Core(
            "system proxy not implemented on this platform yet".into(),
        ))
    }

    fn disable(&self, _snapshot: Option<&SystemProxySnapshot>) -> AppResult<()> {
        Ok(())
    }
}
