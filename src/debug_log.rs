use crate::config::DEBUG_LOGGER_ENABLED;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn init_log() {
    if !DEBUG_LOGGER_ENABLED {
        return;
    }
    if let Ok(home) = std::env::var("HOME") {
        let dir = format!("{home}/.cache/se-ai-translator");
        let _ = std::fs::create_dir_all(&dir);
        let path = format!("{dir}/debug.log");
        if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
            *LOG_FILE.lock().unwrap() = Some(file);
        }
    }
}

pub fn log(msg: &str) {
    if !DEBUG_LOGGER_ENABLED {
        return;
    }
    if let Ok(mut guard) = LOG_FILE.lock()
        && let Some(ref mut file) = *guard {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let _ = writeln!(file, "[{ts}] {msg}");
            let _ = file.flush();
        }
}

#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if $crate::config::DEBUG_LOGGER_ENABLED {
            $crate::debug_log::log(&format!($($arg)*))
        }
    };
}
