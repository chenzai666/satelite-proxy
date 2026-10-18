//! 仅 CI 执行的真实协议回归；由 xray_latency.rs 的 cfg(test) 子模块引入。
//! 在仓库根目录的 Windows Actions 步骤中，先预取 bundled sing-box，再执行：
//! $env:TAURI_CONFIG = Get-Content src-tauri/tauri.singbox-windows.conf.json -Raw
//! cargo test --manifest-path src-tauri/Cargo.toml --locked --lib services::xray_latency::live_protocol_tests::live_delegated_protocols_probe_and_cleanup -- --ignored --exact --nocapture --test-threads=1
//! 服务端参数核对自 SagerNet/sing-box v1.13.15 的 option/{tuic,hysteria2,anytls}.go
//! 及 protocol/{tuic,hysteria2,anytls}/inbound.go；TUIC/HY2 使用真实 UDP/QUIC。

use super::*;
use crate::domain::{Protocol, ProtocolConfig, TlsConfig};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};

const TEST_UUID: &str = "c19e76aa-44c5-48a5-b4c8-d7b860a870e1";
const TEST_PASSWORD: &str = "test-only-live-protocol-password";
const TEST_OBFS: &str = "test-only-salamander-password";
const REQUEST_TIMEOUT_MS: u64 = 3000;

struct TestCertificate {
    directory: PathBuf,
    cert: PathBuf,
    key: PathBuf,
}

impl TestCertificate {
    fn generate() -> Self {
        let base = std::env::var_os("RUNNER_TEMP")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let directory = base.join(format!(
            "satelite-live-protocol-{}-{}",
            std::process::id(),
            crate::config::generate_api_secret()
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory).expect("创建独立 CI 证书目录");
        let owned = Self {
            cert: directory.join("localhost.crt"),
            key: directory.join("localhost.key"),
            directory,
        };
        let output = Command::new(find_openssl())
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
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost,IP:127.0.0.1",
                "-keyout",
            ])
            .arg(&owned.key)
            .arg("-out")
            .arg(&owned.cert)
            .env("MSYS_NO_PATHCONV", "1")
            .stdin(Stdio::null())
            .output()
            .expect("运行 CI openssl 生成测试专用自签证书");
        assert!(output.status.success(), "openssl 生成测试证书失败");
        assert!(owned.cert.is_file() && owned.key.is_file());
        owned
    }
}

impl Drop for TestCertificate {
    fn drop(&mut self) {
        // 仅删除本测试 create_dir 成功创建的随机目录，不操作系统信任库。
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn find_openssl() -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            candidates.push(directory.join(if cfg!(windows) {
                "openssl.exe"
            } else {
                "openssl"
            }));
        }
    }
    #[cfg(windows)]
    {
        // Git for Windows 的 openssl 往往未加入 Actions 的 PATH。
        for variable in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(root) = std::env::var_os(variable) {
                candidates.push(PathBuf::from(root).join("Git/usr/bin/openssl.exe"));
            }
        }
        if let Some(root) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(PathBuf::from(root).join("Programs/Git/usr/bin/openssl.exe"));
        }
        candidates.push(PathBuf::from(r"C:\Program Files\Git\usr\bin\openssl.exe"));
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .expect("CI 需要 PATH 中的 openssl 或 Windows Git/usr/bin/openssl.exe")
}

/// 每个场景独享目标及计数；连接一到即计数，连未完成 HTTP 的绕行也会被发现。
struct HttpTarget {
    address: SocketAddr,
    path: String,
    connections: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Vec<String>>>,
}

impl HttpTarget {
    fn start(label: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("分配 HTTP 目标端口");
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let path = format!("/{label}/generate_204");
        let connections = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let seen = connections.clone();
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            loop {
                let (mut stream, peer) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        // 停止时先排空 accept 队列，再冻结计数，避免漏记迟到绕行。
                        if stopping.load(Ordering::SeqCst) {
                            return requests;
                        }
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("HTTP 目标 accept 失败: {error}"),
                };
                seen.fetch_add(1, Ordering::SeqCst);
                assert!(peer.ip().is_loopback());
                // Winsock 可能继承 listener 的非阻塞状态，读请求前显式恢复。
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                    let mut byte = [0];
                    stream
                        .read_exact(&mut byte)
                        .expect("读取经节点传来的 HTTP 请求");
                    head.push(byte[0]);
                }
                assert!(head.ends_with(b"\r\n\r\n"), "HTTP 请求头超限");
                requests.push(
                    String::from_utf8(head)
                        .expect("HTTP 请求头 UTF-8")
                        .lines()
                        .next()
                        .unwrap()
                        .to_owned(),
                );
                stream
                    .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .expect("响应本机 HTTP 目标");
            }
        });
        Self {
            address,
            path,
            connections,
            stop,
            worker: Some(worker),
        }
    }

    fn url(&self) -> String {
        format!("http://{}{}", self.address, self.path)
    }

    fn assert_requests(mut self, expected: usize) {
        self.stop.store(true, Ordering::SeqCst);
        let requests = self.worker.take().unwrap().join().expect("HTTP 目标线程");
        assert_eq!(
            self.connections.load(Ordering::SeqCst),
            expected,
            "{} 的连接数",
            self.path
        );
        assert_eq!(requests.len(), expected, "{} 的完整 GET 数", self.path);
        for request in requests {
            assert_eq!(request, format!("GET {} HTTP/1.1", self.path));
        }
    }
}

