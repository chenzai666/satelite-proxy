//! Isolated Xray HTTP latency probes. No main runtime, system proxy, TUN,
//! active config, geodata, or selector mutations. Config is sent via stdin.
use crate::config::build_xray_sidecar_config;
use crate::core::CoreKind;
use crate::domain::ProxyNode;
use crate::services::latency::LatencyResult;
use serde_json::{json, Value};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Semaphore;

static BATCH: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));
const CONCURRENCY: usize = 4;

fn result(node: &ProxyNode, value: Result<u32, String>) -> LatencyResult {
    let (latency_ms, error) = match value {
        Ok(ms) => (Some(ms), None),
        Err(error) => (None, Some(error)),
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
        method: "xray_http".into(),
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
        return Err("该节点不受 Xray 支持，请使用相应内核测速".into());
    }
    let mut config = build_xray_sidecar_config(&[(node.clone(), port)], fragment)
        .map_err(|_| "无法生成该节点的 Xray 测速配置，请检查协议参数")?
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
    config["log"] = json!({"loglevel":"none"});
    Ok(config)
}

struct ProbeChild {
    child: Child,
    feeder: Option<std::thread::JoinHandle<()>>,
}
impl Drop for ProbeChild {
    fn drop(&mut self) {
        // Only kill the child created here; never reclaim a foreign port.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(feeder) = self.feeder.take() {
            let _ = feeder.join();
        }
    }
}

fn spawn_probe(bin: &Path, config: Value) -> Result<ProbeChild, String> {
    let bytes = serde_json::to_vec(&config).map_err(|_| "测速配置序列化失败")?;
    if bytes.len() > 128 * 1024 {
        return Err("单节点测速配置超过大小限制".into());
    }
    let mut command = Command::new(bin);
    command
        .args(["run", "-c", "stdin:"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let child = command
        .spawn()
        .map_err(|_| "无法启动 Xray 测速进程，请检查内核是否安装")?;
    let mut owned = ProbeChild {
        child,
        feeder: None,
    };
    #[cfg(target_os = "windows")]
    crate::core::job::ensure_child_killed_on_parent_exit(owned.child.id())
        .map_err(|_| "无法绑定测速进程生命周期，已停止测速")?;
    let mut stdin = owned.child.stdin.take().ok_or("测速进程输入不可用")?;
    owned.feeder = Some(std::thread::spawn(move || {
        let _ = stdin.write_all(&bytes);
    }));
    Ok(owned)
}

fn probe_one(
    bin: &Path,
    node: &ProxyNode,
    url: &str,
    timeout_ms: u64,
    fragment: bool,
) -> Result<u32, String> {
    let reservation = TcpListener::bind("127.0.0.1:0").map_err(|_| "无法分配本机测速端口")?;
    let addr = reservation.local_addr().map_err(|_| "无法读取测速端口")?;
    let token = crate::config::generate_api_secret();
    let config = probe_config(node, addr.port(), &token, fragment)?;
    drop(reservation);
    let mut process = spawn_probe(bin, config)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if process
            .child
            .try_wait()
            .map_err(|_| "无法检查测速进程")?
            .is_some()
        {
            return Err("Xray 测速进程退出，请检查节点配置与内核兼容性".into());
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(80)).is_ok() {
            break;
        }
        if Instant::now() >= deadline {
            return Err("Xray 测速入口启动超时".into());
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
        let proxy = reqwest::Proxy::http(format!("http://{addr}"))
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
                _ => Err("经该节点的 HTTP 测速失败：超时、连接/证书错误或非 2xx 响应".to_string()),
            }
        };
        let first = request().await;
        let second = request().await;
        second.or(first)
    });
    // Check liveness after the request as well, avoiding a false success
    // from another service that won the bind race while Xray was exiting.
    if process
        .child
        .try_wait()
        .map_err(|_| "无法检查测速进程")?
        .is_some()
    {
        return Err("Xray 测速入口不可用，可能存在端口冲突".into());
    }
    tested
}

/// One batch globally, four bounded blocking workers. The permit is shared
/// with workers so dropping the caller cannot start unbounded orphan work.
pub async fn probe_xray_nodes(
    nodes: Vec<ProxyNode>,
    bin: PathBuf,
    url: String,
    timeout_ms: Option<u64>,
    fragment: bool,
    on_result: impl Fn(&LatencyResult) + Send,
) -> Result<Vec<LatencyResult>, String> {
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
        let bin = bin.clone();
        let url = url.clone();
        let batch = batch.clone();
        tasks.spawn_blocking(move || {
            let _batch = batch;
            let tested = probe_one(&bin, &node, &url, timeout_ms, fragment);
            (index, result(&node, tested))
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
    }

    // Fake upstream accepts SOCKS5 and answers the tunneled HTTP request.
    // The destination is .invalid, so a direct/system-proxy bypass cannot pass.
    fn socks_fixture(status: u16) -> (ProxyNode, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
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
                stream
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                    .unwrap();
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
            let (node, seen, upstream) = socks_fixture(status);
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
