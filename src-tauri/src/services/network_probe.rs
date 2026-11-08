//! 检测走当前内核入口；不用环境变量代理，不在失败后静默切为直连。
use crate::runtime::ProxyStatus;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeRoute {
    pub proxy_port: Option<u16>,
}

impl ProbeRoute {
    pub fn via_proxy(self) -> bool {
        self.proxy_port.is_some()
    }
}

pub fn route_for_status(status: &ProxyStatus) -> Result<ProbeRoute, String> {
    let proxy_port = if !status.running {
        None
    } else if status.runtime_source == "singbox" {
        Some(
            status
                .custom_inbound_port
                .filter(|port| *port != 0)
                .ok_or_else(|| "自定义配置没有可供检测使用的 HTTP/mixed 入站".to_string())?,
        )
    } else {
        Some(status.mixed_port)
    };
    if proxy_port == Some(0) {
        return Err("检测入口端口无效".into());
    }
    Ok(ProbeRoute { proxy_port })
}

pub fn build_client(route: ProbeRoute, timeout: Duration) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(4))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Satelite/1.0.46 NetworkDiagnostics");
    if let Some(port) = route.proxy_port {
        builder = builder.proxy(
            reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))
                .map_err(|_| "检测代理地址无效".to_string())?,
        );
    }
    builder.build().map_err(|_| "无法创建网络检测客户端".into())
}

pub fn same_session(before: &ProxyStatus, after: &ProxyStatus) -> bool {
    before.running == after.running
        && before.core_started_at == after.core_started_at
        && before.core_type == after.core_type
        && before.current_node_id == after.current_node_id
        && before.runtime_profile_id == after.runtime_profile_id
        && before.outbound_mode == after.outbound_mode
        && route_for_status(before).ok() == route_for_status(after).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_running_modes_through_core_and_stopped_directly() {
        let mut status =
            crate::runtime::Runtime::new().status(&crate::storage::AppStore::default());
        assert_eq!(route_for_status(&status).unwrap().proxy_port, None);
        status.running = true;
        status.mixed_port = 2080;
        status.outbound_mode = "direct".into();
        assert_eq!(route_for_status(&status).unwrap().proxy_port, Some(2080));
        status.runtime_source = "singbox".into();
        assert!(route_for_status(&status).is_err());
        status.custom_inbound_port = Some(8080);
        assert_eq!(route_for_status(&status).unwrap().proxy_port, Some(8080));
    }
    #[test]
    fn rejects_stale_selection_or_core_session() {
        let before = crate::runtime::Runtime::new().status(&crate::storage::AppStore::default());
        let mut after = before.clone();
        assert!(same_session(&before, &after));
        after.current_node_id = Some("another".into());
        assert!(!same_session(&before, &after));
        after = before.clone();
        after.core_started_at = Some(123);
        assert!(!same_session(&before, &after));
    }
}
