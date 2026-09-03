//! Mihomo 内核日志监听：把 `/logs` WebSocket 中的出站拨号失败纳入
//! 智能切换的被动健康统计。
//!
//! Mihomo 只有在出站拨号成功后才创建连接追踪器，因此拨号超时不会出现
//! 在 `/connections` 中；日志流是识别这类失败的唯一被动信号。sing-box
//! 不启用此监听，因为它的连接快照已经能看见拨号失败记录。

use crate::api::ClashApi;
use crate::state::AppState;
use serde::Deserialize;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::Message;
use tungstenite::{client as ws_client, Error as WsError};

const IDLE_MS: u64 = 1_000;
const RECONNECT_MS: u64 = 2_000;
const READ_TIMEOUT_MS: u64 = 500;
const MAIN_GROUP: &str = "proxy";
const NODE_TAG_PREFIX: &str = "node-";

pub fn spawn_log_listener(app: AppHandle) {
    if let Err(error) = thread::Builder::new()
        .name("log-listener".into())
        .spawn(move || log_listener_loop(app))
    {
        crate::app_log::error(
            "log_listener",
            format!("failed to start log listener: {error}"),
        );
    }
}

fn log_listener_loop(app: AppHandle) {
    loop {
        let Some(state) = app.try_state::<AppState>() else {
            thread::sleep(Duration::from_millis(IDLE_MS));
            continue;
        };
        if state.is_core_transitioning() {
            thread::sleep(Duration::from_millis(IDLE_MS));
            continue;
        }

        let (api, is_mihomo) = {
            let runtime = state.lock_runtime();
            (
                runtime.clash_api_clone(),
                runtime.core.kind() == crate::core::CoreKind::Mihomo,
            )
        };
        let Some(api) = api else {
            thread::sleep(Duration::from_millis(IDLE_MS));
            continue;
        };
        if !is_mihomo || !state.is_core_running() {
            thread::sleep(Duration::from_millis(IDLE_MS));
            continue;
        }

        if let Err(error) = stream_kernel_logs(&state, &api) {
            crate::app_log::debug("log_listener", format!("log WS: {error}"));
            thread::sleep(Duration::from_millis(RECONNECT_MS));
        }
    }
}

fn stream_kernel_logs(state: &AppState, api: &ClashApi) -> Result<(), String> {
    let url = api.logs_ws_url("warning");
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| format!("ws request: {error}"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", api.secret)
            .parse()
            .map_err(|error| format!("auth header: {error}"))?,
    );

    let host_port = api
        .base
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("127.0.0.1:19090")
        .to_string();
    let stream =
        TcpStream::connect(&host_port).map_err(|error| format!("tcp {host_port}: {error}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)));
    let _ = stream.set_nodelay(true);
    let (mut socket, _response) =
        ws_client(request, stream).map_err(|error| format!("ws handshake: {error}"))?;

    loop {
        if !api.is_active() || state.is_core_transitioning() {
            let _ = socket.close(None);
            return Ok(());
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                if let Some(line) = parse_dial_failure_frame(text.as_str()) {
                    record_failure(state, line);
                }
            }
            Ok(Message::Binary(bytes)) => {
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    if let Some(line) = parse_dial_failure_frame(text) {
                        record_failure(state, line);
                    }
                }
            }
            Ok(Message::Ping(payload)) => {
                let _ = socket.send(Message::Pong(payload));
            }
            Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            Ok(Message::Close(_)) => return Ok(()),
            Err(WsError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        }
    }
}

#[derive(Debug, PartialEq)]
struct DialFailureLine {
    proxy: String,
    dest: String,
}

#[derive(Deserialize)]
struct LogFrame {
    #[serde(rename = "type")]
    level: String,
    payload: String,
}

fn parse_dial_failure_frame(text: &str) -> Option<DialFailureLine> {
    let frame: LogFrame = serde_json::from_str(text).ok()?;
    let level = frame.level.to_ascii_lowercase();
    if level != "warning" && level != "error" {
        return None;
    }
    parse_dial_failure(&frame.payload)
}

/// 解析 Mihomo 的 `[TCP]/[UDP] dial ... --> ... error:` 日志。
fn parse_dial_failure(payload: &str) -> Option<DialFailureLine> {
    let rest = payload
        .strip_prefix("[TCP] dial ")
        .or_else(|| payload.strip_prefix("[UDP] dial "))?;
    let error_index = rest.find(" error: ")?;
    let head = &rest[..error_index];
    let arrow = head.rfind(" --> ")?;
    let dest = dest_host_key(head[arrow + 5..].trim());
    let name_part = &head[..arrow];
    let proxy = match name_part.find(" (match ") {
        Some(index) => name_part[..index].trim(),
        None => name_part
            .split(' ')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())?,
    };
    if proxy.is_empty() || dest.is_empty() {
        return None;
    }
    Some(DialFailureLine {
        proxy: proxy.to_string(),
        dest,
    })
}

