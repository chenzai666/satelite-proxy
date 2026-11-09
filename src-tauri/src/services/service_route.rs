//! 服务单次响应的连接证据与受控出口诊断。
//!
//! `scope` 必须包住每个服务任务（task-local 不自动传播到新 spawn 的任务）。
//! 在读取正文前调用 `observe`，正文中的可信 IP 用 `record_service_ip` 归入同一 URL。
//! `finish` 在业务检测完成后调用，避免出口查询占用服务正文的超时预算。
//! 本模块只允许切换 `service-diag`，不持有 runtime/store 锁。

use crate::api::{ClashApi, ConnectionInfo};
use hyper_util::client::legacy::connect::HttpInfo;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use url::Url;

const MAX_CONFIG_BYTES: usize = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 8;
const MAX_NODES: usize = 8192;
const MAX_MEMBERS: usize = 65536;
const MAX_CHAIN: usize = 64;
const DIAG_GROUP: &str = "service-diag";
const DIAG_INBOUND: &str = "service-diag-in";
const DIAG_USER: &str = "service-probe";
const OBSERVE_BUDGET: Duration = Duration::from_millis(500);
const CACHE_TTL: Duration = Duration::from_secs(30);
const CACHE_LIMIT: usize = 64;
const DIAG_WAIT: Duration = Duration::from_secs(8);
const EXIT_TIMEOUT: Duration = Duration::from_secs(4);
const FINISH_BUDGET: Duration = Duration::from_secs(8);

