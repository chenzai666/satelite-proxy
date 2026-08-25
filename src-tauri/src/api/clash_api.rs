//! Minimal Clash-compatible API client (sing-box experimental.clash_api).
//!
//! HTTP via **ureq** (no Tokio). Do not use `reqwest::blocking` here — it embeds
//! a nested runtime and panics when used from Tauri async workers / smart_switch:
//! "Cannot drop a runtime in a context where blocking is not allowed".

use crate::error::{AppError, AppResult};
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

/// Clash / sing-box sometimes emit ports as strings, sometimes as numbers.
fn deserialize_stringish<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct Stringish;
    impl<'de> Visitor<'de> for Stringish {
        type Value = String;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a string or number")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_string<E: de::Error>(self, v: String) -> Result<String, E> {
            Ok(v)
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_bool<E: de::Error>(self, v: bool) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_unit<E: de::Error>(self) -> Result<String, E> {
            Ok(String::new())
        }
        fn visit_none<E: de::Error>(self) -> Result<String, E> {
            Ok(String::new())
        }
        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<String, D2::Error> {
            deserialize_stringish(d)
        }
    }
    deserializer.deserialize_any(Stringish)
}

#[derive(Debug, Clone)]
pub struct ClashApi {
    pub base: String,
    pub secret: String,
    active: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClashProxyGroup {
    pub name: String,
    pub group_type: String,
    pub now: String,
    pub all: Vec<String>,
    pub alive: Option<bool>,
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
}

fn shared_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .max_idle_connections(0)
            .timeout_connect(Duration::from_secs(3))
            .timeout(Duration::from_secs(30))
            .build()
    })
}

fn map_ureq(err: ureq::Error) -> AppError {
    AppError::Core(format!("clash_api: {err}"))
}

fn auth(secret: &str) -> String {
    format!("Bearer {secret}")
}

/// No-op kept for call sites that used to warm reqwest blocking.
pub fn warmup_blocking_client() {
    let _ = shared_agent();
}

