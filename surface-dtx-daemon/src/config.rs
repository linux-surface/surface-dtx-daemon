use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};


const DEFAULT_CONFIG_PATH: &str = "/etc/surface-dtx/surface-dtx-daemon.conf";


#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Config {
    #[serde(skip)]
    pub dir: PathBuf,

    #[serde(default)]
    pub log: Log,

    #[serde(default)]
    pub handler: Handler,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Log {
    #[serde(default)]
    pub level: LogLevel,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all="lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Handler {
    #[serde(default)]
    pub detach: DetachHandler,

    #[serde(default)]
    pub detach_abort: DetachAbortHandler,

    #[serde(default)]
    pub attach: AttachHandler,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct DetachHandler {
    #[serde(default)]
    pub exec: Option<PathBuf>,

    #[serde(default="defaults::task_timeout")]
    pub timeout: f32,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct DetachAbortHandler {
    #[serde(default)]
    pub exec: Option<PathBuf>,

    #[serde(default="defaults::task_timeout")]
    pub timeout: f32,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct AttachHandler {
    #[serde(default)]
    pub exec: Option<PathBuf>,

    #[serde(default="defaults::task_timeout")]
    pub timeout: f32,

    #[serde(default="defaults::delay_attach")]
    pub delay: f32,
}


impl Config {
    pub fn load() -> Result<(Config, Diagnostics)> {
        if Path::new(DEFAULT_CONFIG_PATH).exists() {
            Config::load_file(DEFAULT_CONFIG_PATH)
        } else {
            Ok((Config::default(), Diagnostics::empty()))
        }
    }

    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<(Config, Diagnostics)> {
        use std::io::Read;

        let mut buf = Vec::new();
        let mut file = std::fs::File::open(path.as_ref())
            .context("Failed to open config file")?;

        file.read_to_end(&mut buf)
            .with_context(|| format!("Failed to read config file (path: {:?})", path.as_ref()))?;

        let data = std::str::from_utf8(&buf)
            .with_context(|| format!("Failed to read config file (path: {:?})", path.as_ref()))?;

        let de = toml::Deserializer::new(data);

        let mut unknowns = BTreeSet::new();
        let mut config: Config = serde_ignored::deserialize(de, |path| {
            unknowns.insert(path.to_string());
        }).with_context(|| format!("Failed to read config file (path: {:?})", path.as_ref()))?;

        config.dir = path.as_ref().parent().unwrap().into();

        let diag = Diagnostics {
            path: path.as_ref().into(),
            unknowns,
        };

        Ok((config, diag))
    }
}


pub struct Diagnostics {
    pub path: PathBuf,
    pub unknowns: BTreeSet<String>,
}

impl Diagnostics {
    fn empty() -> Self {
        Diagnostics {
            path: PathBuf::new(),
            unknowns: BTreeSet::new()
        }
    }

    pub fn log(&self) {
        let span = tracing::info_span!("config", file=?self.path);
        let _guard = span.enter();

        debug!(target: "sdtxd::config", "configuration loaded");
        for item in &self.unknowns {
            warn!(target: "sdtxd::config", item = %item, "unknown config item")
        }
    }
}


impl Default for LogLevel {
    fn default() -> LogLevel {
        LogLevel::Info
    }
}

mod defaults {
    pub fn delay_attach() -> f32 {
        5.0
    }

    pub fn task_timeout() -> f32 {
        60.0
    }
}


impl From<LogLevel> for tracing::Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Error => tracing::Level::ERROR,
            LogLevel::Warn  => tracing::Level::WARN,
            LogLevel::Info  => tracing::Level::INFO,
            LogLevel::Debug => tracing::Level::DEBUG,
            LogLevel::Trace => tracing::Level::TRACE,
        }
    }
}
