//! sing-box process lifecycle.

use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Windows process creation flag: do not allocate a console window for the child.
/// sing-box.exe is a console subsystem program, so without this a black cmd window
/// flashes on screen every time we spawn it.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

/// How the core process is owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum RunMode {
    #[default]
    None,
    /// Direct child of the GUI process (macOS TUN: setuid binary, still our Child).
    Sidecar,
    /// Windows TUN: elevated process tracked by PID only.
    #[allow(dead_code)] // constructed only on Windows
    ElevatedPid,
}

#[derive(Debug)]
pub struct CoreManager {
    child: Option<Child>,
    /// Windows TUN elevated process, or legacy macOS elevated (should be unused).
    elevated_pid: Option<u32>,
    run_mode: RunMode,
    state: CoreState,
    last_error: Option<String>,
    config_path: Option<PathBuf>,
    binary_path: Option<PathBuf>,
    log_path: Option<PathBuf>,
    log_dir: Option<PathBuf>,
}

impl Default for CoreManager {
    fn default() -> Self {
        Self {
            child: None,
            elevated_pid: None,
            run_mode: RunMode::None,
            state: CoreState::Stopped,
            last_error: None,
            config_path: None,
            binary_path: None,
            log_path: None,
            log_dir: None,
        }
    }
}

impl CoreManager {
    pub fn state(&self) -> CoreState {
        self.state
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn log_path(&self) -> Option<&Path> {
        self.log_path.as_deref()
    }

    fn latest_log_path(&self) -> Option<PathBuf> {
        self.log_dir
            .as_ref()
            .map(|dir| crate::log_retention::hourly_path(dir, "sing-box"))
            .filter(|path| path.exists())
            .or_else(|| self.log_path.clone())
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, CoreState::Running)
    }