tokio::task_local! {
    static CURRENT: Arc<RequestContext>;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestRoute {
    pub role: &'static str,
    pub host: String,
    pub match_status: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub chains: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outbound_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_source: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_cached: Option<bool>,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum Family {
    Any,
    V4,
    V6,
}

impl Family {
    fn from_target(ip: &str) -> Self {
        match public_ip(ip) {
            Some(IpAddr::V4(_)) => Self::V4,
            Some(IpAddr::V6(_)) => Self::V6,
            _ => Self::Any,
        }
    }

    fn accepts(self, ip: IpAddr) -> bool {
        matches!(self, Self::Any)
            || matches!(
                (self, ip),
                (Self::V4, IpAddr::V4(_)) | (Self::V6, IpAddr::V6(_))
            )
    }
}

#[derive(Clone)]
struct Recorded {
    url: Url,
    family: Family,
    route: RequestRoute,
}

#[derive(Default)]
struct Attempt {
    generation: u64,
    records: Vec<Recorded>,
}

pub struct RequestContext {
    route: Arc<RouteContext>,
    primary_url: Option<Url>,
    attempt: Mutex<Attempt>,
}

pub struct RouteContext {
    pub api: Option<ClashApi>,
    pub session_key: String,
    graph: ConfigGraph,
}

impl RouteContext {
    /// 没有配置图时仍可观察规则/chains，不能推导叶子或启用诊断。
    pub fn new(api: Option<ClashApi>, session_key: String) -> Arc<Self> {
        Arc::new(Self {
            api,
            session_key,
            graph: ConfigGraph::default(),
        })
    }

    /// 仅用于当前运行配置；文件读取与解析移至 blocking worker，最多读取 16MiB。
    /// 失败时降级到观察模式，不将配置正文或认证信息放入错误/日志。
    pub async fn from_active_config(
        api: Option<ClashApi>,
        session_key: String,
        config_path: Option<PathBuf>,
        generated: bool,
    ) -> Arc<Self> {
        let expected_secret = api.as_ref().map(|api| api.secret.clone());
        let graph = tokio::task::spawn_blocking(move || {
            let path = config_path?;
            let file = std::fs::File::open(path).ok()?;
            let mut bytes = Vec::new();
            file.take((MAX_CONFIG_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .ok()?;
            ConfigGraph::parse(&bytes, generated, expected_secret.as_deref())
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        Arc::new(Self {
            api,
            session_key,
            graph,
        })
    }

    /// 已读取配置的调用方可用此入口；同样执行大小和能力校验。
    pub fn from_config(
        api: Option<ClashApi>,
        session_key: String,
        content: &str,
        generated: bool,
    ) -> Arc<Self> {
        let graph = ConfigGraph::parse(
            content.as_bytes(),
            generated,
            api.as_ref().map(|api| api.secret.as_str()),
        )
        .unwrap_or_default();
        Arc::new(Self {
            api,
            session_key,
            graph,
        })
    }

    pub fn has_diagnostic(&self) -> bool {
        self.graph.diag.is_some() && self.api.as_ref().is_some_and(ClashApi::is_active)
    }
}

impl RequestContext {
    pub fn new(route: Arc<RouteContext>, primary_url: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            route,
            primary_url: Url::parse(&primary_url.into()).ok(),
            attempt: Mutex::new(Attempt::default()),
        })
    }

    /// 在重试开始前调用；被取消的上一轮 observe/finish 不能再回填本轮。
    pub fn reset_attempt(&self) {
        let mut attempt = self.attempt.lock().unwrap_or_else(|e| e.into_inner());
        attempt.generation = attempt.generation.wrapping_add(1);
        attempt.records.clear();
    }

    pub fn records(&self) -> Vec<RequestRoute> {
        self.attempt
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .records
            .iter()
            .map(|record| record.route.clone())
            .collect()
    }

    /// 业务状态判定完成后补出口。失败不改变 HTTP/业务结果；同一路径使用短缓存。
    pub async fn finish(&self) -> Vec<RequestRoute> {
        let deadline = tokio::time::Instant::now() + FINISH_BUDGET;
        let (generation, records) = {
            let attempt = self.attempt.lock().unwrap_or_else(|e| e.into_inner());
            (attempt.generation, attempt.records.clone())
        };
        for (index, record) in records.into_iter().enumerate() {
            if record.route.match_status != "matched" || record.route.exit_ip.is_some() {
                continue;
            }
            let Some(tag) = record.route.outbound_tag else {
                continue;
            };
            if !self.current_generation(generation) {
                break;
            }
            let exit = match tokio::time::timeout_at(
                deadline,
                probe_exit(self.route.clone(), tag, record.family),
            )
            .await
            {
                Ok(Some(exit)) => exit,
                Ok(None) => continue,
                Err(_) => break,
            };
            let mut attempt = self.attempt.lock().unwrap_or_else(|e| e.into_inner());
            if attempt.generation != generation {
                break;
            }
            if let Some(record) = attempt.records.get_mut(index) {
                // 同期服务回显比另行出口探测更精确，不覆盖它。
                if record.route.exit_source != Some("service_response") {
                    record.route.exit_ip = Some(exit.ip);
                    record.route.exit_source = Some("pinned_outbound_probe");
                    record.route.exit_cached = Some(exit.cached);
                }
            }
        }
        self.records()
    }

    fn current_generation(&self, generation: u64) -> bool {
        self.attempt
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation
            == generation
    }

    fn reserve(&self, url: Url) -> Option<(u64, usize)> {
        let mut attempt = self.attempt.lock().unwrap_or_else(|e| e.into_inner());
        if attempt.records.len() >= MAX_RECORDS {
            return None;
        }
        let index = attempt.records.len();
        let role = if self.primary_url.as_ref() == Some(&url) {
            "primary"
        } else {
            "auxiliary"
        };
        let match_status = if self.route.api.as_ref().is_some_and(ClashApi::is_active) {
            "not_observed"
        } else {
            "unsupported"
        };
        attempt.records.push(Recorded {
            route: RequestRoute {
                role,
                host: url.host_str()?.to_owned(),
                match_status,
                chains: Vec::new(),
                rule: None,
                rule_payload: None,
                outbound_tag: None,
                exit_ip: None,
                exit_source: None,
                exit_cached: None,
            },
            url,
            family: Family::Any,
        });
        Some((attempt.generation, index))
    }
}

pub async fn scope<T>(context: Arc<RequestContext>, future: impl Future<Output = T>) -> T {
    CURRENT.scope(context, future).await
}

/// 响应仍存活时立即关联 HTTPS CONNECT 的客户端 socket；不会读取或消耗正文。
pub async fn observe(response: &reqwest::Response, url: &str) {
    let Ok(context) = CURRENT.try_with(Arc::clone) else {
        return;
    };
    let Some(target) = https_url(url) else { return };
    if response.url() != &target {
        return;
    }
    let Some((generation, index)) = context.reserve(target.clone()) else {
        return;
    };
    let Some(info) = response.extensions().get::<HttpInfo>() else {
        return;
    };
    let local = info.local_addr();
    let remote = info.remote_addr();
    // 当前探测必须通过 loopback 核心入口，不能将外部直连 socket 当作代理证据。
    if !local.ip().is_loopback() || !remote.ip().is_loopback() {
        return;
    }
    if !context.route.graph.http_ports.is_empty()
        && !context.route.graph.http_ports.contains(&remote.port())
    {
        return;
    }
    observe_identity(&context, generation, index, local, &target).await;
}

/// `validated_ip` 应来自该 URL 自身的受信正文；再次验证公网 IP 与完整 URL 归属。
/// 不向其他 URL、主请求或出口缓存传播，也不接受上一轮重试的记录。
pub fn record_service_ip(url: &str, validated_ip: &str) {
    let Ok(context) = CURRENT.try_with(Arc::clone) else {
        return;
    };
    let Some(url) = https_url(url) else { return };
    let Some(ip) = public_ip(validated_ip) else {
        return;
    };
    let mut attempt = context.attempt.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(record) = attempt
        .records
        .iter_mut()
        .rev()
        .find(|record| record.url == url)
    {
        record.route.exit_ip = Some(ip.to_string());
        record.route.exit_source = Some("service_response");
        record.route.exit_cached = Some(false);
    }
}

async fn observe_identity(
    context: &RequestContext,
    generation: u64,
    index: usize,
    local: SocketAddr,
    target: &Url,
) {
    let Some(api) = context.route.api.clone().filter(ClashApi::is_active) else {
        return;
    };
    // 许可随 blocking 调用一起存活；调用方超时不会不断堆积后台 API 请求。
    let Ok(permit) = observation_gate().try_acquire_owned() else {
        return;
    };
    let check_api = api.clone();
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if !api.is_active() {
            return None;
        }
        api.list_connections().ok()
    });
    let Ok(Ok(Some(snapshot))) = tokio::time::timeout(OBSERVE_BUDGET, task).await else {
        return;
    };
    if !check_api.is_active() {
        return;
    }
    let Some(connection) = exact_connection(&snapshot.connections, local, target) else {
        return;
    };
    let mut attempt = context.attempt.lock().unwrap_or_else(|e| e.into_inner());
    if attempt.generation != generation {
        return;
    }
    let Some(record) = attempt.records.get_mut(index) else {
        return;
    };
    record.family = Family::from_target(&connection.destination_ip);
    record.route.match_status = "matched";
    // 与前端完整结果校验边界一致。超限证据不截断，以免改变 tag/规则含义。
    record.route.chains = if connection.chains.len() <= 16
        && connection
            .chains
            .iter()
            .all(|tag| wire_text(tag, 256).is_some())
    {
        connection.chains.clone()
    } else {
        Vec::new()
    };
    record.route.rule = wire_text(&connection.rule, 512);
    record.route.rule_payload = wire_text(&connection.rule_payload, 512);
    record.route.outbound_tag = context
        .route
        .graph
        .leaf(&connection.chains)
        .filter(|tag| wire_text(tag, 256).is_some());
}

fn observation_gate() -> Arc<Semaphore> {
    static GATE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(Semaphore::new(4))).clone()
}

fn diagnostic_gate() -> Arc<Semaphore> {
    static GATE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(Semaphore::new(1))).clone()
}

fn https_url(raw: &str) -> Option<Url> {
    let url = Url::parse(raw).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}

fn wire_text(value: &str, max_utf16: usize) -> Option<String> {
    (!value.is_empty() && value.encode_utf16().take(max_utf16 + 1).count() <= max_utf16)
        .then(|| value.to_owned())
}

fn source_socket(source: &str) -> Option<SocketAddr> {
    source.parse().ok().or_else(|| {
        // 现有 ConnectionInfo 拼接 IPv6 时没有方括号，按最后一个冒号拆端口。
        let (ip, port) = source.rsplit_once(':')?;
        Some(SocketAddr::new(ip.parse().ok()?, port.parse().ok()?))
    })
}