impl Drop for HttpTarget {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn test_node(protocol: Protocol, port: u16, insecure: bool) -> ProxyNode {
    let config = match protocol {
        Protocol::Tuic => ProtocolConfig::Tuic {
            uuid: TEST_UUID.into(),
            password: TEST_PASSWORD.into(),
            congestion_control: Some("cubic".into()),
            udp_relay_mode: Some("native".into()),
            zero_rtt_handshake: false,
        },
        Protocol::Hysteria2 => ProtocolConfig::Hysteria2 {
            password: TEST_PASSWORD.into(),
            up_mbps: None,
            down_mbps: None,
            // 真实 obfs 同时保证开启证书校验时仍需委托 sing-box。
            obfs: Some("salamander".into()),
            obfs_password: Some(TEST_OBFS.into()),
        },
        Protocol::AnyTls => ProtocolConfig::AnyTls {
            password: TEST_PASSWORD.into(),
        },
        _ => panic!("非本测试协议"),
    };
    ProxyNode {
        id: format!("live-{}-{port}", protocol.as_str()),
        name: format!("CI-only {}", protocol.as_str()),
        protocol,
        server: "127.0.0.1".into(),
        port,
        tls: Some(TlsConfig {
            enabled: true,
            server_name: Some("localhost".into()),
            insecure: Some(insecure),
            // TUIC 故意不填 ALPN，由生产兼容 helper 补 h3。
            ..TlsConfig::default()
        }),
        transport: None,
        udp: Some(true),
        config,
        source: None,
        raw: None,
        latency_ms: None,
        latency_at: None,
    }
}

fn wait_ready(process: &mut ProbeChild, address: SocketAddr) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            process.child.try_wait().expect("检查测试子进程").is_none(),
            "测试内核提前退出: {:?}",
            process.diagnostic_error()
        );
        if TcpStream::connect_timeout(&address, Duration::from_millis(50)).is_ok() {
            // 监听成功后再检查一次自有子进程，发现端口竞争导致的退出。
            assert!(process.child.try_wait().unwrap().is_none());
            return;
        }
        assert!(Instant::now() < deadline, "测试内核就绪超时");
        thread::sleep(Duration::from_millis(25));
    }
}

enum ProtocolPort {
    Udp(UdpSocket),
    Tcp(TcpListener),
}

impl ProtocolPort {
    fn reserve(protocol: Protocol) -> Self {
        if protocol == Protocol::AnyTls {
            Self::Tcp(TcpListener::bind("127.0.0.1:0").unwrap())
        } else {
            Self::Udp(UdpSocket::bind("127.0.0.1:0").unwrap())
        }
    }

    fn address(&self) -> SocketAddr {
        match self {
            Self::Udp(socket) => socket.local_addr().unwrap(),
            Self::Tcp(listener) => listener.local_addr().unwrap(),
        }
    }
}

fn server_config(
    protocol: Protocol,
    address: SocketAddr,
    ready: SocketAddr,
    certificate: &TestCertificate,
    target_ports: &[u16],
) -> Value {
    let mut inbound = json!({
        "type": protocol.as_str(), "tag": "live-protocol",
        "listen": "127.0.0.1", "listen_port": address.port(),
        "users": [{ "name": "ci-only", "password": TEST_PASSWORD }],
        "tls": { "enabled": true, "certificate_path": certificate.cert, "key_path": certificate.key }
    });
    match protocol {
        Protocol::Tuic => {
            inbound["users"][0]["uuid"] = json!(TEST_UUID);
            inbound["congestion_control"] = json!("cubic");
            inbound["auth_timeout"] = json!("1s");
            inbound["tls"]["alpn"] = json!(["h3"]);
        }
        Protocol::Hysteria2 => {
            inbound["obfs"] = json!({"type": "salamander", "password": TEST_OBFS});
            inbound["tls"]["alpn"] = json!(["h3"]);
        }
        Protocol::AnyTls => {}
        _ => unreachable!(),
    }
    json!({
        "log": {"level": "warn", "timestamp": false},
        // 第二入站只用于同一进程的启动屏障；协议端口仍是真 UDP 或 TLS。
        "inbounds": [inbound, {
            "type": "http", "tag": "ready", "listen": "127.0.0.1", "listen_port": ready.port(),
            "users": [{"username": "ci-only", "password": crate::config::generate_api_secret()}]
        }],
        "outbounds": [{"type": "direct", "tag": "fixture-direct"}],
        "route": {"rules": [
            {"inbound": ["live-protocol"], "ip_cidr": ["127.0.0.1/32"], "port": target_ports,
             "action": "route", "outbound": "fixture-direct"},
            {"action": "reject"}
        ]}
    })
}

