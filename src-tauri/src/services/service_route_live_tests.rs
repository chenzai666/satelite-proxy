//! CI 专用真实 HTTPS CONNECT 回归；不启动代理内核，不查询公网 DNS。
//! 所有 Rust listener 使用 127.0.0.1:0；openssl 端口由相同方式预留后交接。
//! 执行时需同时存在 CI=true 与 GITHUB_ACTIONS=true，且显式 --ignored。

use super::*;
use serde_json::json;
use std::io::{self, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

const TARGET: &str = "https://service.test/";
const CI_SECRET: &str = "isolated-connect-fixture";

fn ci_guard() {
    assert_eq!(
        std::env::var("CI").as_deref(),
        Ok("true"),
        "CI_GUARD：禁止在本机运行 CONNECT 回归"
    );
    assert_eq!(
        std::env::var("GITHUB_ACTIONS").as_deref(),
        Ok("true"),
        "CI_GUARD：仅 GitHub Actions 运行"
    );
}

fn openssl_path() -> PathBuf {
    let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .map(|directory| {
                    directory.join(if cfg!(windows) {
                        "openssl.exe"
                    } else {
                        "openssl"
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    #[cfg(windows)]
    for variable in [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "LOCALAPPDATA",
    ] {
        if let Some(root) = std::env::var_os(variable) {
            let root = PathBuf::from(root);
            paths.push(root.join("Git/usr/bin/openssl.exe"));
            paths.push(root.join("Programs/Git/usr/bin/openssl.exe"));
        }
    }
    paths
        .into_iter()
        .find(|path| path.is_file())
        .expect("CI 需要 openssl（可使用 Git for Windows 内置版本）")
}

struct OwnedProcess {
    child: Option<Child>,
    reaped: bool,
    pid: u32,
    started_at: SystemTime,
    purpose: &'static str,
}

impl OwnedProcess {
    fn spawn(mut command: Command, purpose: &'static str) -> Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let started_at = SystemTime::now();
        let child = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("启动本测试专属 openssl 子进程");
        let pid = child.id();
        eprintln!("CI process start: purpose={purpose}, pid={pid}, started_at={started_at:?}");
        Self {
            child: Some(child),
            reaped: false,
            pid,
            started_at,
            purpose,
        }
    }

    fn wait_success(&mut self, limit: Duration) {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .expect("查询证书生成进程")
            {
                self.child.take();
                self.reaped = true;
                assert!(status.success(), "CI openssl 证书生成失败：{status}");
                return;
            }
            assert!(Instant::now() < deadline, "CI openssl 证书生成超时");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn exited(&mut self) -> Option<ExitStatus> {
        self.child.as_mut()?.try_wait().expect("查询 TLS 服务进程")
    }

    fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        // 先尝试关闭 openssl 交互输入；-www 不处理退出命令时，定向 kill 自己的 child。
        if let Some(mut input) = child.stdin.take() {
            let _ = input.write_all(b"Q\n");
        }
        let deadline = Instant::now() + Duration::from_millis(150);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.reaped = true;
                    break;
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                _ => {
                    let _ = child.kill();
                    match child.wait() {
                        Ok(_) => self.reaped = true,
                        Err(error) => eprintln!("CI child wait 失败：pid={} {error}", self.pid),
                    }
                    break;
                }
            }
        }
        eprintln!(
            "CI process cleanup: purpose={}, pid={}, started_at={:?}, reaped={}",
            self.purpose, self.pid, self.started_at, self.reaped
        );
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Certificate {
    directory: PathBuf,
    cert: PathBuf,
    key: PathBuf,
}

impl Certificate {
    fn generate(openssl: &std::path::Path) -> Self {
        let base = std::env::var_os("RUNNER_TEMP")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .canonicalize()
            .expect("CI 临时根目录");
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = base.join(format!(
            "satelite-service-route-connect-{}-{nonce}",
            std::process::id()
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory).expect("创建本测试专属证书目录");
        let owned = Self {
            cert: directory.join("service-route.crt"),
            key: directory.join("service-route.key"),
            directory,
        };
        let mut command = Command::new(openssl);
        command
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-sha256",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=service.test",
                "-addext",
                "subjectAltName=DNS:service.test",
                "-keyout",
            ])
            .arg(&owned.key)
            .arg("-out")
            .arg(&owned.cert)
            .env("MSYS_NO_PATHCONV", "1")
            .stdin(Stdio::null());
        let mut process = OwnedProcess::spawn(command, "生成 CONNECT 自签证书");
        process.wait_success(Duration::from_secs(10));
        assert!(owned.cert.is_file() && owned.key.is_file());
        owned
    }
}

impl Drop for Certificate {
    fn drop(&mut self) {
        use sha2::Digest;
        // 固定两个可再生成的测试文件：先逐个核验归属/大小/摘要并删除，再移除空目录。
        let Ok(directory) = self.directory.canonicalize() else {
            return;
        };
        for path in [&self.cert, &self.key] {
            if !path.exists() {
                continue;
            }
            let owned = path
                .canonicalize()
                .ok()
                .is_some_and(|resolved| resolved.parent() == Some(directory.as_path()));
            let regular = std::fs::symlink_metadata(path)
                .ok()
                .is_some_and(|metadata| metadata.file_type().is_file());
            if !owned || !regular {
                eprintln!("CI 保留归属异常文件：{path:?}");
                continue;
            }
            match std::fs::read(path) {
                Ok(bytes) => {
                    eprintln!(
                        "CI cleanup: path={path:?}, bytes={}, sha256={}",
                        bytes.len(),
                        hex::encode(sha2::Sha256::digest(&bytes))
                    );
                    if let Err(error) = std::fs::remove_file(path) {
                        eprintln!("CI 文件清理失败：{path:?} {error}");
                    }
                }
                Err(error) => eprintln!("CI 文件核验失败：{path:?} {error}"),
            }
        }
        if std::fs::read_dir(&directory)
            .ok()
            .is_some_and(|mut entries| entries.next().is_none())
        {
            if let Err(error) = std::fs::remove_dir(&directory) {
                eprintln!("CI 空目录清理失败：{directory:?} {error}");
            }
        }
    }
}

fn start_tls(openssl: &std::path::Path, certificate: &Certificate) -> (OwnedProcess, SocketAddr) {
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    let mut command = Command::new(openssl);
    command
        .args(["s_server", "-accept"])
        .arg(address.to_string())
        .arg("-cert")
        .arg(&certificate.cert)
        .arg("-key")
        .arg(&certificate.key)
        .args(["-www", "-quiet", "-ign_eof"])
        .stdin(Stdio::piped());
    drop(reservation);
    let mut process = OwnedProcess::spawn(command, "CONNECT 隔离 TLS 目标");
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        assert!(process.exited().is_none(), "CI openssl s_server 启动失败");
        if let Ok(socket) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
            let _ = socket.shutdown(Shutdown::Both);
            return (process, address);
        }
        assert!(Instant::now() < deadline, "CI TLS 目标就绪超时");
        thread::sleep(Duration::from_millis(10));
    }
}

#[derive(Clone)]
struct Flow {
    source: SocketAddr,
    node: &'static str,
}

/// 拥有 listener、所有 accepted/上游 socket 和子线程，Drop 先 shutdown 再 join。
struct LocalServer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    sockets: Arc<Mutex<Vec<TcpStream>>>,
    worker: Option<JoinHandle<()>>,
}

impl LocalServer {
    fn start(
        handler: impl Fn(TcpStream, SocketAddr, Arc<AtomicBool>, Arc<Mutex<Vec<TcpStream>>>)
            + Send
            + Sync
            + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let sockets = Arc::new(Mutex::new(Vec::new()));
        let (stopping, owned_sockets) = (stop.clone(), sockets.clone());
        let handler = Arc::new(handler);
        let worker = thread::spawn(move || {
            let mut workers = Vec::new();
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((socket, source)) => {
                        if stopping.load(Ordering::Acquire) {
                            break;
                        }
                        socket.set_nonblocking(false).unwrap();
                        socket
                            .set_read_timeout(Some(Duration::from_millis(100)))
                            .unwrap();
                        socket
                            .set_write_timeout(Some(Duration::from_millis(100)))
                            .unwrap();
                        owned_sockets
                            .lock()
                            .unwrap()
                            .push(socket.try_clone().unwrap());
                        let (handler, stopping, owned) =
                            (handler.clone(), stopping.clone(), owned_sockets.clone());
                        workers.push(thread::spawn(move || {
                            handler(socket, source, stopping, owned)
                        }));
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
            for worker in workers {
                worker.join().expect("CI listener 子线程正常退出");
            }
        });
        Self {
            address,
            stop,
            sockets,
            worker: Some(worker),
        }
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // 不持锁 join；accepted worker 登记较晚的 socket 也受 stop/deadline 控制。
        for socket in self
            .sockets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            let _ = socket.shutdown(Shutdown::Both);
        }
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() && !std::thread::panicking() {
                panic!("CI listener 线程退出异常");
            }
        }
    }
}