fn exact_connection<'a>(
    connections: &'a [ConnectionInfo],
    local: SocketAddr,
    target: &Url,
) -> Option<&'a ConnectionInfo> {
    let host = target.host_str()?;
    let port = target.port_or_known_default()?;
    let mut matches = connections.iter().filter(|connection| {
        connection.network.eq_ignore_ascii_case("tcp")
            && source_socket(&connection.source) == Some(local)
            && connection.destination_port.parse::<u16>().ok() == Some(port)
            && exact_host(&connection.host, host)
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn exact_host(actual: &str, expected: &str) -> bool {
    // 只规范 DNS 的大小写、IDNA 和末尾根点；不做后缀/子串/IP 替代匹配。
    let normalize = |host: &str| -> Option<String> {
        let host = host.strip_suffix('.').unwrap_or(host);
        if host.is_empty() || host.contains(['/', ':', '@']) {
            return None;
        }
        idna::domain_to_ascii(host)
            .ok()
            .map(|host| host.to_ascii_lowercase())
    };
    match (normalize(actual), normalize(expected)) {
        (Some(actual), Some(expected)) => actual == expected,
        _ => false,
    }
}

#[derive(Default)]
struct ConfigGraph {
    nodes: HashMap<String, Node>,
    http_ports: HashSet<u16>,
    fingerprint: String,
    diag: Option<Diagnostic>,
}

struct Node {
    kind: String,
    members: Vec<String>,
    detour: Option<String>,
}

struct Diagnostic {
    port: u16,
    secret: String,
    members: HashSet<String>,
}

impl Node {
    fn group(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "selector" | "select" | "urltest" | "url-test" | "fallback" | "load-balance" | "relay"
        )
    }

    fn terminal(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "direct"
                | "socks"
                | "socks5"
                | "http"
                | "https"
                | "shadowsocks"
                | "ss"
                | "ssr"
                | "vmess"
                | "vless"
                | "trojan"
                | "hysteria"
                | "hysteria2"
                | "hy2"
                | "tuic"
                | "anytls"
                | "wireguard"
                | "ssh"
                | "naive"
                | "tor"
                | "snell"
                | "mieru"
                | "masque"
                | "sudoku"
                | "trusttunnel"
                | "openvpn"
                | "juicity"
        )
    }

    // ShadowTLS 只能作为拨号传输依赖，不是可供服务 HTTP 请求使用的出口。
    fn transport_dependency(&self) -> bool {
        self.terminal() || self.kind == "shadowtls"
    }
}

impl ConfigGraph {
    fn parse(bytes: &[u8], generated: bool, expected_secret: Option<&str>) -> Option<Self> {
        use sha2::Digest;
        if bytes.len() > MAX_CONFIG_BYTES {
            return None;
        }
        let document: Value = serde_json::from_slice(bytes)
            .ok()
            .or_else(|| serde_yaml::from_slice(bytes).ok())?;
        let mihomo = document.get("proxies").is_some();
        let outbounds = document
            .get(if mihomo { "proxies" } else { "outbounds" })?
            .as_array()?;
        let mut graph = Self {
            fingerprint: hex::encode(sha2::Sha256::digest(bytes)),
            ..Self::default()
        };
        for outbound in outbounds {
            graph.add_node(outbound, mihomo)?;
        }
        if mihomo {
            if let Some(groups) = document.get("proxy-groups").and_then(Value::as_array) {
                for group in groups {
                    graph.add_node(group, true)?;
                }
            }
            // Mihomo 的内置直连与 GLOBAL 没有配置节点，仍须显式分类，不能按名称猜叶子。
            graph.nodes.entry("DIRECT".into()).or_insert(Node {
                kind: "direct".into(),
                members: Vec::new(),
                detour: None,
            });
            let members = graph
                .nodes
                .keys()
                .filter(|tag| tag.as_str() != "GLOBAL")
                .cloned()
                .collect();
            graph.nodes.entry("GLOBAL".into()).or_insert(Node {
                kind: "select".into(),
                members,
                detour: None,
            });
            for key in ["mixed-port", "port"] {
                if let Some(port) = document.get(key).and_then(port_value) {
                    graph.http_ports.insert(port);
                }
            }
        }
        let inbounds = document
            .get(if mihomo { "listeners" } else { "inbounds" })
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for inbound in inbounds {
            if matches!(
                inbound.get("type").and_then(Value::as_str),
                Some("http" | "mixed")
            ) {
                if let Some(port) = inbound
                    .get(if mihomo { "port" } else { "listen_port" })
                    .and_then(port_value)
                {
                    graph.http_ports.insert(port);
                }
            }
        }
        if generated {
            graph.diag = graph.diagnostic(&document, inbounds, mihomo, expected_secret);
        }
        Some(graph)
    }

