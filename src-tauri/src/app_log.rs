//! In-process application log ring for the Logs UI tab.
//! Thread-safe, no I/O on the hot path beyond a short mutex hold.

use serde::Serialize;
use std::collections::VecDeque;
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
}

impl LogRing {
    fn new() -> Self {
        Self {
            next_id: 1,
            entries: VecDeque::with_capacity(256),
        }
    }

    fn push(&mut self, level: LogLevel, target: impl Into<String>, message: impl Into<String>) {
        let entry = LogEntry {
            id: self.next_id,
            ts_ms: now_ms(),
            level,
            target: target.into(),
            message: message.into(),
        };
        self.next_id = self.next_id.saturating_add(1);
        if self.entries.len() >= MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
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
