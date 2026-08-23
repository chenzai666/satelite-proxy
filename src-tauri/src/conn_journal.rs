//! Connection journal: stream Clash API snapshots and maintain history.
//!
//! Interval adapts: faster when UI is visible, slower when tray-only.

use crate::api::{parse_connections_json, ClashApi};
use crate::state::AppState;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::Message;
use tungstenite::{client as ws_client, Error as WsError};

/// Active UI: catch short-lived conns.
const WS_INTERVAL_ACTIVE_MS: u64 = 100;
/// UI hidden / tray-only. Slow enough to keep history alive without churning
/// the Rust heap while nobody is looking (docs/webview2-memory-optimization-plan.md).
const WS_INTERVAL_BACKGROUND_MS: u64 = 1000;
/// HTTP poll when WS is unavailable.
const FALLBACK_HTTP_MS: u64 = 350;
const IDLE_MS: u64 = 500;
const RECONNECT_MS: u64 = 500;

pub fn spawn_connection_journal(app: AppHandle) {
    if let Err(error) = thread::Builder::new()
        .name("conn-journal".into())
        .spawn(move || journal_loop(app))
    {
        crate::app_log::error(
            "journal",
            format!("failed to start connection journal: {error}"),
        );
    }
}

fn journal_interval_ms(state: &AppState) -> u64 {
    interval_for_visibility(state.is_ui_visible())
}

fn interval_for_visibility(visible: bool) -> u64 {
    if visible {
        WS_INTERVAL_ACTIVE_MS
    } else {
        WS_INTERVAL_BACKGROUND_MS
    }
}

fn journal_loop(app: AppHandle) {
    loop {
        let Some(state) = app.try_state::<AppState>() else {
            thread::sleep(Duration::from_millis(IDLE_MS));
            continue;
        };

        if state.is_core_transitioning() {
            thread::sleep(Duration::from_millis(IDLE_MS));
            continue;
        }

        let api = state.try_clash_api_clone();

        let Some(api) = api else {
            // Xray mode: no Clash API — poll the metrics module for traffic
            // totals instead. Per-connection data does not exist.
            if let Some(metrics) = state.try_xray_metrics_clone() {
                poll_xray_metrics(&state, &metrics);
            } else {
                thread::sleep(Duration::from_millis(IDLE_MS));
            }
            continue;
        };

        let interval = journal_interval_ms(&state);

        match stream_ws_snapshots(
            &api,
            interval,
            || journal_interval_ms(&state) != interval,
            |text| {
                if state.is_core_transitioning() {
                    return;
                }
                match parse_connections_json(text) {
                    Ok(snap) => {
                        state.try_apply_connection_snapshot(&api, snap);
                    }
                    Err(e) => {
                        crate::app_log::debug("journal", format!("parse: {e}"));
                    }
                }
            },
        ) {
            Ok(()) => {}
            Err(e) => {
                crate::app_log::debug("journal", format!("WS: {e}; fallback HTTP"));
                for _ in 0..10 {
                    if state.is_core_transitioning() {
                        break;
                    }
                    let still = state.try_clash_api_clone();
                    let Some(api) = still else { break };
                    match api.list_connections() {
                        Ok(snap) => {
                            state.try_apply_connection_snapshot(&api, snap);
                        }
                        Err(_) => break,
                    }
                    thread::sleep(Duration::from_millis(FALLBACK_HTTP_MS));
                }
                thread::sleep(Duration::from_millis(RECONNECT_MS));
            }
        }
    }
}

/// Xray metrics polling loop: sample `/debug/vars` traffic totals until the
/// session goes away or a core transition starts. 1s cadence — the counters
/// are cumulative, so faster polling buys nothing.
fn poll_xray_metrics(state: &AppState, metrics: &crate::api::XrayMetrics) {
    const XRAY_POLL_MS: u64 = 1000;
    while metrics.is_active() && !state.is_core_transitioning() {
        match metrics.traffic_totals() {
            Some(totals) => {
                state.try_apply_metrics_snapshot(metrics, totals);
            }
            None => break,
        }
        for _ in 0..(XRAY_POLL_MS / 50) {
            if !metrics.is_active() || state.is_core_transitioning() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

fn stream_ws_snapshots(
    api: &ClashApi,
    interval_ms: u64,
    mut interval_changed: impl FnMut() -> bool,
    mut on_text: impl FnMut(&str),
) -> Result<(), String> {
    let url = api.connections_ws_url(interval_ms);
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("ws request: {e}"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", api.secret)
            .parse()
            .map_err(|e| format!("auth header: {e}"))?,
    );

    let host_port = api
        .base
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("127.0.0.1:19090");

    let stream = TcpStream::connect(host_port).map_err(|e| format!("tcp {host_port}: {e}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = stream.set_nodelay(true);

    let (mut socket, _resp) =
        ws_client(request, stream).map_err(|e| format!("ws handshake: {e}"))?;

    loop {
        if !api.is_active() {
            let _ = socket.close(None);
            return Ok(());
        }
        if interval_changed() {
            let _ = socket.close(None);
            return Ok(());
        }
        match socket.read() {
            Ok(Message::Text(text)) => on_text(text.as_str()),
            Ok(Message::Binary(bin)) => {
                if let Ok(text) = std::str::from_utf8(&bin) {
                    on_text(text);
                }
            }
            Ok(Message::Ping(p)) => {
                let _ = socket.send(Message::Pong(p));
            }
            Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            Ok(Message::Close(_)) => return Ok(()),
            Err(WsError::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => return Ok(()),
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_selects_the_expected_sampling_interval() {
        assert_eq!(interval_for_visibility(true), WS_INTERVAL_ACTIVE_MS);
        assert_eq!(interval_for_visibility(false), WS_INTERVAL_BACKGROUND_MS);
        assert!(WS_INTERVAL_BACKGROUND_MS > WS_INTERVAL_ACTIVE_MS);
    }
}