    fn add_node(&mut self, value: &Value, mihomo: bool) -> Option<()> {
        if self.nodes.len() >= MAX_NODES {
            return None;
        }
        let tag = value.get(if mihomo { "name" } else { "tag" })?.as_str()?;
        let kind = value.get("type")?.as_str()?.to_ascii_lowercase();
        if tag.is_empty() || tag.len() > 1024 || self.nodes.contains_key(tag) {
            return None;
        }
        let members = match value.get(if mihomo { "proxies" } else { "outbounds" }) {
            Some(members) => members
                .as_array()?
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        if members.len() > MAX_MEMBERS
            || self
                .nodes
                .values()
                .map(|node| node.members.len())
                .sum::<usize>()
                .saturating_add(members.len())
                > MAX_MEMBERS
        {
            return None;
        }
        let detour = match value.get(if mihomo { "dialer-proxy" } else { "detour" }) {
            Some(value) => Some(value.as_str()?.to_owned()),
            None => None,
        };
        self.nodes.insert(
            tag.into(),
            Node {
                kind,
                members,
                detour,
            },
        );
        Some(())
    }

    fn diagnostic(
        &self,
        document: &Value,
        inbounds: &[Value],
        mihomo: bool,
        expected_secret: Option<&str>,
    ) -> Option<Diagnostic> {
        let secret = if mihomo {
            document.get("secret")
        } else {
            document.pointer("/experimental/clash_api/secret")
        }?
        .as_str()?;
        if secret.is_empty() || Some(secret) != expected_secret {
            return None;
        }
        let mut candidates = inbounds.iter().filter(|inbound| {
            inbound
                .get(if mihomo { "name" } else { "tag" })
                .and_then(Value::as_str)
                == Some(DIAG_INBOUND)
        });
        let inbound = candidates.next()?;
        if candidates.next().is_some()
            || inbound.get("type").and_then(Value::as_str) != Some("mixed")
            || inbound.get("listen").and_then(Value::as_str) != Some("127.0.0.1")
        {
            return None;
        }
        let port = inbound
            .get(if mihomo { "port" } else { "listen_port" })
            .and_then(port_value)?;
        let users = inbound.get("users")?.as_array()?;
        let mut auth = users
            .iter()
            .filter(|user| user.get("username").and_then(Value::as_str) == Some(DIAG_USER));
        if auth.next()?.get("password").and_then(Value::as_str) != Some(secret)
            || auth.next().is_some()
        {
            return None;
        }
        let group = self.nodes.get(DIAG_GROUP)?;
        if !matches!(group.kind.as_str(), "selector" | "select") || group.members.is_empty() {
            return None;
        }
        if group
            .members
            .iter()
            .any(|tag| !self.nodes.get(tag).is_some_and(Node::transport_dependency))
        {
            return None;
        }
        if mihomo {
            if inbound.get("proxy").and_then(Value::as_str) != Some(DIAG_GROUP) {
                return None;
            }
            let hidden = document
                .get("proxy-groups")?
                .as_array()?
                .iter()
                .find(|group| group.get("name").and_then(Value::as_str) == Some(DIAG_GROUP))?
                .get("hidden")?
                .as_bool()?;
            if !hidden {
                return None;
            }
        } else {
            let valid_rule = document
                .pointer("/route/rules")?
                .as_array()?
                .iter()
                .any(|rule| {
                    rule.get("action").and_then(Value::as_str) == Some("route")
                        && rule.get("outbound").and_then(Value::as_str) == Some(DIAG_GROUP)
                        && rule
                            .get("inbound")
                            .and_then(Value::as_array)
                            .is_some_and(|tags| {
                                tags.len() == 1 && tags[0].as_str() == Some(DIAG_INBOUND)
                            })
                });
            if !valid_rule {
                return None;
            }
        }
        Some(Diagnostic {
            port,
            secret: secret.into(),
            members: group
                .members
                .iter()
                .filter(|tag| self.nodes.get(*tag).is_some_and(Node::terminal))
                .cloned()
                .collect(),
        })
    }

    fn leaf(&self, chains: &[String]) -> Option<String> {
        if chains.is_empty() || chains.len() > MAX_CHAIN {
            return None;
        }
        let mut leaves = HashSet::new();
        for tag in chains {
            let node = self.nodes.get(tag)?;
            if node.terminal() {
                leaves.insert(tag);
            } else if !node.group() {
                return None;
            }
        }
        if leaves.len() != 1 {
            return None;
        }
        let leaf = *leaves.iter().next()?;
        for group in chains.iter().filter(|tag| *tag != leaf) {
            if !self.reaches(group, leaf) {
                return None;
            }
        }
        if !self.valid_detour(leaf, &mut HashSet::new(), &mut HashSet::new()) {
            return None;
        }
        Some(leaf.clone())
    }

    fn reaches(&self, tag: &str, leaf: &str) -> bool {
        // 成员数量不是链深度：GLOBAL/节点池可能包含数千叶子，使用有界迭代遍历。
        let mut pending = vec![tag];
        let mut seen = HashSet::new();
        while let Some(tag) = pending.pop() {
            if tag == leaf {
                return true;
            }
            if !seen.insert(tag) {
                continue;
            }
            if seen.len() > MAX_NODES {
                return false;
            }
            if let Some(node) = self.nodes.get(tag).filter(|node| node.group()) {
                pending.extend(node.members.iter().map(String::as_str));
            }
        }
        false
    }

    fn valid_detour(
        &self,
        tag: &str,
        seen: &mut HashSet<String>,
        done: &mut HashSet<String>,
    ) -> bool {
        if done.contains(tag) {
            return true;
        }
        if seen.len() >= MAX_CHAIN || !seen.insert(tag.into()) {
            return false;
        }
        let result = self.nodes.get(tag).is_some_and(|node| {
            if node.group() {
                !node.members.is_empty()
                    && node
                        .members
                        .iter()
                        .all(|member| self.valid_detour(member, seen, done))
            } else if !node.transport_dependency() {
                false
            } else {
                node.detour
                    .as_ref()
                    .is_none_or(|detour| self.valid_detour(detour, seen, done))
            }
        });
        seen.remove(tag);
        if result {
            done.insert(tag.into());
        }
        result
    }
}

fn port_value(value: &Value) -> Option<u16> {
    let port = value.as_u64().and_then(|port| u16::try_from(port).ok())?;
    (port != 0).then_some(port)
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct CacheKey {
    session: String,
    config: String,
    tag: String,
    family: Family,
}

struct CachedExit {
    ip: String,
    at: Instant,
}

struct ExitObservation {
    ip: String,
    cached: bool,
}

fn cache() -> &'static Mutex<HashMap<CacheKey, CachedExit>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, CachedExit>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_exit(key: &CacheKey) -> Option<String> {
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, value| value.at.elapsed() < CACHE_TTL);
    cache.get(key).map(|value| value.ip.clone())
}

fn cache_exit(key: CacheKey, ip: String) {
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, value| value.at.elapsed() < CACHE_TTL);
    if cache.len() >= CACHE_LIMIT {
        let oldest = cache
            .iter()
            .min_by_key(|(_, value)| value.at)
            .map(|(key, _)| key.clone());
        if let Some(key) = oldest {
            cache.remove(&key);
        }
    }
    cache.insert(
        key,
        CachedExit {
            ip,
            at: Instant::now(),
        },
    );
}