fn read_header(socket: &mut TcpStream, stop: &AtomicBool) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut bytes = Vec::new();
    while bytes.len() < 8192 && !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        let mut byte = [0];
        match socket.read(&mut byte) {
            Ok(1) => {
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n\r\n") {
                    return String::from_utf8(bytes).ok();
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            _ => return None,
        }
    }
    None
}

fn pump(mut reader: TcpStream, mut writer: TcpStream, stop: Arc<AtomicBool>) {
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut buffer = [0u8; 8192];
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => break,
        };
        let mut offset = 0;
        while offset < count && !stop.load(Ordering::Acquire) && Instant::now() < deadline {
            match writer.write(&buffer[offset..count]) {
                Ok(0) => return,
                Ok(written) => offset += written,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => return,
            }
        }
    }
    let _ = writer.shutdown(Shutdown::Write);
}

fn fake_api(flows: Arc<Mutex<Vec<Flow>>>) -> LocalServer {
    LocalServer::start(move |mut socket, _, stop, _| {
        let Some(header) = read_header(&mut socket, &stop) else {
            return;
        };
        let valid = header.starts_with("GET /connections HTTP/1.1\r\n")
            && header.lines().any(|line| {
                let Some((key, value)) = line.split_once(':') else {
                    return false;
                };
                key.eq_ignore_ascii_case("authorization")
                    && value.trim() == format!("Bearer {CI_SECRET}")
            });
        let rows: Vec<_> = flows.lock().unwrap().iter().map(|flow| json!({
            "id":format!("socket-{}",flow.source.port()),
            "metadata":{"sourceIP":flow.source.ip().to_string(),"sourcePort":flow.source.port(),
                "destinationIP":"127.0.0.1","destinationPort":443,"host":"service.test","network":"tcp"},
            "chains":[flow.node,"proxy"], "rule":"DomainSuffix", "rulePayload":"service.test"
        })).collect();
        let body = json!({"connections":rows}).to_string();
        let status = if valid { "200 OK" } else { "401 Unauthorized" };
        let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let _ = socket.write_all(reply.as_bytes());
        let _ = socket.shutdown(Shutdown::Both);
    })
}

