//! Isolated Xray/compatible-core HTTP probes. No main runtime, system proxy, TUN,
//! active config, geodata, or selector mutations. Config is sent via stdin.
use crate::config::{
    apply_udp_node_compatibility, build_singbox_probe_config, build_xray_sidecar_config,
};
use crate::core::CoreKind;
use crate::domain::ProxyNode;
use crate::services::latency::LatencyResult;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Semaphore;

static BATCH: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));
const CONCURRENCY: usize = 4;

fn result(node: &ProxyNode, method: &str, value: Result<u32, String>) -> LatencyResult {
    let (latency_ms, error) = match value {
        Ok(ms) => (Some(ms), None),
        Err(error) => (None, Some(error)),
    };
    let method = if error
        .as_deref()
        .is_some_and(|e| e.starts_with("unsupported:"))
    {
        "unsupported"
    } else {
        method
    };
    LatencyResult {
        id: node.id.clone(),
        name: node.name.clone(),
        latency_ms,
        error,
        tested_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
        method: method.into(),
    }
}

#[derive(Clone, Default)]
pub struct ProbeBinaries {
    pub xray: Option<PathBuf>,
    pub singbox: Option<PathBuf>,
}

fn probe_kind(node: &ProxyNode) -> CoreKind {
    // Xray cannot preserve skip-cert-verify. Respect the imported/explicit
    // compatibility setting instead of emitting a different TLS policy.
    let needs_insecure = node.tls.as_ref().is_some_and(|tls| {
        tls.enabled && tls.insecure == Some(true) && tls.reality_public_key.is_none()
    });
    if CoreKind::Xray.supports_node(node) && !needs_insecure {
        CoreKind::Xray
    } else {
        CoreKind::SingBox
    }
}

fn method_for(kind: CoreKind) -> &'static str {
    match kind {
        CoreKind::SingBox => "singbox_http",
        _ => "xray_http",
    }
}

fn validated_url(raw: &str) -> Result<String, String> {
    let raw = if raw.trim().is_empty() {
        "https://www.gstatic.com/generate_204"
    } else {
        raw.trim()
    };
    let url = url::Url::parse(raw).map_err(|_| "测速网址无效")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("测速网址必须是无内嵌凭据的 HTTP/HTTPS 地址".into());
    }
    Ok(url.to_string())
}

fn probe_config(node: &ProxyNode, port: u16, token: &str, fragment: bool) -> Result<Value, String> {
    if !CoreKind::Xray.supports_node(node) {
        return Err("unsupported: 该节点参数不受 Xray 支持".into());
    }
    let mut config = build_xray_sidecar_config(&[(node.clone(), port)], fragment)
        .map_err(|_| "config: 无法生成 Xray 测速配置，请检查节点参数")?
        .value;
    // Auth protects the port-allocation/bind race: a foreign listener must
    // not silently answer the request as if it were the node under test.
    config["inbounds"][0]["protocol"] = json!("http");
    config["inbounds"][0]["settings"] =
        json!({"accounts":[{"user":"satelite-probe","pass":token}]});
    config["inbounds"][0]
        .as_object_mut()
        .unwrap()
        .remove("sniffing");
    config["log"] = json!({"loglevel":"warning"});
    Ok(config)
}

fn config_for(
    kind: CoreKind,
    node: &ProxyNode,
    port: u16,
    token: &str,
    fragment: bool,
) -> Result<Value, String> {
    match kind {
        CoreKind::Xray => probe_config(node, port, token, fragment),
        CoreKind::SingBox => {
            if !kind.supports_node(node)
                || matches!(
                    node.protocol,
                    crate::domain::Protocol::Sudoku
                        | crate::domain::Protocol::Masque
                        | crate::domain::Protocol::WireGuard
                )
                || matches!(node.transport, Some(crate::domain::Transport::Xhttp { .. }))
            {
                return Err("unsupported: 当前独立测速内核无法完整处理此节点参数".into());
            }
            build_singbox_probe_config(node, port, token, fragment)
                .map_err(|_| "config: 无法生成 sing-box 测速配置，请检查节点参数".into())
        }
        _ => Err("unsupported: 当前独立测速内核不可用".into()),
    }
}