    /// Reap child if it exited; update state with log tail when possible.
    pub fn poll(&mut self) {
        if let Some(pid) = self.elevated_pid {
            if !pid_alive(pid) {
                self.elevated_pid = None;
                self.run_mode = RunMode::None;
                if self.state == CoreState::Stopping {
                    self.state = CoreState::Stopped;
                } else if self.state != CoreState::Stopped {
                    self.state = CoreState::Error;
                    let detail = self
                        .latest_log_path()
                        .as_ref()
                        .and_then(|p| read_log_tail(p, 4000))
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| format!("elevated sing-box (pid {pid}) exited"));
                    self.last_error = Some(map_tun_permission_hint(&strip_ansi(&detail)));
                }
            }
            return;
        }

        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.child = None;
                    self.run_mode = RunMode::None;
                    if self.state == CoreState::Stopping {
                        self.state = CoreState::Stopped;
                    } else {
                        self.state = CoreState::Error;
                        let detail = self
                            .latest_log_path()
                            .as_ref()
                            .and_then(|p| read_log_tail(p, 4000))
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or_else(|| format!("sing-box exited: {status}"));
                        self.last_error = Some(map_tun_permission_hint(&strip_ansi(&detail)));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    self.child = None;
                    self.run_mode = RunMode::None;
                    self.state = CoreState::Error;
                    self.last_error = Some(e.to_string());
                }
            }
        }
    }

    pub fn check_config(binary: &Path, config: &Path) -> AppResult<()> {
        let mut cmd = Command::new(binary);
        cmd.args(["check", "-c"]).arg(config);
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let out = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| AppError::Core(format!("check spawn failed: {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let out_s = String::from_utf8_lossy(&out.stdout);
            let mut detail = String::new();
            let e = strip_ansi(err.trim());
            let o = strip_ansi(out_s.trim());
            if !e.is_empty() {
                detail.push_str(&e);
            }
            if !o.is_empty() {
                if !detail.is_empty() {
                    detail.push('\n');
                }
                detail.push_str(&o);
            }

            // SIGKILL / no message ⇒ process killed externally (not a JSON/DNS parse error).
            let status_s = out.status.to_string();
            let killed = status_s.contains("SIGKILL")
                || status_s.contains("signal: 9")
                || out.status.code().is_none() && detail.is_empty();
            if detail.is_empty() {
                detail = if killed {
                    "进程被系统强制结束 (SIGKILL)，通常不是配置/DNS 语法错误。\n\
                     常见原因：\n\
                     1) 路径未加引号：Application Support 含空格，须写成\n\
                        sing-box check -c \"/Users/…/Application Support/…/active.json\"\n\
                     2) 从 target/debug/resources 直接跑内置内核可能被 macOS 杀掉\n\
                        （应用会复制到 Application Support/…/bin/ 再执行）\n\
                     3) 内存不足 / 安全软件拦截\n\
                     请用:  \"…/bin/sing-box\" check -c \"…/active.json\""
                        .into()
                } else {
                    format!("exit status {status_s}")
                };
            } else if killed {
                detail = format!(
                    "{detail}\n(进程随后被 SIGKILL；若仅有此信号，优先排查路径空格/二进制路径，而非 DNS)"
                );
            }

            return Err(AppError::Core(format!(
                "sing-box check failed ({status_s})\nconfig: {}\nbinary: {}\n{detail}",
                config.display(),
                binary.display(),
            )));
        }
        Ok(())
    }

    /// True if nothing is listening on 127.0.0.1:port.
    pub fn is_port_free(port: u16) -> bool {
        TcpListener::bind(("127.0.0.1", port)).is_ok()
    }

    pub fn has_port_listener(port: u16) -> bool {
        port_has_listener(port)
    }

    /// Force-free a TCP listen port: kill listeners + short wait.
    ///
    /// Important: if nothing is in LISTEN, return immediately (or after one short
    /// settle). A false `bind` failure without a listener used to spin ~2s and
    /// made settings restarts feel stuck (e.g. changing route.final).
    pub fn force_free_port(port: u16) -> AppResult<()> {
        if Self::is_port_free(port) {
            return Ok(());
        }
        let mut killed = kill_listeners_on_port(port);

        // No server socket → do not busy-wait (CLOSE_WAIT / TIME_WAIT / bind flake).
        if !port_has_listener(port) {
            std::thread::sleep(Duration::from_millis(40));
            if Self::is_port_free(port) || !port_has_listener(port) {
                return Ok(());
            }
        }

        // Real LISTEN holder: wait briefly for kill to take effect (~360ms max).
        for i in 0..12 {
            if Self::is_port_free(port) {
                return Ok(());
            }
            if !port_has_listener(port) {
                return Ok(());
            }
            if i == 4 || i == 8 {
                killed = kill_listeners_on_port(port);
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        if Self::is_port_free(port) || !port_has_listener(port) {
            return Ok(());
        }
        let manual = if cfg!(windows) {
            format!("netstat -ano | findstr :{port}")
        } else {
            format!("sudo lsof -iTCP:{port} -sTCP:LISTEN")
        };
        Err(AppError::Core(format!(
            "端口 {port} 仍被占用（已尝试结束监听进程: {killed}）。可手动: {manual}"
        )))
    }

    /// Ensure mixed + API ports are free (kill leftovers from previous runs).
    pub fn ensure_ports_free(ports: &[u16]) -> AppResult<()> {
        let list: Vec<u16> = ports.iter().copied().filter(|p| *p != 0).collect();
        if list.is_empty() {
            return Ok(());
        }
        for &p in &list {
            Self::force_free_port(p)?;
        }
        Ok(())
    }

    /// Start core.
    ///
    /// When `elevated` is true (TUN):
    /// - **macOS**: one-time setuid on sing-box (`chown root:admin` + `chmod +sx`),
    ///   then normal sidecar spawn (euid root, ruid user — parent can kill).
    /// - **Windows**: UAC-elevate sing-box directly.
    pub fn start_with_ports(
        &mut self,
        binary: &Path,
        config: &Path,
        log_dir: &Path,
        mixed_port: u16,
        api_port: Option<u16>,
        elevated: bool,
        _resource_dir: Option<&Path>,
    ) -> AppResult<()> {
        self.poll();
        if matches!(self.state, CoreState::Running | CoreState::Starting) {
            return Ok(());
        }

        // Drop our own child first if still tracked.
        let _ = self.stop();
        let mut ports = vec![mixed_port];
        if let Some(api) = api_port {
            if api != mixed_port {
                ports.push(api);
            }
        }
        Self::ensure_ports_free(&ports)?;

        #[cfg(target_os = "macos")]
        if elevated {
            if let Err(e) = super::macos_auth::ensure_core_setuid(binary) {
                self.state = CoreState::Error;
                let msg = map_tun_permission_hint(&e.to_string());
                self.last_error = Some(msg.clone());
                return Err(AppError::Core(msg));
            }
        }

        Self::check_config(binary, config)?;
        // Light re-check only (first ensure_ports_free already waited if needed).
        for &p in &ports {
            if !Self::is_port_free(p) && port_has_listener(p) {
                Self::force_free_port(p)?;
            }
        }

        fs::create_dir_all(log_dir).map_err(|e| AppError::Core(format!("create log dir: {e}")))?;
        // One file per wall-clock hour. Core restarts within that hour append,
        // preserving the sequence around TUN and capture-mode transitions.
        let log_path = crate::log_retention::hourly_path(log_dir, "sing-box");
        crate::log_retention::cleanup_current_hour(log_dir)
            .map_err(|e| AppError::Core(format!("clean logs: {e}")))?;
        let _ = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| AppError::Core(format!("open log: {e}")))?;

        self.state = CoreState::Starting;
        self.last_error = None;
        self.log_path = Some(log_path.clone());
        self.log_dir = Some(log_dir.to_path_buf());
        self.config_path = Some(config.to_path_buf());
        self.binary_path = Some(binary.to_path_buf());
        self.elevated_pid = None;
        self.child = None;
        self.run_mode = RunMode::None;

        #[cfg(target_os = "windows")]
        if elevated {
            return self.start_elevated_windows(binary, config, &log_path, mixed_port);
        }

        let mut cmd = Command::new(binary);
        cmd.args(["run", "-c"]).arg(config);
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                self.state = CoreState::Error;
                self.last_error = Some(e.to_string());
                AppError::Core(format!("spawn sing-box failed: {e}"))
            })?;

        // Tie the child to the parent's lifetime via a Job Object: if this
        // process dies for any reason (crash, installer kill, Task Manager),
        // Windows reaps sing-box too — preventing orphaned ports on next launch.
        #[cfg(target_os = "windows")]
        {
            if let Err(e) = super::job::ensure_child_killed_on_parent_exit(child.id()) {
                crate::app_log::warn(
                    "core",
                    format!("job-object bind failed (orphan possible on crash): {e}"),
                );
            }
        }

        #[cfg(target_os = "macos")]
        if elevated {
            crate::app_log::info(
                "core",
                format!(
                    "started setuid sing-box as sidecar pid={} (TUN)",
                    child.id()
                ),
            );
        }

        let mut child = child;
        let writer = std::sync::Arc::new(std::sync::Mutex::new(RotatingCoreWriter::new(
            log_dir.to_path_buf(),
        )));
        if let Some(stdout) = child.stdout.take() {
            spawn_rotating_log_copy(stdout, std::sync::Arc::clone(&writer));
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_rotating_log_copy(stderr, writer);
        }
        self.child = Some(child);
        self.run_mode = RunMode::Sidecar;

        self.wait_until_ready(mixed_port)
    }

    /// Start sing-box elevated via UAC (Windows). Needed for TUN to create the
    /// virtual adapter. stdout/stderr are appended to `log_path` directly.
    #[cfg(target_os = "windows")]
    fn start_elevated_windows(
        &mut self,
        binary: &Path,
        config: &Path,
        _log_path: &Path,
        mixed_port: u16,
    ) -> AppResult<()> {
        let helper = std::env::current_exe()
            .map_err(|e| AppError::Core(format!("resolve log helper: {e}")))?;
        let log_dir = self
            .log_dir
            .as_ref()
            .ok_or_else(|| AppError::Core("log directory missing".into()))?;
        let args = format!(
            "--satelite-core-helper \"{}\" \"{}\" \"{}\"",
            escape_windows_arg(binary),
            escape_windows_arg(config),
            escape_windows_arg(log_dir)
        );

        let _elevated = match super::elevate::run_elevated(&helper, &args, None) {
            Ok(c) => c,
            Err(e) => {
                self.state = CoreState::Error;
                self.last_error = Some(e.to_string());
                return Err(e);
            }
        };
        // run_elevated returns an ElevatedChild that closes the handle on drop;
        // we only need the PID — we poll via OpenProcess later (elevate::pid_alive)
        // and kill via taskkill. Dropping here is fine: closing the handle does
        // NOT terminate the process.
        let pid = _elevated.pid;

        self.elevated_pid = Some(pid);
        self.run_mode = RunMode::ElevatedPid;
        self.wait_until_ready(mixed_port)
    }

    fn wait_until_ready(&mut self, mixed_port: u16) -> AppResult<()> {
        // wait a bit for immediate FATAL
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(100));
            self.poll();
            if !self.process_tracked_alive() {
                let err = self
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "process exited immediately".into());
                self.state = CoreState::Error;
                self.run_mode = RunMode::None;
                return Err(AppError::Core(map_tun_permission_hint(&err)));
            }
            if !Self::is_port_free(mixed_port) {
                break;
            }
        }

        self.poll();
        if !self.process_tracked_alive() {
            let err = self
                .last_error
                .clone()
                .unwrap_or_else(|| "process exited immediately".into());
            self.state = CoreState::Error;
            self.run_mode = RunMode::None;
            return Err(AppError::Core(map_tun_permission_hint(&err)));
        }

        self.state = CoreState::Running;
        Ok(())
    }

    fn process_tracked_alive(&self) -> bool {
        match self.run_mode {
            RunMode::ElevatedPid => self.elevated_pid.map(pid_alive).unwrap_or(false),
            RunMode::Sidecar => self.child.is_some(),
            RunMode::None => false,
        }
    }

    pub fn stop(&mut self) -> AppResult<()> {
        self.poll();

        if let Some(pid) = self.elevated_pid {
            self.state = CoreState::Stopping;
            elevated_kill(pid);
            let deadline = std::time::Instant::now() + Duration::from_secs(4);
            while pid_alive(pid) && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(80));
            }
            if pid_alive(pid) {
                elevated_kill_force(pid);
            }
            if pid_alive(pid) {
                // Do NOT forget the PID — caller must know stop failed.
                self.state = CoreState::Error;
                self.last_error = Some(format!(
                    "无法结束 elevated sing-box (pid {pid})；可能需要管理员权限"
                ));
                return Err(AppError::Core(self.last_error.clone().unwrap_or_default()));
            }
            self.elevated_pid = None;
            self.run_mode = RunMode::None;
            self.state = CoreState::Stopped;
            self.last_error = None;
            return Ok(());
        }

        let Some(mut child) = self.child.take() else {
            self.run_mode = RunMode::None;
            self.state = CoreState::Stopped;
            return Ok(());
        };

        self.state = CoreState::Stopping;

        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-TERM", &child.id().to_string()])
                .status();
        }
        #[cfg(not(unix))]
        {
            let _ = child.kill();
        }

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                Err(_) => break,
            }
        }

        self.run_mode = RunMode::None;
        self.state = CoreState::Stopped;
        self.last_error = None;
        Ok(())
    }

    /// Hard-stop the managed core process during application exit.
    ///
    /// Do not run the port-orphan sweep here. On Windows, the OS rejects new
    /// process creation once shutdown is in progress, so spawning `netstat`
    /// from the exit callback produces a visible "netstat failed to start"
    /// error. The managed sidecar/elevated PID is stopped directly above; any
    /// stale listener is handled by `ensure_ports_free` on the next startup.
    pub fn force_shutdown(&mut self) {
        let _ = self.stop();
        self.state = CoreState::Stopped;
        self.child = None;
        self.elevated_pid = None;
        self.run_mode = RunMode::None;
        self.last_error = None;
    }
}