impl ClashApi {
    pub fn new(host: &str, port: u16, secret: &str) -> Self {
        Self {
            base: format!("http://{host}:{port}"),
            secret: secret.to_string(),
            active: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Clones from one core session share the same activity token.
    pub fn same_session(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.active, &other.active)
    }

    /// Fast readiness probe (short timeout). Used while waiting for core start.
    pub fn health_ok(&self) -> bool {
        shared_agent()
            .get(&format!("{}/version", self.base))
            .set("Authorization", &auth(&self.secret))
            .timeout(Duration::from_millis(350))
            .call()
            .map(|r| (200..300).contains(&r.status()))
            .unwrap_or(false)
    }

    /// Close all active connections (`DELETE /connections`).
    pub fn close_all_connections(&self) -> AppResult<()> {
        let resp = shared_agent()
            .delete(&format!("{}/connections", self.base))
            .set("Authorization", &auth(&self.secret))
            .timeout(Duration::from_secs(3))
            .call()
            .map_err(map_ureq)?;
        if !(200..300).contains(&resp.status()) {
            return Err(AppError::Core(format!(
                "close connections status {}",
                resp.status()
            )));
        }
        Ok(())
    }

    /// Switch selector/urltest group `proxy` to outbound `name` (tag).
    pub fn select_proxy(&self, group: &str, name: &str) -> AppResult<()> {
        let body = serde_json::json!({ "name": name });
        let encoded = urlencoding::encode(group);
        let resp = shared_agent()
            .put(&format!("{}/proxies/{encoded}", self.base))
            .set("Authorization", &auth(&self.secret))
            .timeout(Duration::from_secs(3))
            .send_json(body)
            .map_err(map_ureq)?;
        if !(200..300).contains(&resp.status()) {
            return Err(AppError::Core(format!(
                "clash_api select status {}",
                resp.status()
            )));
        }
        Ok(())
    }

    /// Current selected outbound tag of a proxy group (`GET /proxies/{group}` → `now`).
    pub fn proxy_group_now_with_timeout(
        &self,
        group: &str,
        timeout: Duration,
    ) -> AppResult<Option<String>> {
        let encoded = urlencoding::encode(group);
        let resp = shared_agent()
            .get(&format!("{}/proxies/{encoded}", self.base))
            .set("Authorization", &auth(&self.secret))
            .timeout(timeout)
            .call()
            .map_err(map_ureq)?;
        if !(200..300).contains(&resp.status()) {
            return Err(AppError::Core(format!(
                "clash_api proxy now status {}",
                resp.status()
            )));
        }
        #[derive(Deserialize)]
        struct ProxyBody {
            #[serde(default)]
            now: Option<String>,
        }
        let body: ProxyBody = resp
            .into_json()
            .map_err(|e| AppError::Core(format!("proxy now json: {e}")))?;
        Ok(body.now.filter(|s| !s.is_empty()))
    }

    /// Live Clash policy groups (`GET /proxies`). Plain node adapters and
    /// DIRECT/REJECT are omitted; only switchable/automatic group adapters
    /// with a non-empty member list are returned.
    pub fn list_proxy_groups(&self) -> AppResult<Vec<ClashProxyGroup>> {
        let resp = shared_agent()
            .get(&format!("{}/proxies", self.base))
            .set("Authorization", &auth(&self.secret))
            .timeout(Duration::from_secs(3))
            .call()
            .map_err(map_ureq)?;
        if !(200..300).contains(&resp.status()) {
            return Err(AppError::Core(format!(
                "clash_api proxies status {}",
                resp.status()
            )));
        }
        let body: serde_json::Value = resp
            .into_json()
            .map_err(|error| AppError::Core(format!("clash_api proxies json: {error}")))?;
        let proxies = body
            .get("proxies")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| AppError::Core("clash_api proxies map missing".into()))?;
        let mut groups = proxies
            .iter()
            .filter_map(|(name, value)| {
                let members = value.get("all")?.as_array()?;
                if members.is_empty() {
                    return None;
                }
                Some(ClashProxyGroup {
                    name: name.clone(),
                    group_type: value
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("Unknown")
                        .to_string(),
                    now: value
                        .get("now")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    all: members
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_string)
                        .collect(),
                    alive: value.get("alive").and_then(serde_json::Value::as_bool),
                    labels: std::collections::BTreeMap::new(),
                })
            })
            .collect::<Vec<_>>();
        groups.sort_by(|left, right| {
            let rank = |name: &str| match name {
                "proxy" => 0,
                "auto" => 1,
                _ => 2,
            };
            rank(&left.name)
                .cmp(&rank(&right.name))
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(groups)
    }

    pub fn delay(&self, proxy: &str, url: &str, timeout_ms: u64) -> AppResult<u32> {
        let encoded = urlencoding::encode(proxy);
        let resp = shared_agent()
            .get(&format!("{}/proxies/{encoded}/delay", self.base))
            .query("url", url)
            .query("timeout", &timeout_ms.to_string())
            .set("Authorization", &auth(&self.secret))
            .timeout(Duration::from_millis(timeout_ms.saturating_add(1000)))
            .call()
            .map_err(map_ureq)?;
        if !(200..300).contains(&resp.status()) {
            return Err(AppError::Core(format!("delay status {}", resp.status())));
        }
        #[derive(Deserialize)]
        struct DelayBody {
            delay: u32,
        }
        let body: DelayBody = resp
            .into_json()
            .map_err(|e| AppError::Core(format!("delay json: {e}")))?;
        Ok(body.delay)
    }

    /// Full connections snapshot from `/connections` (HTTP).
    pub fn list_connections(&self) -> AppResult<ConnectionsSnapshot> {
        let resp = shared_agent()
            .get(&format!("{}/connections", self.base))
            .set("Authorization", &auth(&self.secret))
            .timeout(Duration::from_secs(3))
            .call()
            .map_err(map_ureq)?;
        if !(200..300).contains(&resp.status()) {
            return Err(AppError::Core(format!(
                "connections status {}",
                resp.status()
            )));
        }
        let text = resp
            .into_string()
            .map_err(|e| AppError::Core(format!("connections body: {e}")))?;
        parse_connections_json(&text)
    }

    /// WebSocket URL for streaming connection snapshots (`interval` in ms).
    /// sing-box Clash API defaults to 1000ms; lower values catch more short-lived conns.
    pub fn connections_ws_url(&self, interval_ms: u64) -> String {
        let base = self
            .base
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        // token= works for WS without custom headers (also send Authorization).
        format!(
            "{base}/connections?interval={interval_ms}&token={}",
            urlencoding::encode(&self.secret)
        )
    }
}

/// Parse Clash `/connections` JSON body (HTTP or WebSocket text frame).
pub fn parse_connections_json(text: &str) -> AppResult<ConnectionsSnapshot> {
    let body: RawConnectionsBody = serde_json::from_str(text).map_err(|e| {
        AppError::Core(format!(
            "connections json: {e}; body={}",
            text.chars().take(240).collect::<String>()
        ))
    })?;

    let connections: Vec<ConnectionInfo> = body
        .connections
        .unwrap_or_default()
        .into_iter()
        .map(ConnectionInfo::from_raw)
        .collect();

    Ok(ConnectionsSnapshot {
        upload_total: body.upload_total,
        download_total: body.download_total,
        connections,
    })
}

#[derive(Debug, Deserialize)]
struct RawConnectionsBody {
    #[serde(default, rename = "downloadTotal", alias = "download_total")]
    download_total: u64,
    #[serde(default, rename = "uploadTotal", alias = "upload_total")]
    upload_total: u64,
    /// May be omitted / null when idle.
    #[serde(default)]
    connections: Option<Vec<RawConnection>>,
}

#[derive(Debug, Deserialize)]
struct RawConnection {
    #[serde(default, deserialize_with = "deserialize_stringish")]
    id: String,
    #[serde(default)]
    metadata: RawMetadata,
    #[serde(default)]
    upload: u64,
    #[serde(default)]
    download: u64,
    #[serde(default, deserialize_with = "deserialize_stringish")]
    start: String,
    #[serde(default)]
    chains: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_stringish")]
    rule: String,
    #[serde(
        default,
        rename = "rulePayload",
        alias = "rule_payload",
        deserialize_with = "deserialize_stringish"
    )]
    rule_payload: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawMetadata {
    #[serde(default, deserialize_with = "deserialize_stringish")]
    network: String,
    #[serde(default, rename = "type", deserialize_with = "deserialize_stringish")]
    conn_type: String,
    #[serde(
        default,
        rename = "sourceIP",
        alias = "source_ip",
        deserialize_with = "deserialize_stringish"
    )]
    source_ip: String,
    #[serde(
        default,
        rename = "destinationIP",
        alias = "destination_ip",
        deserialize_with = "deserialize_stringish"
    )]
    destination_ip: String,
    #[serde(
        default,
        rename = "sourcePort",
        alias = "source_port",
        deserialize_with = "deserialize_stringish"
    )]
    source_port: String,
    #[serde(
        default,
        rename = "destinationPort",
        alias = "destination_port",
        deserialize_with = "deserialize_stringish"
    )]
    destination_port: String,
    #[serde(default, deserialize_with = "deserialize_stringish")]
    host: String,
    #[serde(
        default,
        rename = "processPath",
        alias = "process_path",
        deserialize_with = "deserialize_stringish"
    )]
    process_path: String,
    #[serde(
        default,
        rename = "process",
        deserialize_with = "deserialize_stringish"
    )]
    process: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub id: String,
    /// e.g. www.google.com:443 or 1.2.3.4:443
    pub destination: String,
    pub host: String,
    pub destination_ip: String,
    pub destination_port: String,
    pub network: String,
    pub conn_type: String,
    pub source: String,
    pub process: String,
    /// Full chain, first is usually selector or last hop name
    pub chains: Vec<String>,
    /// Primary node used (last non-selector tag preferred)
    pub node: String,
    pub rule: String,
    pub rule_payload: String,
    pub upload: u64,
    pub download: u64,
    pub start: String,
}