#[derive(Default)]
struct Diagnostics {
    tls: bool,
    config: bool,
    timeout: bool,
}
impl Diagnostics {
    fn observe(&mut self, text: &str) {
        let text = text.to_ascii_lowercase();
        self.tls |= [
            "x509:",
            "certificate",
            "tls handshake",
            "tls: failed",
            "crypto_error",
        ]
        .iter()
        .any(|p| text.contains(p));
        self.config |= [
            "failed to parse",
            "failed to load config",
            "unknown field",
            "unknown outbound",
            "decode config",
            "invalid config",
        ]
        .iter()
        .any(|p| text.contains(p));
        self.timeout |= [
            "timeout",
            "timed out",
            "deadline exceeded",
            "no recent network activity",
        ]
        .iter()
        .any(|p| text.contains(p));
    }
    fn error(&self) -> Option<String> {
        if self.tls {
            Some("tls: 节点 TLS/证书握手失败，请检查 SNI、证书及节点兼容设置".into())
        } else if self.config {
            Some("config: 测速内核拒绝了节点配置，请检查协议参数与内核版本".into())
        } else if self.timeout {
            Some("timeout: 经节点的请求超时".into())
        } else {
            None
        }
    }
}

// Drain fixed-size chunks; retain only classification flags, never raw logs
// (which may contain server addresses or credentials). Overlap handles split lines.
fn collect_diagnostics(
    mut pipe: impl Read + Send + 'static,
    flags: Arc<Mutex<Diagnostics>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut bytes = [0u8; 2048];
        let mut tail = String::new();
        while let Ok(n) = pipe.read(&mut bytes) {
            if n == 0 {
                break;
            }
            let text = format!("{tail}{}", String::from_utf8_lossy(&bytes[..n]));
            if let Ok(mut flags) = flags.lock() {
                flags.observe(&text);
            }
            tail = text
                .chars()
                .rev()
                .take(128)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
        }
    })
}

struct ProbeChild {
    child: Child,
    feeder: Option<std::thread::JoinHandle<()>>,
    readers: Vec<std::thread::JoinHandle<()>>,
    diagnostics: Arc<Mutex<Diagnostics>>,
}
impl ProbeChild {
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(feeder) = self.feeder.take() {
            let _ = feeder.join();
        }
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
    fn diagnostic_error(&self) -> Option<String> {
        self.diagnostics.lock().ok().and_then(|d| d.error())
    }
}
impl Drop for ProbeChild {
    fn drop(&mut self) {
        // Only kill the child created here; never reclaim a foreign port.
        self.stop();
    }
}

#[cfg(test)]
fn spawn_probe(bin: &Path, config: Value) -> Result<ProbeChild, String> {
    spawn_core_probe(CoreKind::Xray, bin, config)
}

