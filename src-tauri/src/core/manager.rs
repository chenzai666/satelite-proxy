//! sing-box process lifecycle.

use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
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

#[derive(Debug)]
pub struct CoreManager {
    child: Option<Child>,
    /// When TUN needs root on macOS we spawn via osascript and only keep the PID.
    elevated_pid: Option<u32>,
    state: CoreState,
    last_error: Option<String>,
    config_path: Option<PathBuf>,
    binary_path: Option<PathBuf>,
    log_path: Option<PathBuf>,
}

impl Default for CoreManager {
    fn default() -> Self {
        Self {
            child: None,
            elevated_pid: None,
            state: CoreState::Stopped,
            last_error: None,
            config_path: None,
            binary_path: None,
            log_path: None,
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

    pub fn is_running(&self) -> bool {
        matches!(self.state, CoreState::Running)
    }

    /// Reap child if it exited; update state with log tail when possible.
    pub fn poll(&mut self) {
        if let Some(pid) = self.elevated_pid {
            if !pid_alive(pid) {
                self.elevated_pid = None;
                if self.state == CoreState::Stopping {
                    self.state = CoreState::Stopped;
                } else if self.state != CoreState::Stopped {
                    self.state = CoreState::Error;
                    let detail = self
                        .log_path
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
                    if self.state == CoreState::Stopping {
                        self.state = CoreState::Stopped;
                    } else {
                        self.state = CoreState::Error;
                        let detail = self
                            .log_path
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

    /// Force-free a TCP listen port: kill processes holding it.
    pub fn force_free_port(port: u16) -> AppResult<()> {
        if Self::is_port_free(port) {
            return Ok(());
        }
        let killed = kill_listeners_on_port(port);
        // brief wait for OS to release
        for _ in 0..20 {
            if Self::is_port_free(port) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if Self::is_port_free(port) {
            Ok(())
        } else {
            let manual = if cfg!(windows) {
                format!("netstat -ano | findstr :{port}")
            } else {
                format!("lsof -iTCP:{port} -sTCP:LISTEN")
            };
            Err(AppError::Core(format!(
                "端口 {port} 仍被占用（已尝试结束监听进程: {killed}）。可手动: {manual}"
            )))
        }
    }

    /// Ensure mixed + API ports are free (kill leftovers from previous runs).
    pub fn ensure_ports_free(ports: &[u16]) -> AppResult<()> {
        for &p in ports {
            if p == 0 {
                continue;
            }
            Self::force_free_port(p)?;
        }
        Ok(())
    }

    /// Start core. When `elevated` is true (TUN on macOS), prompts for admin and runs as root.
    pub fn start_with_ports(
        &mut self,
        binary: &Path,
        config: &Path,
        log_dir: &Path,
        mixed_port: u16,
        api_port: Option<u16>,
        elevated: bool,
    ) -> AppResult<()> {
        self.poll();
        if matches!(self.state, CoreState::Running | CoreState::Starting) {
            return Ok(());
        }

        // Drop our own child first if still tracked, then free ports aggressively.
        let _ = self.stop();
        let mut ports = vec![mixed_port];
        if let Some(api) = api_port {
            if api != mixed_port {
                ports.push(api);
            }
        }
        Self::ensure_ports_free(&ports)?;

        Self::check_config(binary, config)?;
        // Re-check after kill (should be free)
        Self::ensure_ports_free(&ports)?;

        fs::create_dir_all(log_dir)
            .map_err(|e| AppError::Core(format!("create log dir: {e}")))?;
        let log_path = log_dir.join("sing-box.log");
        // truncate previous run log
        let _ = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)
            .map_err(|e| AppError::Core(format!("open log: {e}")))?;

        self.state = CoreState::Starting;
        self.last_error = None;
        self.log_path = Some(log_path.clone());
        self.config_path = Some(config.to_path_buf());
        self.binary_path = Some(binary.to_path_buf());
        self.elevated_pid = None;
        self.child = None;

        #[cfg(target_os = "macos")]
        if elevated {
            return self.start_elevated_macos(binary, config, &log_path, mixed_port);
        }

        #[cfg(target_os = "windows")]
        if elevated {
            return self.start_elevated_windows(binary, config, &log_path, mixed_port);
        }

        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| AppError::Core(format!("open log: {e}")))?;
        let log_err = log_file
            .try_clone()
            .map_err(|e| AppError::Core(format!("clone log: {e}")))?;

        let mut cmd = Command::new(binary);
        cmd.args(["run", "-c"]).arg(config);
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let child = cmd
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_err))
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

        self.child = Some(child);

        self.wait_until_ready(mixed_port, false)
    }

    #[cfg(target_os = "macos")]
    fn start_elevated_macos(
        &mut self,
        binary: &Path,
        config: &Path,
        log_path: &Path,
        mixed_port: u16,
    ) -> AppResult<()> {
        // TUN needs root to create utun / install routes. Prompt via macOS auth dialog.
        let bin_q = shell_single_quote(&binary.to_string_lossy());
        let cfg_q = shell_single_quote(&config.to_string_lossy());
        let log_q = shell_single_quote(&log_path.to_string_lossy());
        // Background + print PID. Do NOT use nohup: under `osascript` / do shell script
        // there is no console TTY → "nohup: can't detach from console: Inappropriate ioctl".
        let shell = format!("{bin_q} run -c {cfg_q} </dev/null >>{log_q} 2>&1 & echo $!");
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            escape_applescript_string(&shell)
        );

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| {
                self.state = CoreState::Error;
                let msg = format!("请求管理员权限失败: {e}");
                self.last_error = Some(msg.clone());
                AppError::Core(msg)
            })?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            let out = String::from_utf8_lossy(&output.stdout);
            let raw = format!("{err}{out}").trim().to_string();
            let msg = if raw.contains("User canceled")
                || raw.contains("(-128)")
                || raw.contains("-128")
            {
                "已取消管理员授权。TUN 模式需要管理员权限以创建虚拟网卡。".into()
            } else if raw.is_empty() {
                "管理员授权失败，无法以 root 启动 sing-box（TUN 必需）。".into()
            } else {
                format!("管理员授权失败: {raw}")
            };
            self.state = CoreState::Error;
            self.last_error = Some(msg.clone());
            return Err(AppError::Core(msg));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let pid: u32 = stdout
            .lines()
            .rev()
            .find_map(|l| l.trim().parse().ok())
            .ok_or_else(|| {
                self.state = CoreState::Error;
                let msg = format!("无法解析提权进程 PID，输出: {}", stdout.trim());
                self.last_error = Some(msg.clone());
                AppError::Core(msg)
            })?;

        self.elevated_pid = Some(pid);
        self.wait_until_ready(mixed_port, true)
    }

    /// Start sing-box elevated via UAC (Windows). Needed for TUN to create the
    /// virtual adapter. stdout/stderr are appended to `log_path` directly.
    #[cfg(target_os = "windows")]
    fn start_elevated_windows(
        &mut self,
        binary: &Path,
        config: &Path,
        log_path: &Path,
        mixed_port: u16,
    ) -> AppResult<()> {
        // sing-box redirects its own stdout/stderr when given 2>&1 >>file in the
        // args, so we pass those flags. Quote paths defensively (spaces).
        let bin_s = binary.display().to_string();
        let cfg_s = config.display().to_string();
        let log_s = log_path.display().to_string();
        let args = format!(
            "run -c \"{cfg_s}\" >>\"{log_s}\" 2>&1"
        );

        let _elevated = match super::elevate::run_elevated(Path::new(&bin_s), &args, None) {
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
        self.wait_until_ready(mixed_port, true)
    }

    fn wait_until_ready(&mut self, mixed_port: u16, elevated: bool) -> AppResult<()> {
        // wait a bit for immediate FATAL
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(100));
            self.poll();
            let gone = if elevated {
                self.elevated_pid.is_none()
            } else {
                self.child.is_none()
            };
            if gone {
                let err = self
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "process exited immediately".into());
                self.state = CoreState::Error;
                return Err(AppError::Core(map_tun_permission_hint(&err)));
            }
            if !Self::is_port_free(mixed_port) {
                break;
            }
        }

        self.poll();
        let gone = if elevated {
            self.elevated_pid.is_none()
        } else {
            self.child.is_none()
        };
        if gone {
            let err = self
                .last_error
                .clone()
                .unwrap_or_else(|| "process exited immediately".into());
            self.state = CoreState::Error;
            return Err(AppError::Core(map_tun_permission_hint(&err)));
        }

        self.state = CoreState::Running;
        Ok(())
    }