fn connect_proxy(tls: SocketAddr, flows: Arc<Mutex<Vec<Flow>>>) -> LocalServer {
    LocalServer::start(move |mut client, source, stop, sockets| {
        let Some(header) = read_header(&mut client, &stop) else {
            return;
        };
        // 只接受唯一白名单 CONNECT 目标，始终直拨数值 loopback TLS 地址。
        if !header.starts_with("CONNECT service.test:443 HTTP/1.1\r\n") {
            return;
        }
        {
            let mut flows = flows.lock().unwrap();
            if flows.len() >= 2 {
                return;
            }
            let node = if flows.is_empty() { "node-a" } else { "node-b" };
            flows.push(Flow { source, node });
        }
        let Ok(server) = TcpStream::connect_timeout(&tls, Duration::from_secs(1)) else {
            return;
        };
        server
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        server
            .set_write_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        sockets.lock().unwrap().push(server.try_clone().unwrap());
        if client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .is_err()
        {
            return;
        }
        let (Ok(up_client), Ok(up_server)) = (client.try_clone(), server.try_clone()) else {
            return;
        };
        let up_stop = stop.clone();
        let upstream = thread::spawn(move || pump(up_client, up_server, up_stop));
        pump(server, client, stop);
        upstream.join().expect("CI CONNECT 上行线程退出");
    })
}