fn assert_released(protocol: Protocol, address: SocketAddr, ready: SocketAddr) {
    let ready_listener = TcpListener::bind(ready).expect("服务端启动屏障端口已释放");
    match protocol {
        Protocol::AnyTls => {
            let _listener = TcpListener::bind(address).expect("AnyTLS TCP 端口已释放");
        }
        _ => {
            let _socket = UdpSocket::bind(address).expect("TUIC/HY2 UDP 端口已释放");
        }
    }
    drop(ready_listener);
}

fn assert_probe_drop_releases_port(bin: &Path, node: &ProxyNode) {
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    let config = config_for(
        CoreKind::SingBox,
        node,
        address.port(),
        "ci-only-drop",
        false,
    )
    .unwrap();
    drop(reservation);
    let mut process = spawn_core_probe(CoreKind::SingBox, bin, config).unwrap();
    wait_ready(&mut process, address);
    drop(process); // 显式覆盖生产 Drop 的 kill/wait/feeder/readers 回收路径。
    let _rebound = TcpListener::bind(address).expect("测速入口 Drop 后端口已释放");
}

fn assert_network_failure(result: Result<u32, String>, label: &str) {
    let error = result.expect_err(label);
    assert!(
        ["connection:", "timeout:", "http:"]
            .iter()
            .any(|prefix| error.starts_with(prefix)),
        "{label} 应是协议认证/网络失败，实际为 {error}"
    );
}