#[cfg(target_os = "windows")]
fn escape_windows_arg(path: &Path) -> String {
    path.to_string_lossy().replace('"', "\\\"")
}

/// Elevated helper entry point. It owns the real sing-box child, captures both
/// output streams through the same hourly writer, and binds the child to a Job
/// Object so killing the helper also kills sing-box.
#[cfg(target_os = "windows")]
pub fn try_run_elevated_log_helper() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    let marker = args
        .iter()
        .position(|value| value == "--satelite-core-helper")?;
    if args.len() <= marker + 3 {
        return Some(2);
    }
    let binary = PathBuf::from(&args[marker + 1]);
    let config = PathBuf::from(&args[marker + 2]);
    let log_dir = PathBuf::from(&args[marker + 3]);
    if fs::create_dir_all(&log_dir).is_err() {
        return Some(3);
    }
    let mut command = Command::new(binary);
    command
        .args(["run", "-c"])
        .arg(config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return Some(4),
    };
    if super::job::ensure_child_killed_on_parent_exit(child.id()).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Some(5);
    }
    let writer = std::sync::Arc::new(std::sync::Mutex::new(RotatingCoreWriter::new(log_dir)));
    if let Some(stdout) = child.stdout.take() {
        spawn_rotating_log_copy(stdout, std::sync::Arc::clone(&writer));
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_rotating_log_copy(stderr, writer);
    }
    Some(
        child
            .wait()
            .ok()
            .and_then(|status| status.code())
            .unwrap_or(1),
    )
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "windows")]
    {
        super::elevate::pid_alive(pid)
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = pid;
        false
    }
}

