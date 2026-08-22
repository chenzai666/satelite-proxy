//! In-process application log ring for the Logs UI tab.
//! Thread-safe, persisted to hourly files while retaining the in-memory UI ring.

use serde::Serialize;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{LazyLock, Mutex, Once, OnceLock};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 2_000;
const PERSIST_QUEUE_CAPACITY: usize = 2_048;
const PERSIST_ACK_TIMEOUT: Duration = Duration::from_secs(2);

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

#[derive(Debug, Serialize)]
pub struct LogBatch {
    pub entries: Vec<LogEntry>,
    pub cursor: u64,
    /// Changes whenever the current diagnostic session is cleared. The UI uses
    /// this to discard rows that belonged to the previous proxy run.
    pub session: u64,
}

struct LogRing {
    next_id: u64,
    session: u64,
    entries: VecDeque<LogEntry>,
}

struct LogSink {
    log_dir: Option<PathBuf>,
    file_hour: Option<u64>,
    file: Option<File>,
    file_bytes: u64,
}

impl LogRing {
    fn new() -> Self {
        Self {
            next_id: 1,
            session: 1,
            entries: VecDeque::with_capacity(256),
        }
    }

    fn push(&mut self, level: LogLevel, target: String, message: String) -> LogEntry {
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
        self.entries.push_back(entry.clone());
        entry
    }

    fn list(
        &self,
        min_level: LogLevel,
        limit: usize,
        query: Option<&str>,
        after_id: Option<u64>,
    ) -> LogBatch {
        let q = query
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());
        let matches = |entry: &&LogEntry| {
            entry.level >= min_level && {
                let Some(q) = q.as_ref() else {
                    return true;
                };
                entry.message.to_ascii_lowercase().contains(q)
                    || entry.target.to_ascii_lowercase().contains(q)
            }
        };
        let limit = limit.max(1);
        let (entries, cursor) = if let Some(after_id) = after_id {
            let mut entries = Vec::new();
            let mut cursor = after_id;
            for entry in self.entries.iter().filter(|entry| entry.id > after_id) {
                cursor = entry.id;
                if matches(&entry) {
                    entries.push(entry.clone());
                    if entries.len() >= limit {
                        break;
                    }
                }
            }
            (entries, cursor)
        } else {
            let mut entries: Vec<LogEntry> = self
                .entries
                .iter()
                .rev()
                .filter(matches)
                .take(limit)
                .cloned()
                .collect();
            entries.reverse();
            (entries, self.next_id.saturating_sub(1))
        };
        LogBatch {
            entries,
            cursor,
            session: self.session,
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.session = self.session.saturating_add(1);
    }
}

impl LogSink {
    fn new() -> Self {
        Self {
            log_dir: None,
            file_hour: None,
            file: None,
            file_bytes: 0,
        }
    }

    fn configure(&mut self, log_dir: PathBuf) {
        self.log_dir = Some(log_dir.clone());
        self.file_hour = None;
        self.file = None;
        self.file_bytes = 0;
        let _ = crate::log_retention::cleanup_current_hour(&log_dir);
    }

    /// Remove only Satelite's internal application logs. sing-box output and
    /// panic diagnostics are intentionally retained because they have separate
    /// lifecycle and forensic value.
    fn clear_app_files(&mut self) -> Result<(), String> {
        self.file = None;
        self.file_hour = None;
        self.file_bytes = 0;

        let Some(dir) = self.log_dir.as_ref() else {
            return Ok(());
        };
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("read log directory: {error}")),
        };
        for entry in entries {
            let entry = entry.map_err(|error| format!("read log entry: {error}"))?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("inspect log entry: {error}"))?;
            if !file_type.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("app-") && name.ends_with(".log") {
                std::fs::remove_file(entry.path())
                    .map_err(|error| format!("remove {name}: {error}"))?;
            }
        }
        Ok(())
    }

    fn persist_entry(&mut self, entry: &LogEntry) {
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
}

static RING: LazyLock<Mutex<LogRing>> = LazyLock::new(|| Mutex::new(LogRing::new()));
/// Serializes a ring mutation with its matching writer command. Without this,
/// an entry from the old session could be queued after the clear command.
static LOG_OPERATION: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
static PANIC_HOOK: Once = Once::new();

enum WriterMessage {
    Configure(PathBuf, mpsc::Sender<()>),
    Entry(LogEntry, Option<mpsc::Sender<()>>),
    Flush(mpsc::Sender<()>),
    Clear(mpsc::Sender<Result<(), String>>),
}