fn spawn_core_probe(kind: CoreKind, bin: &Path, config: Value) -> Result<ProbeChild, String> {
    let bytes = serde_json::to_vec(&config).map_err(|_| "测速配置序列化失败")?;
    if bytes.len() > 128 * 1024 {
        return Err("单节点测速配置超过大小限制".into());
    }
    let mut command = Command::new(bin);
    command
        .args([
            "run",
            "-c",
            if kind == CoreKind::SingBox {
                "stdin"
            } else {
                "stdin:"
            },
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let child = command
        .spawn()
        .map_err(|_| "core: 无法启动测速进程，请检查内核安装与安全软件拦截情况")?;
    let mut owned = ProbeChild {
        child,
        feeder: None,
        readers: Vec::new(),
        diagnostics: Arc::new(Mutex::new(Diagnostics::default())),
    };
    #[cfg(target_os = "windows")]
    crate::core::job::ensure_child_killed_on_parent_exit(owned.child.id())
        .map_err(|_| "core: 无法绑定测速进程生命周期，已停止测速")?;
    if let Some(stderr) = owned.child.stderr.take() {
        owned
            .readers
            .push(collect_diagnostics(stderr, owned.diagnostics.clone()));
    }
    if let Some(stdout) = owned.child.stdout.take() {
        owned
            .readers
            .push(collect_diagnostics(stdout, owned.diagnostics.clone()));
    }
    let mut stdin = owned.child.stdin.take().ok_or("测速进程输入不可用")?;
    owned.feeder = Some(std::thread::spawn(move || {
        let _ = stdin.write_all(&bytes);
    }));
    Ok(owned)
}

#[cfg(test)]
fn probe_one(
    bin: &Path,
    node: &ProxyNode,
    url: &str,
    timeout_ms: u64,
    fragment: bool,
) -> Result<u32, String> {
    probe_one_with_core(CoreKind::Xray, bin, node, url, timeout_ms, fragment)
}

fn probe_one_with_core(
    kind: CoreKind,
    bin: &Path,
    node: &ProxyNode,
    url: &str,
    timeout_ms: u64,
    fragment: bool,
) -> Result<u32, String> {
    let reservation = TcpListener::bind("127.0.0.1:0").map_err(|_| "无法分配本机测速端口")?;
    let addr = reservation.local_addr().map_err(|_| "无法读取测速端口")?;
    let token = crate::config::generate_api_secret();
    let config = config_for(kind, node, addr.port(), &token, fragment)?;
    drop(reservation);
    let mut process = spawn_core_probe(kind, bin, config)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if process
            .child
            .try_wait()
            .map_err(|_| "无法检查测速进程")?
            .is_some()
        {
            process.stop();
            return Err(process
                .diagnostic_error()
                .unwrap_or_else(|| "core: 测速进程退出，请检查节点配置与内核兼容性".into()));
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(80)).is_ok() {
            break;
        }
        if Instant::now() >= deadline {
            return Err("core: 本机测速入口启动超时".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    // ureq 2.12 sends lowercase `basic` in CONNECT authentication; Xray
    // v26.3.27 accepts only `Basic`. reqwest emits the compatible scheme.
    // This function runs only on blocking workers (or synchronous CI tests),
    // never on a Tokio async worker. The private runtime is dropped here too.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "无法建立测速请求运行时")?;
    let tested = runtime.block_on(async {
        let proxy = reqwest::Proxy::all(format!("http://{addr}"))
            .map_err(|_| "无法建立本机测速代理")?
            .basic_auth("satelite-probe", &token);
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(proxy)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(timeout_ms))
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|_| "无法建立测速 HTTP 客户端")?;
        let request = || async {
            let started = Instant::now();
            match client
                .get(url)
                .header(reqwest::header::CONNECTION, "close")
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    Ok(started.elapsed().as_millis().min(u32::MAX as u128) as u32)
                }
                Ok(response) => Err(format!(
                    "http: 测速网址返回 HTTP {}",
                    response.status().as_u16()
                )),
                Err(error) if error.is_timeout() => Err("timeout: 经该节点的 HTTP 请求超时".into()),
                Err(error) => {
                    let mut flags = Diagnostics::default();
                    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
                    while let Some(current) = source {
                        flags.observe(&current.to_string());
                        source = current.source();
                    }
                    Err(flags
                        .error()
                        .unwrap_or_else(|| "connection: 经该节点的连接或握手失败".into()))
                }
            }
        };
        let first = request().await;
        let second = request().await;
        match (first, second) {
            (_, Ok(ms)) | (Ok(ms), _) => Ok(ms),
            (_, Err(error)) => Err(error),
        }
    });
    // Check liveness after the request as well, avoiding a false success
    // from another service that won the bind race while Xray was exiting.
    let exited = process
        .child
        .try_wait()
        .map_err(|_| "无法检查测速进程")?
        .is_some();
    process.stop();
    if exited {
        return Err(process
            .diagnostic_error()
            .unwrap_or_else(|| "core: 测速入口不可用，可能存在端口冲突".into()));
    }
    tested.map_err(|error| process.diagnostic_error().unwrap_or(error))
}

/// One batch globally, four bounded blocking workers. The permit is shared
/// with workers so dropping the caller cannot start unbounded orphan work.
#[cfg(test)]
pub async fn probe_xray_nodes(
    nodes: Vec<ProxyNode>,
    bin: PathBuf,
    url: String,
    timeout_ms: Option<u64>,
    fragment: bool,
    on_result: impl Fn(&LatencyResult) + Send,
) -> Result<Vec<LatencyResult>, String> {
    probe_xray_nodes_with_fallback(
        nodes,
        ProbeBinaries {
            xray: Some(bin),
            singbox: None,
        },
        url,
        timeout_ms,
        fragment,
        false,
        false,
        on_result,
    )
    .await
}

