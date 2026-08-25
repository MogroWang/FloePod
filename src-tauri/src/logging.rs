use std::io::Write;
use std::sync::Mutex;

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MESSAGE_CHARS: usize = 16 * 1024;
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// Release builds have no console. Keep one bounded local diagnostic file in
/// the selected data directory without allowing logging failures to affect app work.
pub fn write(message: &str) {
    let _guard = LOG_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let directory = crate::paths::resolve();
    let _ = std::fs::create_dir_all(&directory);
    let log = directory.join("debug.log");
    if log.metadata().map(|metadata| metadata.len()).unwrap_or(0) >= MAX_BYTES {
        let rotated = directory.join("debug.log.1");
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(&log, &rotated);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
    {
        let bounded: String = message.chars().take(MAX_MESSAGE_CHARS).collect();
        let _ = writeln!(file, "{bounded}");
    }
    eprintln!("{message}");
}