/// Terminate an elevated (non-service) sing-box process.
/// Windows: parent retains PROCESS_TERMINATE. macOS legacy path: osascript.
fn elevated_kill(pid: u32) {
    #[cfg(target_os = "macos")]
    {
        let shell = format!("kill -TERM {pid} 2>/dev/null || true");
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            shell.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let _ = Command::new("osascript").arg("-e").arg(&script).status();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = super::elevate::terminate_pid(pid);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
}

fn elevated_kill_force(pid: u32) {
    #[cfg(target_os = "macos")]
    {
        let shell = format!("kill -KILL {pid} 2>/dev/null || true");
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            shell.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let _ = Command::new("osascript").arg("-e").arg(&script).status();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = super::elevate::terminate_pid(pid);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status();
    }
}

fn map_tun_permission_hint(err: &str) -> String {
    let lower = err.to_ascii_lowercase();
    if lower.contains("operation not permitted")
        || lower.contains("configure tun")
        || lower.contains("permission denied")
        || lower.contains("access is denied")
    {
        let platform_hint = if cfg!(target_os = "windows") {
            "TUN 模式需要管理员权限以创建虚拟网卡。开启 TUN 时应用会弹出 UAC 授权框并以管理员身份运行 sing-box。\n\
             请在 UAC 弹窗中点「是」；若点了「否」，请关闭 TUN 开关后重试，或以管理员身份运行本程序。"
        } else {
            "TUN 需要更高权限才能创建虚拟网卡 (utun)。\n\
             macOS：首次开启 TUN 会为 sing-box 设置 setuid（一次 Touch ID / 密码），之后启停不再弹密码。\n\
             若刚更新过内核，可能需重新授权一次。"
        };
        format!("{err}\n\n{platform_hint}")
    } else {
        err.to_string()
    }
}

fn port_has_listener(port: u16) -> bool {
    !listener_pids_on_port(port).is_empty()
}

fn listener_pids_on_port(port: u16) -> Vec<u32> {
    #[cfg(unix)]
    {
        let out = Command::new("lsof")
            .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
            .output();
        let Ok(out) = out else {
            return Vec::new();
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let mut pids: Vec<u32> = text.lines().filter_map(|l| l.trim().parse().ok()).collect();
        pids.sort_unstable();
        pids.dedup();
        let self_pid = std::process::id();
        pids.retain(|p| *p != self_pid);
        pids
    }
    #[cfg(not(unix))]
    {
        let _ = port;
        Vec::new()
    }
}

/// Kill PIDs listening on `port` (TCP LISTEN). Returns a short summary string.
fn kill_listeners_on_port(port: u16) -> String {
    #[cfg(unix)]
    {
        let pids = listener_pids_on_port(port);
        if pids.is_empty() {
            return "未找到监听进程".into();
        }
        let mut killed = Vec::new();
        for pid in pids {
            // setuid core keeps ruid=user so TERM/KILL from the app works.
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
            std::thread::sleep(Duration::from_millis(80));
            let still = Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if still {
                let _ = Command::new("kill")
                    .args(["-KILL", &pid.to_string()])
                    .status();
            }
            killed.push(pid.to_string());
        }
        format!("已结束 PID {}", killed.join(","))
    }
    #[cfg(not(unix))]
    {
        // netstat -ano lists every TCP row with the owning PID in the last column.
        // We find rows whose local address ends with ":<port>" in LISTENING state,
        // then taskkill each owning PID.
        let mut cmd = Command::new("netstat");
        cmd.args(["-ano"]);
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let out = match cmd.output() {
            Ok(o) => o,
            Err(e) => return format!("netstat 不可用: {e}"),
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let needle = format!(":{port}");
        let mut pids: Vec<u32> = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            // Row shape: "  TCP    127.0.0.1:2080     0.0.0.0:0    LISTENING    10528"
            if !trimmed.to_ascii_uppercase().contains("LISTENING") {
                continue;
            }
            if !trimmed.contains(&needle) {
                continue;
            }
            // PID is the last whitespace-delimited token.
            if let Some(pid) = trimmed
                .split_whitespace()
                .last()
                .and_then(|s| s.parse().ok())
            {
                pids.push(pid);
            }
        }
        pids.sort_unstable();
        pids.dedup();
        // Don't kill ourselves
        let self_pid = std::process::id();
        pids.retain(|p| *p != self_pid);
        if pids.is_empty() {
            return "未找到监听进程".into();
        }
        let mut killed = Vec::new();
        for pid in pids {
            // taskkill /F /T: force-kill the process tree (sing-box may have children).
            let mut k = Command::new("taskkill");
            k.args(["/F", "/T", "/PID", &pid.to_string()]);
            #[cfg(target_os = "windows")]
            k.creation_flags(CREATE_NO_WINDOW);
            match k.status() {
                Ok(s) if s.success() => killed.push(pid.to_string()),
                _ => killed.push(format!("{pid}?(失败)")),
            }
        }
        format!("已结束 PID {}", killed.join(","))
    }
}

fn read_log_tail(path: &Path, max_bytes: u64) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).ok()?;
    // prefer last FATAL/ERROR line
    let useful: Vec<&str> = buf
        .lines()
        .filter(|l| {
            let u = l.to_ascii_uppercase();
            u.contains("FATAL") || u.contains("ERROR") || u.contains("FAILED")
        })
        .collect();
    if let Some(last) = useful.last() {
        return Some((*last).to_string());
    }
    Some(buf.trim().to_string())
}