pub async fn probe_xray_nodes_with_fallback(
    mut nodes: Vec<ProxyNode>,
    binaries: ProbeBinaries,
    url: String,
    timeout_ms: Option<u64>,
    fragment_xray: bool,
    fragment_singbox: bool,
    udp_tls_compat: bool,
    on_result: impl Fn(&LatencyResult) + Send,
) -> Result<Vec<LatencyResult>, String> {
    apply_udp_node_compatibility(&mut nodes, udp_tls_compat);
    let url = validated_url(&url)?;
    let timeout_ms = timeout_ms.unwrap_or(5000).clamp(500, 15000);
    let batch = Arc::new(
        BATCH
            .clone()
            .try_acquire_owned()
            .map_err(|_| "Xray 测速正在进行，请等待本轮完成")?,
    );
    let mut pending = nodes.into_iter().enumerate();
    let mut tasks = tokio::task::JoinSet::new();
    let spawn = |tasks: &mut tokio::task::JoinSet<_>, index, node: ProxyNode| {
        let binaries = binaries.clone();
        let url = url.clone();
        let batch = batch.clone();
        tasks.spawn_blocking(move || {
            let _batch = batch;
            let kind = probe_kind(&node);
            let bin = match kind {
                CoreKind::Xray => binaries.xray,
                _ => binaries.singbox,
            };
            let fragment = if kind == CoreKind::Xray {
                fragment_xray
            } else {
                fragment_singbox
            };
            // Validate unsupported shapes before reporting a missing binary.
            let preflight = config_for(kind, &node, 1, "preflight-only", fragment);
            let tested = match (preflight, bin) {
                (Err(error), _) => Err(error),
                (Ok(_), Some(bin)) => {
                    probe_one_with_core(kind, &bin, &node, &url, timeout_ms, fragment)
                }
                (Ok(_), None) => Err(if kind == CoreKind::SingBox {
                    "missing_core: 此节点需要 sing-box 独立测速，请先在设置中安装 sing-box".into()
                } else {
                    "missing_core: 请先在设置中安装 Xray 内核".into()
                }),
            };
            (index, result(&node, method_for(kind), tested))
        });
    };
    for _ in 0..CONCURRENCY {
        if let Some((index, node)) = pending.next() {
            spawn(&mut tasks, index, node);
        }
    }
    let mut results = Vec::new();
    while let Some(next) = tasks.join_next().await {
        let (index, result) = next.map_err(|_| "Xray 测速任务异常终止")?;
        on_result(&result);
        results.push((index, result));
        if let Some((index, node)) = pending.next() {
            spawn(&mut tasks, index, node);
        }
    }
    results.sort_by_key(|(index, _)| *index);
    Ok(results.into_iter().map(|(_, result)| result).collect())
}

