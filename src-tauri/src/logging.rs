//! Structured logging to a daily-rotating file in the app log directory (7 files kept).

use std::path::{Path, PathBuf};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, reload, EnvFilter, Registry};

pub const LOG_PREFIX: &str = "kittytooth";

pub struct Logging {
    pub dir: PathBuf,
    _guard: Option<WorkerGuard>,
    filter: reload::Handle<EnvFilter, Registry>,
}

fn filter(debug: bool) -> EnvFilter {
    if debug {
        EnvFilter::new("info,kittytooth=debug,kittytooth_lib=debug,kittytooth_core=debug")
    } else {
        EnvFilter::new("info,mdns_sd=warn,btleplug=warn")
    }
}

impl Logging {
    pub fn init(dir: &Path, debug: bool) -> Logging {
        let _ = std::fs::create_dir_all(dir);
        let (filter_layer, handle) = reload::Layer::new(filter(debug));
        let appender = Builder::new()
            .rotation(Rotation::DAILY)
            .filename_prefix(LOG_PREFIX)
            .filename_suffix("log")
            .max_log_files(7)
            .build(dir)
            .ok();
        let mut guard = None;
        let file_layer = appender.map(|a| {
            let (nb, g) = tracing_appender::non_blocking(a);
            guard = Some(g);
            fmt::layer().with_writer(nb).with_ansi(false).with_target(true)
        });
        let stderr_layer = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));
        let _ = tracing_subscriber::registry()
            .with(filter_layer)
            .with(file_layer)
            .with(stderr_layer)
            .try_init();
        std::panic::set_hook(Box::new(|info| {
            tracing::error!(panic = %info, "unexpected internal error");
        }));
        Logging {
            dir: dir.to_path_buf(),
            _guard: guard,
            filter: handle,
        }
    }

    pub fn set_debug(&self, debug: bool) {
        let _ = self.filter.reload(filter(debug));
    }

    /// Last `n` lines of the newest log file.
    pub fn tail(&self, n: usize) -> String {
        let newest = std::fs::read_dir(&self.dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(LOG_PREFIX))
            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
        let Some(entry) = newest else {
            return "(no log file yet)".into();
        };
        match std::fs::read_to_string(entry.path()) {
            Ok(text) => {
                let lines: Vec<&str> = text.lines().collect();
                lines[lines.len().saturating_sub(n)..].join("\n")
            }
            Err(e) => format!("(could not read log: {e})"),
        }
    }
}