fn dest_host_key(dest: &str) -> String {
    if let Some((host, port)) = dest.rsplit_once(':') {
        if !port.is_empty()
            && port.chars().all(|value| value.is_ascii_digit())
            && !host.is_empty()
            && !host.ends_with(':')
        {
            return host
                .trim_matches(|value| value == '[' || value == ']')
                .to_string();
        }
    }
    dest.to_string()
}

fn record_failure(state: &AppState, line: DialFailureLine) {
    let tag = if line.proxy.starts_with(NODE_TAG_PREFIX) {
        line.proxy
    } else if line.proxy == MAIN_GROUP {
        let current = state.with_store(|store| {
            Ok(store
                .settings
                .current_node_id
                .as_ref()
                .and_then(|id| store.find_node(id))
                .map(crate::config::outbound_tag))
        });
        match current.unwrap_or(None) {
            Some(tag) => tag,
            None => return,
        }
    } else {
        return;
    };
    state
        .lock_runtime()
        .record_proxy_dial_failure(&tag, &line.dest);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rule_matched_dial_failure() {
        let line = parse_dial_failure(
            "[TCP] dial proxy (match RuleSet/proxy) 192.168.1.2:52341 --> github.com:443 error: dial tcp 13.250.231.171:39645: i/o timeout",
        )
        .expect("should parse");
        assert_eq!(line.proxy, "proxy");
        assert_eq!(line.dest, "github.com");
    }

    #[test]
    fn parses_retry_line_and_pinned_node() {
        let line = parse_dial_failure(
            "[TCP] dial node-2db123c73eacab2b (match GeoSite/cn) 192.168.1.2:1 --> ipwho.is:443 error: connect: connection refused, retry 2",
        )
        .expect("should parse");
        assert_eq!(line.proxy, "node-2db123c73eacab2b");
        assert_eq!(line.dest, "ipwho.is");
    }

    #[test]
    fn parses_rule_nil_form_and_ipv6() {
        let line = parse_dial_failure(
            "[UDP] dial GLOBAL 192.168.1.2:1 --> 8.8.8.8:53 error: dial udp: i/o timeout",
        )
        .expect("should parse");
        assert_eq!(line.proxy, "GLOBAL");
        assert_eq!(line.dest, "8.8.8.8");

        let ipv6 = parse_dial_failure(
            "[TCP] dial proxy (match Match) ::1 --> [2001:db8::1]:443 error: i/o timeout",
        )
        .expect("should parse IPv6");
        assert_eq!(ipv6.dest, "2001:db8::1");
    }

    #[test]
    fn frame_level_filter_ignores_info_and_non_json() {
        assert!(parse_dial_failure_frame(
            r#"{"type":"warning","payload":"[TCP] dial proxy (match Match) a --> b.com:443 error: x"}"#,
        )
        .is_some());
        assert!(parse_dial_failure_frame(
            r#"{"type":"info","payload":"[TCP] dial proxy (match Match) a --> b.com:443 error: x"}"#,
        )
        .is_none());
        assert!(parse_dial_failure_frame("not json").is_none());
    }

    #[test]
    fn destination_host_key_strips_only_numeric_ports() {
        assert_eq!(dest_host_key("github.com:443"), "github.com");
        assert_eq!(dest_host_key("[2001:db8::1]:443"), "2001:db8::1");
        assert_eq!(dest_host_key("2001:db8::1"), "2001:db8::1");
        assert_eq!(dest_host_key("bare-host"), "bare-host");
    }
}