#[cfg(test)]
#[path = "xray_latency_live_tests.rs"]
mod live_protocol_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn node(port: u16) -> ProxyNode {
        crate::subscription::parse_subscription(&format!("socks5://127.0.0.1:{port}#fixture"))
            .unwrap()
            .nodes
            .remove(0)
    }

    fn protocol_node(kind: &str) -> ProxyNode {
        crate::subscription::parse_subscription(&format!(
            "proxies:\n  - name: fixture\n    type: {kind}\n    server: localhost\n    port: 443\n    password: fixture-password\n    uuid: 11111111-1111-1111-1111-111111111111\n    skip-cert-verify: false\n"
        )).unwrap().nodes.remove(0)
    }

    #[test]
    fn compatible_probes_keep_udp_toggle_and_tls_semantics() {
        for protocol in ["hysteria2", "tuic", "anytls"] {
            let original = protocol_node(protocol);
            let mut nodes = vec![original.clone()];
            apply_udp_node_compatibility(&mut nodes, false);
            let node = &nodes[0];
            assert_ne!(node.tls.as_ref().unwrap().insecure, Some(true));
            let expected = if protocol == "hysteria2" {
                CoreKind::Xray
            } else {
                CoreKind::SingBox
            };
            assert_eq!(probe_kind(node), expected);
            let doc = config_for(CoreKind::SingBox, node, 30002, "test-token", false).unwrap();
            assert_eq!(doc["inbounds"][0]["listen"], "127.0.0.1");
            assert_eq!(doc["inbounds"][0]["users"][0]["password"], "test-token");
            assert_eq!(doc["outbounds"].as_array().unwrap().len(), 1);
            assert_eq!(doc["outbounds"][0]["type"], protocol);
            assert_ne!(doc["outbounds"][0]["tls"]["insecure"], true);
            assert!(doc.get("experimental").is_none());
            assert!(doc["route"].get("rules").is_none());
            if protocol == "tuic" {
                assert_eq!(doc["outbounds"][0]["tls"]["alpn"], json!(["h3"]));
            }
            apply_udp_node_compatibility(&mut nodes, true);
            let node = &nodes[0];
            if protocol != "anytls" {
                assert_eq!(probe_kind(node), CoreKind::SingBox);
                let doc = config_for(probe_kind(node), node, 30002, "test-token", false).unwrap();
                assert_eq!(doc["outbounds"][0]["tls"]["insecure"], true);
            } else {
                assert_ne!(
                    node.tls.as_ref().unwrap().insecure,
                    Some(true),
                    "UDP toggle must not weaken AnyTLS"
                );
            }
            assert_ne!(
                original.tls.as_ref().unwrap().insecure,
                Some(true),
                "probe must not mutate stored nodes"
            );
        }
        let mut hy2 = protocol_node("hysteria2");
        if let crate::domain::ProtocolConfig::Hysteria2 {
            obfs,
            obfs_password,
            ..
        } = &mut hy2.config
        {
            *obfs = Some("salamander".into());
            *obfs_password = Some("fixture-obfs".into());
        }
        assert_eq!(probe_kind(&hy2), CoreKind::SingBox);
        let doc = config_for(probe_kind(&hy2), &hy2, 30002, "token", false).unwrap();
        assert_eq!(doc["outbounds"][0]["obfs"]["password"], "fixture-obfs");
    }

    #[test]
    fn diagnostics_are_classified_without_leaking_raw_content() {
        for (raw, prefix) in [
            (
                "x509: certificate signed by unknown authority password=SECRET",
                "tls:",
            ),
            ("decode config: unknown field SECRET", "config:"),
            ("context deadline exceeded SECRET", "timeout:"),
        ] {
            let mut flags = Diagnostics::default();
            flags.observe(raw);
            let error = flags.error().unwrap();
            assert!(error.starts_with(prefix));
            assert!(!error.contains("SECRET"));
        }
        let unsupported = result(
            &node(30001),
            "xray_http",
            Err("unsupported: unsupported shape".into()),
        );
        assert_eq!(unsupported.method, "unsupported");
        assert_eq!(
            result(&node(30001), "singbox_http", Ok(123)).method,
            "singbox_http"
        );
    }
    #[test]
    fn config_is_isolated_authenticated_and_has_no_direct_fallback() {
        let node = node(30001);
        let doc = probe_config(&node, 30002, "test-only-token", false).unwrap();
        assert_eq!(doc["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(doc["inbounds"][0]["protocol"], "http");
        assert_eq!(
            doc["inbounds"][0]["settings"]["accounts"][0]["pass"],
            "test-only-token"
        );
        assert_eq!(doc["outbounds"].as_array().unwrap().len(), 1);
        assert_eq!(doc["outbounds"][0]["protocol"], "socks");
        for key in ["dns", "api", "metrics", "observatory"] {
            assert!(doc.get(key).is_none());
        }
        assert!(validated_url("file:///secret").is_err());
        assert!(validated_url("https://user:pass@example.com").is_err());
    }

    #[tokio::test]
    async fn missing_binary_streams_failures_without_tcp_fallback() {
        let mut second = node(30004);
        second.id = "another-test-node".into();
        let nodes = vec![node(30003), second];
        let streamed = std::sync::Mutex::new(Vec::new());
        let tested = probe_xray_nodes(
            nodes.clone(),
            PathBuf::from("missing-satelite-test-core"),
            "https://example.test/204".into(),
            Some(500),
            false,
            |result| streamed.lock().unwrap().push(result.id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(
            tested.iter().map(|r| &r.id).collect::<Vec<_>>(),
            nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
        );
        assert_eq!(streamed.lock().unwrap().len(), 2);
        assert!(tested
            .iter()
            .all(|r| r.latency_ms.is_none() && r.method == "xray_http"));

        let tested = probe_xray_nodes_with_fallback(
            vec![
                protocol_node("tuic"),
                protocol_node("anytls"),
                protocol_node("hysteria2"),
            ],
            ProbeBinaries::default(),
            "https://example.test/204".into(),
            Some(500),
            false,
            false,
            true,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(tested.len(), 3);
        assert!(tested.iter().all(|r| r.method == "singbox_http"
            && r.error.as_deref().unwrap().starts_with("missing_core:")));
    }

    // Fake upstream accepts SOCKS5 and answers the tunneled HTTP request.
    // The destination is .invalid, so a direct/system-proxy bypass cannot pass.
    fn socks_fixture(
        status: u16,
        tls: bool,
    ) -> (ProxyNode, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let node = node(listener.local_addr().unwrap().port());
        let seen = Arc::new(AtomicUsize::new(0));
        let requests = seen.clone();
        let thread = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(12);
            while Instant::now() < deadline && requests.load(Ordering::SeqCst) < 2 {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                };
                // Winsock can inherit the listener's nonblocking mode. Only
                // accept is polled; a connected fixture stream uses timeouts.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut head = [0; 2];
                stream.read_exact(&mut head).unwrap();
                assert_eq!(head[0], 5);
                let mut methods = vec![0; head[1] as usize];
                stream.read_exact(&mut methods).unwrap();
                stream.write_all(&[5, 0]).unwrap();
                let mut connect = [0; 4];
                stream.read_exact(&mut connect).unwrap();
                assert_eq!(connect[1], 1);
                let length = match connect[3] {
                    1 => 4,
                    4 => 16,
                    3 => {
                        let mut n = [0];
                        stream.read_exact(&mut n).unwrap();
                        n[0] as usize
                    }
                    _ => panic!("invalid SOCKS address"),
                };
                let mut destination = vec![0; length + 2];
                stream.read_exact(&mut destination).unwrap();
                assert!(String::from_utf8_lossy(&destination).contains("probe.invalid"));
                stream
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                    .unwrap();
                if tls {
                    // Record the HTTPS tunnel then close it. TLS must fail,
                    // not fall back to direct or disable certificate checks.
                    requests.fetch_add(1, Ordering::SeqCst);
                    continue;
                }
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                assert!(String::from_utf8_lossy(&request).contains("probe.invalid"));
                requests.fetch_add(1, Ordering::SeqCst);
                stream.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            }
        });
        (node, seen, thread)
    }

    #[test]
    #[ignore] // Real binary only on the CI runner; never the user's live core.
    fn live_xray_http_probe_uses_node_and_cleans_up() {
        let bin = crate::core::find_bundled_core(None, CoreKind::Xray).expect("bundled Xray");
        for status in [204, 503] {
            let (node, seen, upstream) = socks_fixture(status, false);
            let tested = probe_one(
                &bin,
                &node,
                "http://probe.invalid/generate_204",
                1500,
                false,
            );
            upstream.join().unwrap();
            assert_eq!(
                seen.load(Ordering::SeqCst),
                2,
                "both probes must traverse the node; result: {tested:?}"
            );
            assert_eq!(tested.is_ok(), status == 204, "{tested:?}");
        }
        let (https_node, seen, upstream) = socks_fixture(204, true);
        let tested = probe_one(
            &bin,
            &https_node,
            "https://probe.invalid/generate_204",
            1500,
            false,
        );
        upstream.join().unwrap();
        assert_eq!(
            seen.load(Ordering::SeqCst),
            2,
            "HTTPS must traverse the node: {tested:?}"
        );
        assert!(tested.is_err(), "a broken TLS tunnel cannot be a success");
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = reservation.local_addr().unwrap();
        drop(reservation);
        let child = spawn_probe(
            &bin,
            probe_config(&node(30001), addr.port(), "test-only", false).unwrap(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while TcpStream::connect_timeout(&addr, Duration::from_millis(50)).is_err() {
            assert!(Instant::now() < deadline, "probe listener not ready");
            std::thread::sleep(Duration::from_millis(25));
        }
        drop(child);
        let rebound = TcpListener::bind(addr).expect("probe listener must be released after drop");
        drop(rebound);
    }
}
