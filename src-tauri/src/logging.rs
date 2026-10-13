//! Tiny rolling log file (`<data dir>/logs/kura.log`).
//!
//! Release builds on Windows have no console, so `eprintln!` output is lost. Everything logged
//! through [`klog!`](crate::klog) goes to stderr *and* this file, which the Diagnostics panel
//! can show and copy. The file rotates to `kura.old.log` once it grows past [`MAX_BYTES`].

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: u64 = 1024 * 1024;

struct Sink {
    path: PathBuf,
    file: Option<File>,
}

static SINK: OnceLock<Mutex<Sink>> = OnceLock::new();
static WRITES: AtomicU32 = AtomicU32::new(0);

pub fn log_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

pub fn log_path(data_dir: &Path) -> PathBuf {
    log_dir(data_dir).join("kura.log")
}

fn rotate_if_big(path: &Path) {
    if std::fs::metadata(path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(path, path.with_file_name("kura.old.log"));
    }
}

fn open(path: &Path) -> Option<File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Start logging to the data dir and record panics. Safe to call once at startup.
pub fn init(data_dir: &Path) {
    let path = log_path(data_dir);
    let _ = std::fs::create_dir_all(log_dir(data_dir));
    rotate_if_big(&path);
    let file = open(&path);
    let _ = SINK.set(Mutex::new(Sink { path, file }));

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write(&format!("PANIC: {info}"));
        prev(info);
    }));
}

/// Append one line (timestamped). Also echoed to stderr.
pub fn write(msg: &str) {
    eprintln!("{msg}");
    let Some(sink) = SINK.get() else { return };
    let Ok(mut s) = sink.lock() else { return };
    // Check the size now and then, not on every line.
    if WRITES.fetch_add(1, Ordering::Relaxed) % 200 == 199 {
        s.file = None;
        rotate_if_big(&s.path);
    }
    if s.file.is_none() {
        s.file = open(&s.path);
    }
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Some(f) = s.file.as_mut() {
        let _ = writeln!(f, "{ts} {msg}");
    }
}

/// Last `max_lines` lines of the current log (oldest first).
pub fn tail(data_dir: &Path, max_lines: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(log_path(data_dir)) else { return Vec::new() };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].iter().map(|l| l.to_string()).collect()
}

/// Log like `eprintln!`, but also into the rolling log file.
#[macro_export]
macro_rules! klog {
    ($($arg:tt)*) => {
        $crate::logging::write(&format!($($arg)*))
    };
}