static WRITER: LazyLock<SyncSender<WriterMessage>> = LazyLock::new(|| {
    let (tx, rx) = mpsc::sync_channel(PERSIST_QUEUE_CAPACITY);
    std::thread::Builder::new()
        .name("satelite-log-writer".into())
        .spawn(move || writer_loop(rx))
        .expect("spawn app log writer");
    tx
});

fn writer_loop(rx: Receiver<WriterMessage>) {
    let mut sink = LogSink::new();
    while let Ok(message) = rx.recv() {
        match message {
            WriterMessage::Configure(dir, ack) => {
                sink.configure(dir);
                let _ = ack.send(());
            }
            WriterMessage::Entry(entry, ack) => {
                sink.persist_entry(&entry);
                if let Some(ack) = ack {
                    let _ = ack.send(());
                }
            }
            WriterMessage::Flush(ack) => {
                if let Some(file) = sink.file.as_mut() {
                    let _ = file.flush();
                }
                let _ = ack.send(());
            }
            WriterMessage::Clear(ack) => {
                let _ = ack.send(sink.clear_app_files());
            }
        }
    }
}

fn lock_ring() -> std::sync::MutexGuard<'static, LogRing> {
    RING.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn init(log_dir: PathBuf) {
    let _ = std::fs::create_dir_all(&log_dir);
    let _ = LOG_DIR.set(log_dir.clone());
    let (ack_tx, ack_rx) = mpsc::channel();
    if send_writer_bounded(WriterMessage::Configure(log_dir, ack_tx)) {
        let _ = ack_rx.recv_timeout(PERSIST_ACK_TIMEOUT);
    }
    // Each process starts with a fresh diagnostic session. A crash may leave
    // files behind, so startup cleanup complements graceful-exit cleanup.
    if let Err(error) = clear() {
        eprintln!("[satelite][warn][app_log] startup clear failed: {error}");
    }
}

/// Persist panic details without touching the in-memory log mutex. A panic may
/// have started while that mutex was held, so the regular logger could deadlock.
pub fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            if let Some(log_dir) = LOG_DIR.get() {
                let location = panic_info
                    .location()
                    .map(|location| {
                        format!(
                            "{}:{}:{}",
                            location.file(),
                            location.line(),
                            location.column()
                        )
                    })
                    .unwrap_or_else(|| "unknown".into());
                let payload = panic_info
                    .payload()
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| {
                        panic_info
                            .payload()
                            .downcast_ref::<String>()
                            .map(String::as_str)
                    })
                    .unwrap_or("non-string panic payload");
                if let Ok(mut file) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log_dir.join("panic.log"))
                {
                    let _ = writeln!(
                        file,
                        "{} [panic] [{}] {}\n{}",
                        now_ms(),
                        location,
                        payload,
                        std::backtrace::Backtrace::force_capture()
                    );
                    let _ = file.flush();
                }
            }
            previous(panic_info);
        }));
    });
}

pub fn push(level: LogLevel, target: impl Into<String>, message: impl Into<String>) {
    let _operation = LOG_OPERATION.lock().unwrap_or_else(|p| p.into_inner());
    let target = target.into();
    let message = message.into();
    // Mirror to stderr for dev / Console.app
    eprintln!("[satelite][{}][{}] {}", level.as_str(), target, message);
    let entry = lock_ring().push(level, target, message);
    enqueue_persist(entry);
}

fn enqueue_persist(entry: LogEntry) {
    if entry.level >= LogLevel::Warn {
        let (ack_tx, ack_rx) = mpsc::channel();
        if send_writer_bounded(WriterMessage::Entry(entry, Some(ack_tx))) {
            let _ = ack_rx.recv_timeout(PERSIST_ACK_TIMEOUT);
        }
        return;
    }

    let message = WriterMessage::Entry(entry, None);
    match WRITER.try_send(message) {
        Ok(()) => {}
        // Persistence is intentionally lossy for non-critical logs once the
        // bounded queue is full. The in-memory UI ring still keeps the entry.
        Err(TrySendError::Full(_)) => {}
        Err(TrySendError::Disconnected(_)) => {}
    }
}

