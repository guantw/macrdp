use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

static LOG_FILE: OnceLock<std::sync::Mutex<File>> = OnceLock::new();
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub struct LogBridgeLayer;

impl LogBridgeLayer {
    pub fn new() -> Self { Self }
}

/// Initialize the log file. Call once at startup.
pub fn init_log_file() -> PathBuf {
    let path = std::env::temp_dir().join("macrdp-logs.jsonl");
    // Truncate on startup
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .expect("failed to create log file");
    let _ = LOG_FILE.set(std::sync::Mutex::new(file));
    let _ = LOG_PATH.set(path.clone());
    path
}

/// Get the log file path (for UI to read).
pub fn log_file_path() -> Option<&'static PathBuf> {
    LOG_PATH.get()
}

impl<S: Subscriber> Layer<S> for LogBridgeLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let Some(file) = LOG_FILE.get() else { return };
        let Ok(mut file) = file.try_lock() else { return };

        let metadata = event.metadata();
        let level = metadata.level().as_str();

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let message = if visitor.message.is_empty() {
            metadata.name()
        } else {
            &visitor.message
        };

        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let _ = write_entry(&mut *file, level, message, &ts.to_string());
        let _ = file.flush();
    }
}

fn write_entry(writer: &mut impl Write, level: &str, message: &str, timestamp: &str) -> std::io::Result<()> {
    // Error chains contain literal newlines. Every event must remain one valid JSONL
    // line because the UI reads and parses the file one line at a time.
    serde_json::to_writer(&mut *writer, &serde_json::json!({
        "level": level.to_lowercase(),
        "message": message,
        "timestamp": timestamp,
    }))?;
    writeln!(writer)
}

#[cfg(test)]
mod tests {
    use super::write_entry;

    #[test]
    fn multiline_errors_survive_the_ui_line_parser() {
        let message = "Connection error\n\nCaused by:\n    No display found\r\n\t\"quoted\" \\ \u{0}";
        let mut bytes = Vec::new();
        write_entry(&mut bytes, "ERROR", message, "2026-10-27 12:00:00.000").unwrap();
        write_entry(&mut bytes, "WARN", "next event", "2026-10-27 12:00:01.000").unwrap();
        let content = String::from_utf8(bytes).unwrap();
        let entries: Vec<serde_json::Value> = content.lines()
            .map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["message"], message);
        assert_eq!(entries[0]["level"], "error");
        assert_eq!(entries[1]["message"], "next event");
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        } else if self.message.is_empty() {
            self.message = format!("{} = {:?}", field.name(), value);
        } else {
            self.message.push_str(&format!(" {} = {:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else if self.message.is_empty() {
            self.message = format!("{} = {}", field.name(), value);
        } else {
            self.message.push_str(&format!(" {} = {}", field.name(), value));
        }
    }
}