impl ConnectionInfo {
    fn from_raw(raw: RawConnection) -> Self {
        let host = raw.metadata.host.clone();
        let dip = raw.metadata.destination_ip.clone();
        let dport = raw.metadata.destination_port.clone();
        let destination = if !host.is_empty() {
            if dport.is_empty() {
                host.clone()
            } else {
                format!("{host}:{dport}")
            }
        } else if !dip.is_empty() {
            if dport.is_empty() {
                dip.clone()
            } else {
                format!("{dip}:{dport}")
            }
        } else {
            "—".into()
        };

        let source = format!("{}:{}", raw.metadata.source_ip, raw.metadata.source_port);

        let process = if !raw.metadata.process.is_empty() {
            raw.metadata.process
        } else if !raw.metadata.process_path.is_empty() {
            // basename
            raw.metadata
                .process_path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&raw.metadata.process_path)
                .to_string()
        } else {
            String::new()
        };

        // chains: [node, proxy] or [node, auto, proxy] — last hop toward outbound is usually first element
        let node = pick_node_from_chains(&raw.chains);

        Self {
            id: raw.id,
            destination,
            host,
            destination_ip: dip,
            destination_port: dport,
            network: raw.metadata.network,
            conn_type: raw.metadata.conn_type,
            source,
            process,
            chains: raw.chains,
            node,
            rule: raw.rule,
            rule_payload: raw.rule_payload,
            upload: raw.upload,
            download: raw.download,
            start: raw.start,
        }
    }
}