#[test]
#[ignore = "仅 CI：需要 bundled sing-box、openssl，启动隔离的真实 UDP/TLS 服务端"]
fn live_delegated_protocols_probe_and_cleanup() {
    assert_eq!(
        std::env::var("CI").as_deref(),
        Ok("true"),
        "仅允许 CI 执行 live 协议测试"
    );
    let bin = crate::core::find_bundled_core(None, CoreKind::SingBox).expect("CI bundled sing-box");
    let certificate = TestCertificate::generate();
    for protocol in [Protocol::Tuic, Protocol::Hysteria2, Protocol::AnyTls] {
        let success = HttpTarget::start("success");
        let wrong_password = HttpTarget::start("wrong-password");
        let verified_tls = HttpTarget::start("verified-tls");
        let stopped_server = HttpTarget::start("stopped-server");
        let target_ports = [
            success.address.port(),
            wrong_password.address.port(),
            verified_tls.address.port(),
            stopped_server.address.port(),
        ];
        let protocol_port = ProtocolPort::reserve(protocol);
        let address = protocol_port.address();
        let ready_reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let ready = ready_reservation.local_addr().unwrap();
        let server_doc = server_config(protocol, address, ready, &certificate, &target_ports);
        drop(protocol_port);
        drop(ready_reservation);
        let mut server = spawn_core_probe(CoreKind::SingBox, &bin, server_doc.clone()).unwrap();
        wait_ready(&mut server, ready);
        if protocol != Protocol::AnyTls {
            let error = UdpSocket::bind(address).expect_err("真实 QUIC 服务端必须占用 UDP 端口");
            assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        }

        let mut original = test_node(protocol, address.port(), false);
        let mut compatible = original.clone();
        let changed = apply_udp_node_compatibility(std::slice::from_mut(&mut compatible), true);
        if protocol == Protocol::AnyTls {
            assert_eq!(changed, 0);
            assert_eq!(compatible.tls.as_ref().unwrap().insecure, Some(false));
            // AnyTLS 不属于 UDP 兼容范围，成功测试显式设置订阅 insecure。
            original.tls.as_mut().unwrap().insecure = Some(true);
            compatible = original.clone();
        } else {
            assert_eq!(changed, 1);
            assert_eq!(compatible.tls.as_ref().unwrap().insecure, Some(true));
        }
        assert_eq!(probe_kind(&compatible), CoreKind::SingBox);
        // 关闭兼容开关不得覆盖订阅本来就显式设置的 insecure=true。
        apply_udp_node_compatibility(std::slice::from_mut(&mut compatible), false);
        assert_eq!(compatible.tls.as_ref().unwrap().insecure, Some(true));
        let config = config_for(
            CoreKind::SingBox,
            &compatible,
            30001,
            "ci-only-config",
            false,
        )
        .unwrap();
        assert_eq!(
            config,
            build_singbox_probe_config(&compatible, 30001, "ci-only-config", false).unwrap()
        );
        assert_eq!(config["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(
            config["inbounds"][0]["users"][0]["password"],
            "ci-only-config"
        );
        let outbounds = config["outbounds"].as_array().unwrap();
        assert_eq!(outbounds.len(), 1, "单协议测速没有直连兜底");
        assert_eq!(outbounds[0]["type"], protocol.as_str());
        assert_eq!(outbounds[0]["tls"]["insecure"], true);
        assert_eq!(config["route"]["final"], outbounds[0]["tag"]);
        if protocol == Protocol::Tuic {
            assert_eq!(outbounds[0]["tls"]["alpn"], json!(["h3"]));
        }

        // 传未预处理的 HY2/TUIC，验证批量入口实际继承兼容设置和内核分派。
        let streamed = Mutex::new(Vec::new());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let results = runtime
            .block_on(probe_xray_nodes_with_fallback(
                vec![original.clone()],
                ProbeBinaries {
                    xray: None,
                    singbox: Some(bin.clone()),
                },
                success.url(),
                Some(REQUEST_TIMEOUT_MS),
                false,
                false,
                true,
                |result| streamed.lock().unwrap().push(result.id.clone()),
            ))
            .expect("执行真实协议批量测速");
        drop(runtime);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, original.id);
        assert_eq!(results[0].method, "singbox_http");
        assert!(
            results[0].latency_ms.is_some() && results[0].error.is_none(),
            "{protocol:?}: {:?}",
            results[0]
        );
        assert_eq!(*streamed.lock().unwrap(), vec![original.id.clone()]);
        success.assert_requests(2);

        let mut wrong = compatible.clone();
        match &mut wrong.config {
            ProtocolConfig::Tuic { password, .. }
            | ProtocolConfig::Hysteria2 { password, .. }
            | ProtocolConfig::AnyTls { password } => *password = "test-only-wrong-password".into(),
            _ => unreachable!(),
        }
        assert_network_failure(
            probe_one_with_core(
                probe_kind(&wrong),
                &bin,
                &wrong,
                &wrong_password.url(),
                REQUEST_TIMEOUT_MS,
                false,
            ),
            "错误密码必须失败",
        );
        wrong_password.assert_requests(0);

        // 从全新 insecure=false 节点出发，避免把关闭开关误读为重写显式订阅。
        let mut verified = test_node(protocol, address.port(), false);
        apply_udp_node_compatibility(std::slice::from_mut(&mut verified), false);
        assert_eq!(verified.tls.as_ref().unwrap().insecure, Some(false));
        assert_eq!(probe_kind(&verified), CoreKind::SingBox);
        let verified_config = config_for(
            probe_kind(&verified),
            &verified,
            30002,
            "ci-only-verify",
            false,
        )
        .unwrap();
        assert_ne!(verified_config["outbounds"][0]["tls"]["insecure"], true);
        let error = probe_one_with_core(
            probe_kind(&verified),
            &bin,
            &verified,
            &verified_tls.url(),
            REQUEST_TIMEOUT_MS,
            false,
        )
        .expect_err("未信任的 localhost 自签证书必须校验失败");
        assert!(
            error.starts_with("tls:"),
            "证书失败应分类为 tls:，实际为 {error}"
        );
        verified_tls.assert_requests(0);
        assert_probe_drop_releases_port(&bin, &compatible);

        // stop 后的进程状态确认 wait 已收尸，再验证真实 UDP/TCP 监听释放。
        server.stop();
        assert!(
            server.child.try_wait().unwrap().is_some(),
            "自有服务端进程已退出"
        );
        drop(server);
        assert_released(protocol, address, ready);
        // 再启动同一服务端并只调用 Drop，覆盖带真实 UDP 监听的清理分支。
        let mut server = spawn_core_probe(CoreKind::SingBox, &bin, server_doc).unwrap();
        wait_ready(&mut server, ready);
        drop(server);
        assert_released(protocol, address, ready);
        assert_network_failure(
            probe_one_with_core(
                probe_kind(&compatible),
                &bin,
                &compatible,
                &stopped_server.url(),
                REQUEST_TIMEOUT_MS,
                false,
            ),
            "服务端关闭后不得绕过节点直连",
        );
        stopped_server.assert_requests(0);
    }
    let certificate_directory = certificate.directory.clone();
    drop(certificate);
    assert!(!certificate_directory.exists(), "测试证书和私钥已清理");
}