async fn probe_exit(
    context: Arc<RouteContext>,
    tag: String,
    family: Family,
) -> Option<ExitObservation> {
    let api = context.api.clone().filter(ClashApi::is_active)?;
    let diag = context.graph.diag.as_ref()?;
    if !diag.members.contains(&tag) {
        return None;
    }
    let key = CacheKey {
        session: context.session_key.clone(),
        config: context.graph.fingerprint.clone(),
        tag: tag.clone(),
        family,
    };
    if let Some(ip) = cached_exit(&key) {
        return Some(ExitObservation { ip, cached: true });
    }
    let permit = tokio::time::timeout(DIAG_WAIT, diagnostic_gate().acquire_owned())
        .await
        .ok()?
        .ok()?;
    if !api.is_active() {
        return None;
    }
    if let Some(ip) = cached_exit(&key) {
        return Some(ExitObservation { ip, cached: true });
    }
    let port = diag.port;
    let secret = diag.secret.clone();
    let deadline = Instant::now() + EXIT_TIMEOUT;
    // 取消 finish 仅停止等待。工作任务继续持有许可，直至 blocking PUT 和 HTTP 均结束。
    // 这样旧 PUT 不能在许可被提前释放后覆盖下一条出口；没有无界排队的后台任务。
    tokio::spawn(async move {
        let (permit, api) = select_diagnostic(api, tag.clone(), permit, deadline).await?;
        let _permit = permit;
        if !api.is_active() {
            return None;
        }
        let remaining = deadline.checked_duration_since(Instant::now())?;
        let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))
            .ok()?
            .basic_auth(DIAG_USER, &secret);
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(proxy)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(remaining.min(Duration::from_secs(2)))
            .timeout(remaining)
            .pool_max_idle_per_host(0)
            .http1_only()
            .user_agent("Satelite/1.0.46 ServiceRoute")
            .build()
            .ok()?;
        let remaining = deadline.checked_duration_since(Instant::now())?;
        let ip = tokio::time::timeout(remaining, fetch_exit(&client, family))
            .await
            .ok()??;
        if !api.is_active() {
            return None;
        }
        cache_exit(key, ip.clone());
        Some(ExitObservation { ip, cached: false })
    })
    .await
    .ok()
    .flatten()
}

async fn select_diagnostic(
    api: ClashApi,
    tag: String,
    permit: OwnedSemaphorePermit,
    deadline: Instant,
) -> Option<(OwnedSemaphorePermit, ClashApi)> {
    tokio::task::spawn_blocking(move || {
        // select_proxy 自带 3s 超时；随后核对/HTTPS 共享剩余预算。
        // 如果 blocking worker 被调度得过晚，不再开始 PUT。
        if !api.is_active()
            || deadline.saturating_duration_since(Instant::now()) < Duration::from_secs(3)
        {
            return None;
        }
        api.select_proxy(DIAG_GROUP, &tag).ok()?;
        if !api.is_active() {
            return None;
        }
        let remaining = deadline.checked_duration_since(Instant::now())?;
        if remaining < Duration::from_millis(10) {
            return None;
        }
        let now = api
            .proxy_group_now_with_timeout(DIAG_GROUP, remaining.min(Duration::from_millis(250)))
            .ok()??;
        if !api.is_active() || now != tag {
            return None;
        }
        Some((permit, api))
    })
    .await
    .ok()
    .flatten()
}

async fn fetch_exit(client: &reqwest::Client, family: Family) -> Option<String> {
    let endpoint = match family {
        Family::V4 => "https://ipv4.icanhazip.com/",
        Family::V6 => "https://ipv6.icanhazip.com/",
        Family::Any => "https://api.ip.sb/geoip",
    };
    let response = client
        .get(endpoint)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .ok()?;
    read_exit_response(response, family).await
}