struct RotatingCoreWriter {
    log_dir: PathBuf,
    file_hour: Option<u64>,
    file: Option<File>,
    file_bytes: u64,
    bytes_since_cleanup: u64,
}

impl RotatingCoreWriter {
    fn new(log_dir: PathBuf) -> Self {
        Self {
            log_dir,
            file_hour: None,
            file: None,
            file_bytes: 0,
            bytes_since_cleanup: 0,
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        let hour = crate::log_retention::current_hour();
        if self.file_hour != Some(hour) || self.file.is_none() {
            let path = crate::log_retention::hourly_path_for(&self.log_dir, "sing-box", hour);
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(opened) => {
                    self.file_bytes = opened.metadata().map(|m| m.len()).unwrap_or(0);
                    self.bytes_since_cleanup = 0;
                    self.file = Some(opened);
                    self.file_hour = Some(hour);
                    let _ = crate::log_retention::cleanup_current_hour(&self.log_dir);
                }
                Err(error) => {
                    crate::app_log::error("core_log", format!("open {}: {error}", path.display()));
                    self.file = None;
                    self.file_hour = None;
                    self.file_bytes = 0;
                    return;
                }
            }
        }
        if self.file_bytes >= crate::log_retention::CORE_ACTIVE_MAX_BYTES {
            return;
        }
        let allowed = crate::log_retention::CORE_ACTIVE_MAX_BYTES - self.file_bytes;
        let write_count = bytes.len().min(allowed as usize);
        let Some(active) = self.file.as_mut() else {
            return;
        };
        if let Err(error) = active
            .write_all(&bytes[..write_count])
            .and_then(|_| active.flush())
        {
            crate::app_log::error("core_log", format!("write: {error}"));
            self.file = None;
            self.file_hour = None;
            self.file_bytes = 0;
            return;
        }
        self.file_bytes = self.file_bytes.saturating_add(write_count as u64);
        self.bytes_since_cleanup = self.bytes_since_cleanup.saturating_add(write_count as u64);
        if self.bytes_since_cleanup >= 1024 * 1024 {
            let _ = crate::log_retention::cleanup_current_hour(&self.log_dir);
            self.bytes_since_cleanup = 0;
        }
    }
}

/// Copy one core output stream into the active hourly file. Reading in chunks
/// avoids blocking sing-box on a full pipe; each chunk is flushed so a crash
/// still leaves a useful final log tail.
fn spawn_rotating_log_copy<R>(
    mut reader: R,
    writer: std::sync::Arc<std::sync::Mutex<RotatingCoreWriter>>,
) where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let count = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) => {
                    crate::app_log::warn("core_log", format!("read core output: {error}"));
                    break;
                }
            };
            writer
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .write(&buffer[..count]);
        }
    });
}

fn strip_ansi(s: &str) -> String {
    // remove simple ANSI color sequences
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for x in chars.by_ref() {
                    if x.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}