fn send_writer_bounded(mut message: WriterMessage) -> bool {
    let started = Instant::now();
    loop {
        match WRITER.try_send(message) {
            Ok(()) => return true,
            Err(TrySendError::Full(returned)) => {
                if started.elapsed() >= PERSIST_ACK_TIMEOUT {
                    return false;
                }
                message = returned;
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(TrySendError::Disconnected(_)) => return false,
        }
    }
}

pub fn flush() {
    let (ack_tx, ack_rx) = mpsc::channel();
    if send_writer_bounded(WriterMessage::Flush(ack_tx)) {
        let _ = ack_rx.recv_timeout(PERSIST_ACK_TIMEOUT);
    }
}

pub fn list(
    min_level: LogLevel,
    limit: usize,
    query: Option<&str>,
    after_id: Option<u64>,
) -> LogBatch {
    lock_ring().list(min_level, limit, query, after_id)
}

pub fn clear() -> Result<(), String> {
    let _operation = LOG_OPERATION.lock().unwrap_or_else(|p| p.into_inner());
    lock_ring().clear();
    let (ack_tx, ack_rx) = mpsc::channel();
    if !send_writer_bounded(WriterMessage::Clear(ack_tx)) {
        return Err("log writer unavailable".into());
    }
    ack_rx
        .recv_timeout(PERSIST_ACK_TIMEOUT)
        .map_err(|_| "timed out while clearing persisted logs".to_string())?
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

    fn test_log_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "satelite-app-log-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ))
    }

    #[test]
    fn incremental_list_advances_cursor_past_filtered_entries() {
        let mut ring = LogRing::new();
        ring.push(LogLevel::Info, "core".into(), "ready".into());
        ring.push(LogLevel::Debug, "probe".into(), "hidden".into());
        let initial = ring.list(LogLevel::Info, 10, None, None);
        assert_eq!(initial.entries.len(), 1);
        assert_eq!(initial.cursor, 2);

        ring.push(LogLevel::Warn, "core".into(), "retry".into());
        let incremental = ring.list(LogLevel::Info, 10, None, Some(initial.cursor));
        assert_eq!(incremental.entries.len(), 1);
        assert_eq!(incremental.entries[0].message, "retry");
        assert_eq!(incremental.cursor, 3);
    }

    #[test]
    fn persisted_log_is_immediately_visible_on_disk() {
        let dir = test_log_dir("sink");
        std::fs::create_dir_all(&dir).unwrap();
        let mut sink = LogSink::new();
        sink.configure(dir.clone());
        sink.persist_entry(&LogEntry {
            id: 1,
            ts_ms: now_ms(),
            level: LogLevel::Info,
            target: "test".into(),
            message: "persist-now".into(),
        });
        let path = crate::log_retention::hourly_path(&dir, "app");
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("[info] [test] persist-now"));
        drop(sink);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn writer_ack_means_error_is_persisted() {
        let dir = test_log_dir("writer");
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = mpsc::sync_channel(8);
        let writer = std::thread::spawn(move || writer_loop(rx));

        let (configure_tx, configure_rx) = mpsc::channel();
        tx.send(WriterMessage::Configure(dir.clone(), configure_tx))
            .unwrap();
        configure_rx.recv_timeout(PERSIST_ACK_TIMEOUT).unwrap();

        let (entry_tx, entry_rx) = mpsc::channel();
        tx.send(WriterMessage::Entry(
            LogEntry {
                id: 1,
                ts_ms: now_ms(),
                level: LogLevel::Error,
                target: "test".into(),
                message: "persist-before-ack".into(),
            },
            Some(entry_tx),
        ))
        .unwrap();
        entry_rx.recv_timeout(PERSIST_ACK_TIMEOUT).unwrap();

        let path = crate::log_retention::hourly_path(&dir, "app");
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("[error] [test] persist-before-ack"));
        drop(tx);
        writer.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_files_removes_only_internal_app_logs() {
        let dir = test_log_dir("clear");
        std::fs::create_dir_all(&dir).unwrap();
        let mut sink = LogSink::new();
        sink.configure(dir.clone());
        sink.persist_entry(&LogEntry {
            id: 1,
            ts_ms: now_ms(),
            level: LogLevel::Info,
            target: "test".into(),
            message: "before-clear".into(),
        });
        let app_path = crate::log_retention::hourly_path(&dir, "app");
        let core_path = crate::log_retention::hourly_path(&dir, "sing-box");
        let panic_path = dir.join("panic.log");
        File::create(&core_path).unwrap();
        File::create(&panic_path).unwrap();

        sink.clear_app_files().unwrap();

        assert!(!app_path.exists());
        assert!(core_path.exists());
        assert!(panic_path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