async fn read_exit_response(mut response: reqwest::Response, family: Family) -> Option<String> {
    if !response.status().is_success() || response.content_length().is_some_and(|len| len > 16384) {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if chunk.len() > 16384usize.saturating_sub(body.len()) {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    let body = std::str::from_utf8(&body).ok()?;
    let ip = if family == Family::Any {
        let document: Value = serde_json::from_str(body).ok()?;
        public_ip(document.get("ip")?.as_str()?)?
    } else {
        public_ip(body)?
    };
    family.accepts(ip).then(|| ip.to_string())
}

fn public_ip(raw: &str) -> Option<IpAddr> {
    let ip = raw.trim().parse::<IpAddr>().ok()?;
    let valid = match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && a != 0
                && a < 240
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (b == 18 || b == 19))
                && !(a == 192 && b == 0 && c == 0)
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            segments[0] & 0xe000 == 0x2000
                && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
                && !(segments[0] == 0x3fff && segments[1] & 0xf000 == 0)
        }
    };
    valid.then_some(ip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::parse_connections_json;
    use serde_json::json;

    fn fixture(mihomo: bool, port: u16) -> Value {
        // 仅隔离测试配置使用的凭据，与真实运行会话无关。
        let secret = "ci-fixture";
        if mihomo {
            json!({
                "secret": secret, "mixed-port": 2080,
                "proxies": [
                    {"name":"node-a","type":"ss"},
                    {"name":"node-b","type":"ss"},
                    {"name":"Satelite-DIRECT","type":"direct","ip-version":"ipv4"}
                ],
                "proxy-groups": [
                    {"name":"proxy","type":"select","proxies":["node-a","node-b","Satelite-DIRECT"]},
                    {"name":DIAG_GROUP,"type":"select","hidden":true,
                        "proxies":["node-a","node-b","DIRECT","Satelite-DIRECT"]}
                ],
                "listeners":[{"name":DIAG_INBOUND,"type":"mixed","listen":"127.0.0.1",
                    "port":port,"proxy":DIAG_GROUP,
                    "users":[{"username":DIAG_USER,"password":secret}]}]
            })
        } else {
            json!({
                "experimental":{"clash_api":{"secret":secret}},
                "outbounds":[
                    {"tag":"node-a","type":"shadowsocks"},
                    {"tag":"node-b","type":"socks"},
                    {"tag":"direct","type":"direct"},
                    {"tag":"chain-exit","type":"shadowsocks","detour":"node-a"},
                    {"tag":"proxy","type":"selector","outbounds":["node-a","node-b","chain-exit"]},
                    {"tag":DIAG_GROUP,"type":"selector","outbounds":["node-a","node-b","direct","chain-exit"]}
                ],
                "inbounds":[
                    {"tag":"mixed-in","type":"mixed","listen":"127.0.0.1","listen_port":2080},
                    {"tag":DIAG_INBOUND,"type":"mixed","listen":"127.0.0.1","listen_port":port,
                        "users":[{"username":DIAG_USER,"password":secret}]}
                ],
                "route":{"rules":[{"action":"sniff"},
                    {"inbound":[DIAG_INBOUND],"action":"route","outbound":DIAG_GROUP}]}
            })
        }
    }

    fn route(document: &Value, generated: bool) -> Arc<RouteContext> {
        RouteContext::from_config(
            Some(ClashApi::new("127.0.0.1", 1, "ci-fixture")),
            "ci-session".into(),
            &document.to_string(),
            generated,
        )
    }

    fn chains(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|tag| (*tag).into()).collect()
    }

    fn snapshot(rows: &[(u16, &str, &str)]) -> Vec<ConnectionInfo> {
        let rows: Vec<_> = rows.iter().enumerate().map(|(index, (port, host, network))| json!({
            "id":index.to_string(),
            "metadata":{"sourceIP":"127.0.0.1","sourcePort":port,
                "destinationPort":"443","host":host,"network":network,"destinationIP":"8.8.8.8"},
            "chains":["node-a","proxy"], "rule":"DomainSuffix", "rulePayload":"example.com"
        })).collect();
        parse_connections_json(&json!({"connections":rows}).to_string())
            .unwrap()
            .connections
    }

    #[test]
    fn exact_tuple_rejects_same_host_ambiguity_and_partial_matches() {
        assert!(wire_text(&"😀".repeat(128), 256).is_some());
        assert!(wire_text(&"😀".repeat(129), 256).is_none());
        assert!(wire_text(&"x".repeat(513), 512).is_none());
        let target = https_url("https://example.com/a").unwrap();
        let local = "127.0.0.1:50001".parse().unwrap();
        let rows = snapshot(&[
            (50002, "example.com", "tcp"),
            (50001, "EXAMPLE.COM.", "tcp"),
        ]);
        assert_eq!(exact_connection(&rows, local, &target).unwrap().id, "1");
        for rows in [
            snapshot(&[(50001, "example.com", "tcp"), (50001, "example.com", "tcp")]),
            snapshot(&[(50001, "example.com.attacker.invalid", "tcp")]),
            snapshot(&[(50001, "example.com", "udp")]),
            snapshot(&[(50002, "example.com", "tcp")]),
        ] {
            assert!(exact_connection(&rows, local, &target).is_none());
        }
        let mut wrong_port = rows.clone();
        wrong_port[1].destination_port = "8443".into();
        assert!(exact_connection(&wrong_port, local, &target).is_none());
        wrong_port[1].destination_port = "443".into();
        wrong_port[1].source = "127.0.0.2:50001".into();
        assert!(exact_connection(&wrong_port, local, &target).is_none());
        assert_eq!(
            source_socket("::1:50001"),
            Some("[::1]:50001".parse().unwrap())
        );
        assert!(exact_host("xn--bcher-kva.example", "bücher.example"));
        assert!(!exact_host("", "example.com"));
    }

    #[test]
    fn graph_preserves_observed_direct_and_detour_tags_without_now_guessing() {
        let singbox = route(&fixture(false, 27011), true);
        assert_eq!(
            singbox
                .graph
                .leaf(&chains(&["chain-exit", "proxy"]))
                .as_deref(),
            Some("chain-exit")
        );
        assert!(singbox.graph.leaf(&chains(&["proxy"])).is_none());
        assert!(singbox
            .graph
            .leaf(&chains(&["node-a", "node-b", "proxy"]))
            .is_none());
        assert!(singbox
            .graph
            .leaf(&chains(&["node-a", "missing"]))
            .is_none());
        let mihomo = route(&fixture(true, 27012), true);
        assert_eq!(
            mihomo
                .graph
                .leaf(&chains(&["Satelite-DIRECT", "proxy"]))
                .as_deref(),
            Some("Satelite-DIRECT")
        );
        assert_eq!(
            mihomo.graph.leaf(&chains(&["DIRECT"])).as_deref(),
            Some("DIRECT")
        );
        assert_eq!(
            mihomo
                .graph
                .leaf(&chains(&["Satelite-DIRECT", "GLOBAL"]))
                .as_deref(),
            Some("Satelite-DIRECT")
        );
        let mut cyclic = fixture(false, 27011);
        cyclic["outbounds"][0]["detour"] = json!("chain-exit");
        assert!(route(&cyclic, true)
            .graph
            .leaf(&chains(&["chain-exit", "proxy"]))
            .is_none());
        let mut transport = fixture(false, 27011);
        transport["outbounds"][0]["detour"] = json!("node-a-shadowtls");
        transport["outbounds"]
            .as_array_mut()
            .unwrap()
            .push(json!({"tag":"node-a-shadowtls","type":"shadowtls"}));
        transport["outbounds"][5]["outbounds"]
            .as_array_mut()
            .unwrap()
            .push(json!("node-a-shadowtls"));
        let transport = route(&transport, true);
        assert!(transport.has_diagnostic());
        assert_eq!(
            transport
                .graph
                .leaf(&chains(&["node-a", "proxy"]))
                .as_deref(),
            Some("node-a")
        );
        assert!(transport
            .graph
            .leaf(&chains(&["node-a-shadowtls"]))
            .is_none());
        assert!(!transport
            .graph
            .diag
            .as_ref()
            .unwrap()
            .members
            .contains("node-a-shadowtls"));
    }

    #[test]
    fn public_futures_and_contexts_satisfy_send_contract() {
        fn assert_send<T: Send>(_: T) {}
        fn assert_shared<T: Send + Sync>() {}
        fn response_contract(response: &reqwest::Response) {
            assert_send(observe(response, "https://example.com/"));
        }
        assert_shared::<RouteContext>();
        assert_shared::<RequestContext>();
        let context = RequestContext::new(
            RouteContext::new(None, "ci-send".into()),
            "https://example.com/",
        );
        assert_send(context.finish());
        assert_send(scope(context.clone(), std::future::ready(())));
        assert_send(RouteContext::from_active_config(
            None,
            "ci-send".into(),
            None,
            false,
        ));
        let _ = response_contract as fn(&reqwest::Response);
    }

    #[test]
    fn diagnostic_requires_generated_marker_runtime_auth_and_dynamic_loopback_port() {
        for mihomo in [false, true] {
            let fixture = fixture(mihomo, 27013);
            assert_eq!(
                route(&fixture, true).graph.diag.as_ref().unwrap().port,
                27013
            );
            assert!(route(&fixture, false).graph.diag.is_none());
            let key = if mihomo { "listeners" } else { "inbounds" };
            let index = if mihomo { 0 } else { 1 };
            for (field, bad) in [("listen", json!("0.0.0.0")), ("users", json!([]))] {
                let mut bad_config = fixture.clone();
                bad_config[key][index][field] = bad;
                assert!(route(&bad_config, true).graph.diag.is_none());
            }
            let mut bad_auth = fixture.clone();
            bad_auth[key][index]["users"][0]["password"] = json!("different-test-session");
            assert!(route(&bad_auth, true).graph.diag.is_none());
            let wrong_api = RouteContext::from_config(
                Some(ClashApi::new("127.0.0.1", 1, "other-test-session")),
                "ci".into(),
                &fixture.to_string(),
                true,
            );
            assert!(!wrong_api.has_diagnostic());
        }
        let oversized = vec![b' '; MAX_CONFIG_BYTES + 1];
        assert!(ConfigGraph::parse(&oversized, true, Some("ci-fixture")).is_none());
        let mut duplicate = fixture(false, 27013);
        duplicate["outbounds"]
            .as_array_mut()
            .unwrap()
            .push(json!({"tag":"node-a","type":"socks"}));
        assert!(
            ConfigGraph::parse(duplicate.to_string().as_bytes(), true, Some("ci-fixture"))
                .is_none()
        );
    }

    #[tokio::test]
    async fn url_roles_service_ip_cap_reset_and_scope_are_isolated() {
        let context = RequestContext::new(
            RouteContext::new(None, "ci".into()),
            "https://example.com/primary",
        );
        scope(context.clone(), async {
            let primary = https_url("https://example.com/primary").unwrap();
            let auxiliary = https_url("https://example.com/trace").unwrap();
            context.reserve(primary.clone()).unwrap();
            context.reserve(auxiliary.clone()).unwrap();
            record_service_ip(auxiliary.as_str(), "8.8.4.4");
            record_service_ip(primary.as_str(), "127.0.0.1");
            let records = context.records();
            assert_eq!(records[0].role, "primary");
            assert_eq!(records[1].role, "auxiliary");
            assert_eq!(records[0].match_status, "unsupported");
            assert!(records[0].exit_ip.is_none());
            assert_eq!(records[1].exit_source, Some("service_response"));
            let serialized = serde_json::to_value(&records[1]).unwrap();
            assert_eq!(serialized["exitCached"], false);
            assert_eq!(serialized["matchStatus"], "unsupported");
            assert!(serialized.get("exit_ip").is_none());
            for _ in 2..MAX_RECORDS {
                context.reserve(primary.clone()).unwrap();
            }
            assert!(context.reserve(primary).is_none());
            let old = context.attempt.lock().unwrap().generation;
            context.reset_attempt();
            assert!(!context.current_generation(old));
            record_service_ip(auxiliary.as_str(), "8.8.8.8");
            assert!(context.records().is_empty());
        })
        .await;
        // 不在 scope 内的调用不写入之前的上下文。
        record_service_ip("https://example.com/trace", "8.8.8.8");
        assert!(context.records().is_empty());
    }

    #[test]
    fn ip_validation_and_cache_never_use_target_or_cross_session_family() {
        for bad in [
            "",
            "target.example",
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "::1",
            "fe80::1",
            "2001:db8::1",
        ] {
            assert!(public_ip(bad).is_none());
        }
        assert!(public_ip("8.8.8.8").is_some());
        assert!(public_ip("2606:4700:4700::1111").is_some());
        assert!(!Family::V4.accepts("2606:4700:4700::1111".parse().unwrap()));
        assert!(matches!(Family::from_target("198.18.0.1"), Family::Any));
        let key = CacheKey {
            session: "isolated-cache-test".into(),
            config: "config-a".into(),
            tag: "node-a".into(),
            family: Family::V4,
        };
        cache_exit(key.clone(), "8.8.8.8".into());
        assert_eq!(cached_exit(&key).as_deref(), Some("8.8.8.8"));
        for changed in [
            CacheKey {
                session: "another-session".into(),
                ..key.clone()
            },
            CacheKey {
                config: "config-b".into(),
                ..key.clone()
            },
            CacheKey {
                family: Family::V6,
                ..key.clone()
            },
            CacheKey {
                tag: "node-b".into(),
                ..key.clone()
            },
        ] {
            assert!(cached_exit(&changed).is_none());
        }
        cache().lock().unwrap().get_mut(&key).unwrap().at = Instant::now() - CACHE_TTL;
        assert!(cached_exit(&key).is_none());
        for index in 0..(CACHE_LIMIT + 2) {
            cache_exit(
                CacheKey {
                    tag: format!("cache-node-{index}"),
                    ..key.clone()
                },
                "8.8.8.8".into(),
            );
        }
        assert!(cache().lock().unwrap().len() <= CACHE_LIMIT);
        cache()
            .lock()
            .unwrap()
            .retain(|key, _| key.session != "isolated-cache-test");
    }

    // 下面仅在 CI=true 时绑定临时 loopback stub；不启动代理内核，不访问公网。
    // 每个 stub 有读写超时，Drop 停止并 join 自己的线程，无临时文件。
    struct Stub {
        addr: SocketAddr,
        stop: Arc<std::sync::atomic::AtomicBool>,
        requests: Arc<Mutex<Vec<String>>>,
        peers: Arc<Mutex<Vec<SocketAddr>>>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl Stub {
        fn start(respond: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
            use std::io::Write;
            use std::sync::atomic::{AtomicBool, Ordering};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let addr = listener.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let peers = Arc::new(Mutex::new(Vec::new()));
            let (worker_stop, worker_requests, worker_peers) =
                (stop.clone(), requests.clone(), peers.clone());
            let worker = std::thread::spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    let (mut socket, peer) = match listener.accept() {
                        Ok(accepted) => accepted,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(_) => break,
                    };
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_millis(400)))
                        .unwrap();
                    socket
                        .set_write_timeout(Some(Duration::from_millis(400)))
                        .unwrap();
                    worker_peers.lock().unwrap().push(peer);
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 2048];
                    loop {
                        let Ok(read) = socket.read(&mut buffer) else {
                            break;
                        };
                        if read == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..read]);
                        if request.len() > 65536 {
                            break;
                        }
                        if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&request[..end]);
                            let length = headers
                                .lines()
                                .find_map(|line| {
                                    let (key, value) = line.split_once(':')?;
                                    key.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .unwrap_or(0);
                            if request.len() >= end + 4 + length {
                                break;
                            }
                        }
                    }
                    let request = String::from_utf8_lossy(&request).into_owned();
                    worker_requests.lock().unwrap().push(request.clone());
                    let body = respond(&request);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes());
                    let _ = socket.shutdown(std::net::Shutdown::Both);
                }
            });
            Self {
                addr,
                stop,
                requests,
                peers,
                worker: Some(worker),
            }
        }

        fn api(&self) -> ClashApi {
            ClashApi::new("127.0.0.1", self.addr.port(), "ci-fixture")
        }
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }

    fn ci() -> bool {
        std::env::var("CI").ok().as_deref() == Some("true")
    }

    #[tokio::test]
    async fn ci_api_association_uses_exact_tuple_and_rejects_reset_and_duplicates() {
        if !ci() {
            return;
        }
        let body = Arc::new(Mutex::new(String::new()));
        let response_body = body.clone();
        let stub = Stub::start(move |_| response_body.lock().unwrap().clone());
        let rows = snapshot(&[(50001, "example.com", "tcp"), (50002, "example.com", "tcp")]);
        let wire = |duplicate: bool| {
            let row = json!({"id":"exact", "metadata":{"sourceIP":"127.0.0.1","sourcePort":"50001",
                "destinationPort":443,"host":"example.com","network":"tcp","destinationIP":"8.8.8.8"},
                "chains":["Satelite-DIRECT","proxy"],"rule":"DomainSuffix","rulePayload":"example.com"});
            json!({"connections":if duplicate {vec![row.clone(),row]} else {vec![row]}}).to_string()
        };
        *body.lock().unwrap() = wire(false);
        let context = RequestContext::new(
            RouteContext::from_config(
                Some(stub.api()),
                "ci-api".into(),
                &fixture(true, 27013).to_string(),
                false,
            ),
            "https://example.com/a",
        );
        let target = https_url("https://example.com/a").unwrap();
        let (generation, index) = context.reserve(target.clone()).unwrap();
        observe_identity(
            &context,
            generation,
            index,
            "127.0.0.1:50001".parse().unwrap(),
            &target,
        )
        .await;
        let record = &context.records()[0];
        assert_eq!(record.match_status, "matched");
        assert_eq!(record.outbound_tag.as_deref(), Some("Satelite-DIRECT"));
        assert!(record.exit_ip.is_none());
        assert!(exact_connection(&rows, "127.0.0.1:50001".parse().unwrap(), &target).is_some());
        context.reset_attempt();
        let (_, next_index) = context.reserve(target.clone()).unwrap();
        observe_identity(
            &context,
            generation,
            next_index,
            "127.0.0.1:50001".parse().unwrap(),
            &target,
        )
        .await;
        assert_eq!(context.records()[0].match_status, "not_observed");
        *body.lock().unwrap() = wire(true);
        let generation = context.attempt.lock().unwrap().generation;
        observe_identity(
            &context,
            generation,
            next_index,
            "127.0.0.1:50001".parse().unwrap(),
            &target,
        )
        .await;
        assert_eq!(context.records()[0].match_status, "not_observed");
        assert!(stub
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.starts_with("GET /connections ")));
    }

    #[tokio::test]
    async fn ci_httpinfo_and_bounded_exit_body_use_only_loopback_stub() {
        if !ci() {
            return;
        }
        let stub = Stub::start(|request| {
            if request.starts_with("GET /large ") {
                "x".repeat(16385)
            } else if request.starts_with("GET /private ") {
                "127.0.0.1".into()
            } else {
                "8.8.4.4\n".into()
            }
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let response = client
            .get(format!("http://{}/ip", stub.addr))
            .send()
            .await
            .unwrap();
        let info = response.extensions().get::<HttpInfo>().unwrap();
        assert_eq!(info.remote_addr(), stub.addr);
        assert_eq!(info.local_addr(), stub.peers.lock().unwrap()[0]);
        assert_eq!(
            read_exit_response(response, Family::V4).await.as_deref(),
            Some("8.8.4.4")
        );
        for path in ["large", "private"] {
            let response = client
                .get(format!("http://{}/{path}", stub.addr))
                .send()
                .await
                .unwrap();
            assert!(read_exit_response(response, Family::V4).await.is_none());
        }
        drop(stub);
    }

    #[tokio::test]
    async fn ci_cancelled_waiter_keeps_selector_permit_through_blocking_and_request_work() {
        if !ci() {
            return;
        }
        let stub = Stub::start(|request| {
            if request.starts_with("PUT ") {
                std::thread::sleep(Duration::from_millis(120));
                "{}".into()
            } else {
                json!({"now":"node-a"}).to_string()
            }
        });
        let gate = Arc::new(Semaphore::new(1));
        let expired = gate.clone().acquire_owned().await.unwrap();
        assert!(
            select_diagnostic(stub.api(), "node-a".into(), expired, Instant::now())
                .await
                .is_none()
        );
        assert!(stub.requests.lock().unwrap().is_empty());
        let permit = gate.clone().acquire_owned().await.unwrap();
        let api = stub.api();
        let (done, completed) = tokio::sync::oneshot::channel();
        let job = tokio::spawn(async move {
            let selected =
                select_diagnostic(api, "node-a".into(), permit, Instant::now() + EXIT_TIMEOUT)
                    .await;
            let (permit, _) = selected.expect("仅私有 selector 可切换");
            let _permit = permit;
            tokio::time::sleep(Duration::from_millis(120)).await;
            let _ = done.send(());
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while stub.requests.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(10), job)
            .await
            .is_err());
        assert!(gate.clone().try_acquire_owned().is_err());
        tokio::time::timeout(Duration::from_secs(2), completed)
            .await
            .unwrap()
            .unwrap();
        let permit = tokio::time::timeout(Duration::from_secs(1), gate.clone().acquire_owned())
            .await
            .unwrap()
            .unwrap();
        drop(permit);
        let requests = stub.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("PUT /proxies/service-diag "));
        assert!(requests[0].contains("\"name\":\"node-a\""));
        assert!(requests[1].starts_with("GET /proxies/service-diag "));
    }
}

#[cfg(test)]
#[path = "service_route_live_tests.rs"]
mod live_tests;
