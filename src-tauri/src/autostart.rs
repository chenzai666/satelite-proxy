//! OS login item / launch-at-login helpers.

use crate::error::{AppError, AppResult};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "macos")]
const LAUNCH_AGENT_LABEL: &str = "com.satelite.proxy";

fn current_exe() -> AppResult<PathBuf> {
    std::env::current_exe().map_err(|e| AppError::Core(format!("current_exe: {e}")))
}

#[cfg(target_os = "macos")]
fn launch_agent_path() -> AppResult<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| AppError::Core("HOME unset".into()))?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist")))
}

/// Enable or disable launch at login for the current executable.
pub fn set_launch_at_login(enabled: bool) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let plist = launch_agent_path()?;
        if enabled {
            let exe = current_exe()?;
            let exe_s = exe.to_string_lossy();
            // Escape for XML
            let exe_xml = exe_s
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;");
            let body = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCH_AGENT_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe_xml}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
</dict>
</plist>
"#
            );
            if let Some(parent) = plist.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&plist, body)?;
            // Best-effort load (ignore if already loaded / GUI session rules)
            let _ = Command::new("launchctl")
                .args(["unload", "-w"])
                .arg(&plist)
                .output();
            let _ = Command::new("launchctl")
                .args(["load", "-w"])
                .arg(&plist)
                .output();
            Ok(())
        } else {
            if plist.is_file() {
                let _ = Command::new("launchctl")
                    .args(["unload", "-w"])
                    .arg(&plist)
                    .output();
                let _ = fs::remove_file(&plist);
            }
            Ok(())
        }
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var_os("HOME").ok_or_else(|| AppError::Core("HOME unset".into()))?;
        let desktop = PathBuf::from(home)
            .join(".config/autostart")
            .join("satelite-proxy.desktop");
        if enabled {
            let exe = current_exe()?;
            let body = format!(
                "[Desktop Entry]\nType=Application\nName=Satelite\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\n",
                exe.display()
            );
            if let Some(parent) = desktop.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&desktop, body)?;
            Ok(())
        } else {
            let _ = fs::remove_file(&desktop);
            Ok(())
        }
    }
    #[cfg(target_os = "windows")]
    {
        // Registry Run key via reg.exe
        let exe = current_exe()?;
        if enabled {
            let mut cmd = Command::new("reg");
            cmd.args([
                "add",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "SateliteProxy",
                "/t",
                "REG_SZ",
                "/d",
                &format!("\"{}\"", exe.display()),
                "/f",
            ]);
            #[cfg(target_os = "windows")]
            cmd.creation_flags(CREATE_NO_WINDOW);
            let status = cmd
                .status()
                .map_err(|e| AppError::Core(format!("reg: {e}")))?;
            if !status.success() {
                return Err(AppError::Core("failed to set Windows autostart".into()));
            }
            Ok(())
        } else {
            let mut cmd = Command::new("reg");
            cmd.args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "SateliteProxy",
                "/f",
            ]);
            #[cfg(target_os = "windows")]
            cmd.creation_flags(CREATE_NO_WINDOW);
            let _ = cmd.status();
            Ok(())
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = enabled;
        Err(AppError::Core(
            "autostart unsupported on this platform".into(),
        ))
    }
}
