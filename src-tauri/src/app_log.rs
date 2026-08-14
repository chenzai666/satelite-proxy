//! In-process application log ring for the Logs UI tab.
//! Thread-safe, persisted to hourly files while retaining the in-memory UI ring.

use serde::Serialize;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl LogLevel {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "trace" => Some(Self::Trace),
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" | "warning" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub id: u64,
    pub ts_ms: i64,
    pub level: LogLevel,
    pub target: String,
    pub message: String,
}

struct LogRing {
    next_id: u64,
    entries: VecDeque<LogEntry>,
    log_dir: Option<PathBuf>,
    file_hour: Option<u64>,
    file: Option<File>,
    file_bytes: u64,
}

impl LogRing {
    fn new() -> Self {
        Self {
            next_id: 1,
            entries: VecDeque::with_capacity(256),
            log_dir: None,
            file_hour: None,
            file: None,
            file_bytes: 0,
        }
    }

    fn push(&mut self, level: LogLevel, target: impl Into<String>, message: impl Into<String>) {
        let target = target.into();
        let message = message.into();
        let entry = LogEntry {
            id: self.next_id,
            ts_ms: now_ms(),
            level,
            target,
            message,
        };
        self.next_id = self.next_id.saturating_add(1);
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.persist(&entry);
        self.entries.push_back(entry);
    }

    fn persist(&mut self, entry: &LogEntry) {
        let Some(dir) = self.log_dir.clone() else {
            return;
        };
        let hour = crate::log_retention::current_hour();
        if self.file_hour != Some(hour) || self.file.is_none() {
            let path = crate::log_retention::hourly_path_for(&dir, "app", hour);
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(file) => {
                    self.file_bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
                    self.file = Some(file);
                    self.file_hour = Some(hour);
                }
                Err(error) => {
                    self.file = None;
                    self.file_hour = None;
                    eprintln!(
                        "[satelite][error][app_log] open {}: {error}",
                        path.display()
                    );
                    return;
                }
            }
            let _ = crate::log_retention::cleanup_current_hour(&dir);
        }
        let Some(file) = self.file.as_mut() else {
            return;
        };
        let message = entry.message.replace('\r', "").replace('\n', "\\n");
        let line = format!(
            "{} [{}] [{}] {}\n",
            entry.ts_ms,
            entry.level.as_str(),
            entry.target,
            message
        );
        let line_bytes = line.len() as u64;
        if self.file_bytes.saturating_add(line_bytes) > crate::log_retention::APP_ACTIVE_MAX_BYTES {
            return;
        }
        if let Err(error) = file.write_all(line.as_bytes()).and_then(|_| file.flush()) {
            eprintln!("[satelite][error][app_log] write: {error}");
            self.file = None;
            self.file_hour = None;
            self.file_bytes = 0;
        } else {
            self.file_bytes = self.file_bytes.saturating_add(line_bytes);
        }
    }

    fn list(&self, min_level: LogLevel, limit: usize, query: Option<&str>) -> Vec<LogEntry> {
        let q = query
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());
        let mut out: Vec<LogEntry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| e.level >= min_level)
            .filter(|e| {
                let Some(q) = q.as_ref() else {
                    return true;
                };
                e.message.to_ascii_lowercase().contains(q)
                    || e.target.to_ascii_lowercase().contains(q)
            })
            .take(limit.max(1))
            .cloned()
            .collect();
        out.reverse();
        out
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

static RING: LazyLock<Mutex<LogRing>> = LazyLock::new(|| Mutex::new(LogRing::new()));

fn lock_ring() -> std::sync::MutexGuard<'static, LogRing> {
    RING.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn init(log_dir: PathBuf) {
    let _ = std::fs::create_dir_all(&log_dir);
    let mut ring = lock_ring();
    ring.log_dir = Some(log_dir.clone());
    ring.file_hour = None;
    ring.file = None;
    ring.file_bytes = 0;
    let _ = crate::log_retention::cleanup_current_hour(&log_dir);
}

pub fn push(level: LogLevel, target: impl Into<String>, message: impl Into<String>) {
    let target = target.into();
    let message = message.into();
    // Mirror to stderr for dev / Console.app
    eprintln!("[satelite][{}][{}] {}", level.as_str(), target, message);
    lock_ring().push(level, target, message);
}

pub fn list(min_level: LogLevel, limit: usize, query: Option<&str>) -> Vec<LogEntry> {
    lock_ring().list(min_level, limit, query)
}

pub fn clear() {
    lock_ring().clear();
}

pub fn info(target: &str, message: impl Into<String>) {
    push(LogLevel::Info, target, message);
}

pub fn warn(target: &str, message: impl Into<String>) {
    push(LogLevel::Warn, target, message);
}

pub fn error(target: &str, message: impl Into<String>) {
    push(LogLevel::Error, target, message);
}

pub fn debug(target: &str, message: impl Into<String>) {
    push(LogLevel::Debug, target, message);
}

pub fn trace(target: &str, message: impl Into<String>) {
    push(LogLevel::Trace, target, message);
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_log_is_immediately_visible_on_disk() {
        let dir = std::env::temp_dir().join(format!(
            "satelite-app-log-{}-{}",
            std::process::id(),
            crate::log_retention::current_hour()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut ring = LogRing::new();
        ring.log_dir = Some(dir.clone());
        ring.push(LogLevel::Info, "test", "persist-now");
        let path = crate::log_retention::hourly_path(&dir, "app");
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("[info] [test] persist-now"));
        drop(ring);
        let _ = std::fs::remove_dir_all(dir);
    }
}