    pub fn stop(&mut self) -> AppResult<()> {
        self.poll();

        if let Some(pid) = self.elevated_pid.take() {
            self.state = CoreState::Stopping;
            elevated_kill_macos(pid);
            // wait for exit
            let deadline = std::time::Instant::now() + Duration::from_secs(4);
            while pid_alive(pid) && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(80));
            }
            if pid_alive(pid) {
                elevated_kill_macos_force(pid);
            }
            self.state = CoreState::Stopped;
            self.last_error = None;
            return Ok(());
        }

        let Some(mut child) = self.child.take() else {
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

        self.state = CoreState::Stopped;
        self.last_error = None;
        Ok(())
    }

    /// Hard-stop managed child and anything still holding our ports (orphans).
    pub fn force_shutdown(&mut self, ports: &[u16]) {
        let _ = self.stop();
        for &p in ports {
            if p != 0 {
                let _ = kill_listeners_on_port(p);
            }
        }
        self.state = CoreState::Stopped;
        self.child = None;
        self.elevated_pid = None;
        self.last_error = None;
    }
}

fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn escape_applescript_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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

/// Terminate an elevated sing-box process. macOS re-elevates via osascript to
/// keep root privileges for the kill; Windows uses taskkill (which itself runs
/// with the current user's rights — sufficient because the elevated child was
/// launched by this user and is killable by it despite running high).
fn elevated_kill_macos(pid: u32) {
    #[cfg(target_os = "macos")]
    {
        let shell = format!("kill -TERM {pid} 2>/dev/null || true");
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            escape_applescript_string(&shell)
        );
        let _ = Command::new("osascript").arg("-e").arg(&script).status();
    }
    #[cfg(target_os = "windows")]
    {
        // The elevated sing-box was launched by us via ShellExecuteEx, so we hold
        // PROCESS_TERMINATE access on it despite it running at a higher integrity
        // level. Use the direct Win32 call instead of taskkill — no UAC prompt,
        // and it actually works (plain taskkill fails on elevated children).
        let _ = super::elevate::terminate_pid(pid);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
}

fn elevated_kill_macos_force(pid: u32) {
    #[cfg(target_os = "macos")]
    {
        let shell = format!("kill -KILL {pid} 2>/dev/null || true");
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            escape_applescript_string(&shell)
        );
        let _ = Command::new("osascript").arg("-e").arg(&script).status();
    }
    #[cfg(target_os = "windows")]
    {
        // Same rationale as elevated_kill_macos: direct TerminateProcess via the
        // handle we're entitled to as the launching parent.
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
             macOS：开启 TUN 时应用会弹出管理员密码框并以 root 运行内核。\n\
             请确认已输入密码且未点「取消」；开发模式也可用：sudo \"path/to/sing-box\" run -c config.json"
        };
        format!("{err}\n\n{platform_hint}")
    } else {
        err.to_string()
    }
}

/// Kill PIDs listening on `port` (TCP LISTEN). Returns a short summary string.
fn kill_listeners_on_port(port: u16) -> String {
    #[cfg(unix)]
    {
        let out = Command::new("lsof")
            .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
            .output();
        let Ok(out) = out else {
            return "lsof 不可用".into();
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let mut pids: Vec<u32> = text
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect();
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
            // TERM then KILL
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
            std::thread::sleep(Duration::from_millis(80));
            // still alive?
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