async fn request_through_connect(
    proxy: SocketAddr,
    route: Arc<RouteContext>,
    flows: Arc<Mutex<Vec<Flow>>>,
) -> (SocketAddr, RequestRoute) {
    let context = RequestContext::new(route, TARGET);
    scope(context.clone(), async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(format!("http://{proxy}")).unwrap())
            // 仅此 CI 自签证书客户端；不写入系统信任库，不影响生产客户端。
            .danger_accept_invalid_certs(true)
            .http1_only()
            .pool_max_idle_per_host(0)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let response = client
            .get(TARGET)
            .send()
            .await
            .expect("真实 CONNECT + TLS + HTTP 请求成功");
        assert!(response.status().is_success());
        let info = response
            .extensions()
            .get::<HttpInfo>()
            .expect("HTTPS CONNECT 响应必须保留 HttpInfo");
        let local = info.local_addr();
        assert_eq!(info.remote_addr(), proxy);
        observe(&response, TARGET).await;
        let records = context.records();
        assert_eq!(records.len(), 1);
        let record = records[0].clone();
        let expected = flows
            .lock()
            .unwrap()
            .iter()
            .find(|flow| flow.source == local)
            .cloned()
            .expect("HttpInfo.local_addr 精确等于 CONNECT 代理接受的 peer");
        assert_eq!(record.match_status, "matched");
        assert_eq!(record.role, "primary");
        assert_eq!(record.host, "service.test");
        assert_eq!(record.outbound_tag.as_deref(), Some(expected.node));
        assert_eq!(
            record.chains,
            vec![expected.node.to_owned(), "proxy".to_owned()]
        );
        assert_eq!(record.rule.as_deref(), Some("DomainSuffix"));
        assert_eq!(record.rule_payload.as_deref(), Some("service.test"));
        assert!(record.exit_ip.is_none());
        drop(response);
        drop(client);
        (local, record)
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "CI only: guarded loopback CONNECT + openssl TLS; no proxy core or public network"]
async fn live_connect_httpinfo_exact_routes_and_cleanup() {
    ci_guard();
    let openssl = openssl_path();
    let certificate = Certificate::generate(&openssl);
    let directory = certificate.directory.clone();
    let (mut tls_process, tls_address) = start_tls(&openssl, &certificate);
    let flows = Arc::new(Mutex::new(Vec::new()));
    let api = fake_api(flows.clone());
    let proxy = connect_proxy(tls_address, flows.clone());
    let (api_address, proxy_address) = (api.address, proxy.address);
    let config = json!({
        "outbounds":[{"tag":"node-a","type":"socks"},{"tag":"node-b","type":"socks"},
            {"tag":"proxy","type":"selector","outbounds":["node-a","node-b"]}],
        "inbounds":[{"tag":"mixed-in","type":"mixed","listen":"127.0.0.1","listen_port":proxy.address.port()}]
    });
    let route = RouteContext::from_config(
        Some(ClashApi::new("127.0.0.1", api.address.port(), CI_SECRET)),
        format!("ci-connect:{}", proxy.address),
        &config.to_string(),
        false,
    );
    let results = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            request_through_connect(proxy.address, route.clone(), flows.clone()),
            request_through_connect(proxy.address, route.clone(), flows.clone()),
        )
    })
    .await
    .expect("两个同 host CONNECT 请求在预算内完成");
    assert_ne!(results.0 .0, results.1 .0, "必须使用不同客户端源端口");
    assert_ne!(
        results.0 .1.outbound_tag, results.1 .1.outbound_tag,
        "同 host 两连接不得串路由"
    );
    let seen = flows.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().any(|flow| flow.node == "node-a"));
    assert!(seen.iter().any(|flow| flow.node == "node-b"));
    drop(proxy); // 停 listener、shutdown 两侧 socket、join 所有转发线程。
    drop(api);
    tls_process.stop();
    assert!(tls_process.child.is_none());
    assert!(tls_process.reaped, "TLS 子进程必须确认 wait 完成");
    for address in [proxy_address, api_address, tls_address] {
        assert!(
            TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err(),
            "测试监听已退出：{address}"
        );
    }
    drop(certificate); // 子进程已 wait，逐个删除两个证书文件后删除空目录。
    assert!(
        !directory.exists(),
        "CI 专属证书目录必须完整清理：{directory:?}"
    );
}