fn pick_node_from_chains(chains: &[String]) -> String {
    // Clash: chains[0] is the outbound leaf often; skip generic names
    let skip = ["proxy", "auto", "GLOBAL", "global", "select", "url-test"];
    for name in chains {
        let lower = name.to_ascii_lowercase();
        if skip.iter().any(|s| lower == *s) {
            continue;
        }
        if name.starts_with("node-") || !name.is_empty() {
            return name.clone();
        }
    }
    chains.first().cloned().unwrap_or_else(|| "—".into())
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionsSnapshot {
    pub upload_total: u64,
    pub download_total: u64,
    pub connections: Vec<ConnectionInfo>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TrafficTotals {
    pub upload_total: u64,
    pub download_total: u64,
    pub connections: u32,
}

/// Historical request record (accumulated while core runs).
#[derive(Debug, Clone, Serialize)]
pub struct RequestRecord {
    pub id: String,
    /// Monotonic sequence assigned whenever the request becomes closed/history-visible.
    #[serde(default)]
    pub history_seq: u64,
    pub destination: String,
    pub host: String,
    pub network: String,
    pub conn_type: String,
    pub node: String,
    pub chains: Vec<String>,
    pub rule: String,
    pub rule_payload: String,
    pub process: String,
    pub source: String,
    pub upload: u64,
    pub download: u64,
    /// First seen unix ms (by our journal)
    pub first_seen: i64,
    /// Last updated unix ms
    pub last_seen: i64,
    /// True after connection disappears from live snapshot.
    #[serde(default)]
    pub closed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<i64>,
}

impl RequestRecord {
    pub fn from_connection(c: &ConnectionInfo, now_ms: i64) -> Self {
        Self {
            id: c.id.clone(),
            history_seq: 0,
            destination: c.destination.clone(),
            host: c.host.clone(),
            network: c.network.clone(),
            conn_type: c.conn_type.clone(),
            node: c.node.clone(),
            chains: c.chains.clone(),
            rule: c.rule.clone(),
            rule_payload: c.rule_payload.clone(),
            process: c.process.clone(),
            source: c.source.clone(),
            upload: c.upload,
            download: c.download,
            first_seen: now_ms,
            last_seen: now_ms,
            closed: false,
            closed_at: None,
        }
    }

    pub fn matches_query(&self, q: &str) -> bool {
        if q.is_empty() {
            return true;
        }
        let q = q.to_ascii_lowercase();
        let hay = [
            self.destination.as_str(),
            self.host.as_str(),
            self.node.as_str(),
            self.rule.as_str(),
            self.rule_payload.as_str(),
            self.process.as_str(),
            self.network.as_str(),
            self.conn_type.as_str(),
            self.source.as_str(),
            &self.chains.join(" > "),
        ];
        hay.iter().any(|s| s.to_ascii_lowercase().contains(&q))
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn read_http_request(socket: &mut std::net::TcpStream) -> String {
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let read = socket.read(&mut chunk).expect("read HTTP request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end + 4]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        String::from_utf8(request).expect("HTTP request text")
    }

    #[test]
    fn close_all_connections_uses_authenticated_delete() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake clash api");
        let port = listener.local_addr().expect("fake api address").port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept close request");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            loop {
                let read = socket.read(&mut chunk).expect("read close request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).expect("http request text");
            assert!(request.starts_with("DELETE /connections HTTP/1.1\r\n"));
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer test-secret\r\n"));
            socket
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .expect("write close response");
        });

        ClashApi::new("127.0.0.1", port, "test-secret")
            .close_all_connections()
            .expect("close all connections");
        server.join().expect("fake clash api server");
    }

    #[test]
    fn lists_only_policy_groups_in_clash_order() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake clash api");
        let port = listener.local_addr().expect("fake api address").port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept proxies request");
            let request = read_http_request(&mut socket);
            assert!(request.starts_with("GET /proxies HTTP/1.1\r\n"));
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer test-secret\r\n"));
            let body = r#"{"proxies":{"node-a":{"type":"VLESS","alive":true},"搜索引擎":{"type":"Selector","now":"proxy","all":["proxy","DIRECT"]},"auto":{"type":"URLTest","now":"node-a","all":["node-a"]},"proxy":{"type":"Selector","now":"auto","all":["auto","node-a"]}}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .expect("write proxies response");
        });

        let groups = ClashApi::new("127.0.0.1", port, "test-secret")
            .list_proxy_groups()
            .expect("list policy groups");
        server.join().expect("fake clash api server");
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].name, "proxy");
        assert_eq!(groups[1].name, "auto");
        assert_eq!(groups[2].name, "搜索引擎");
        assert_eq!(groups[0].now, "auto");
        assert_eq!(groups[2].all, ["proxy", "DIRECT"]);
    }

    #[test]
    fn selects_unicode_group_with_encoded_path() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake clash api");
        let port = listener.local_addr().expect("fake api address").port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept select request");
            let request = read_http_request(&mut socket);
            assert!(request
                .starts_with("PUT /proxies/%E6%90%9C%E7%B4%A2%E5%BC%95%E6%93%8E HTTP/1.1\r\n"));
            assert!(request.contains(r#"{"name":"DIRECT"}"#));
            socket
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .expect("write select response");
        });

        ClashApi::new("127.0.0.1", port, "test-secret")
            .select_proxy("搜索引擎", "DIRECT")
            .expect("select policy member");
        server.join().expect("fake clash api server");
    }

    #[test]
    fn connections_json_numeric_ports() {
        let raw = r#"{
          "downloadTotal": 1,
          "uploadTotal": 2,
          "connections": [{
            "id": "abc",
            "upload": 10,
            "download": 20,
            "start": "2024-01-01T00:00:00Z",
            "chains": ["node-1", "proxy"],
            "rule": "GeoIP",
            "rulePayload": "CN",
            "metadata": {
              "network": "tcp",
              "type": "HTTP",
              "sourceIP": "127.0.0.1",
              "destinationIP": "1.2.3.4",
              "sourcePort": 54321,
              "destinationPort": 443,
              "host": "example.com",
              "processPath": "/Apps/Foo.app",
              "process": "Foo"
            }
          }]
        }"#;
        let body: RawConnectionsBody = serde_json::from_str(raw).expect("parse");
        let conns = body.connections.unwrap_or_default();
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0].metadata.destination_port, "443");
        assert_eq!(conns[0].metadata.source_port, "54321");
        assert_eq!(conns[0].metadata.host, "example.com");
        let info = ConnectionInfo::from_raw(conns.into_iter().next().unwrap());
        assert_eq!(info.destination, "example.com:443");
    }

    #[test]
    fn connections_null_ok() {
        let raw = r#"{"connections":null,"downloadTotal":1,"uploadTotal":2}"#;
        let body: RawConnectionsBody = serde_json::from_str(raw).expect("parse");
        assert!(body.connections.unwrap_or_default().is_empty());
        let snap = parse_connections_json(raw).expect("snap");
        assert!(snap.connections.is_empty());
        assert_eq!(snap.download_total, 1);
    }
}
