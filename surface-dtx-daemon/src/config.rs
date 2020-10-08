use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::error::{ErrorKind, Result, ResultExt};


const DEFAULT_CONFIG_PATH: &str = "/etc/surface-dtx/surface-dtx-daemon.conf";


#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Config {
    #[serde(skip)]
    pub dir: PathBuf,

    #[serde(default)]
    pub log: Log,

    #[serde(default)]
    pub handler: Handler,

    #[serde(default)]
    pub delay: Delay,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Log {
    #[serde(default)]
    pub level: LogLevel,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all="lowercase")]
pub enum LogLevel {
    Critical,
    Error,
    Warning,
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Handler {
    #[serde(default)]
    pub detach: Option<PathBuf>,

    #[serde(default)]
    pub detach_abort: Option<PathBuf>,

    #[serde(default)]
    pub attach: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Delay {
    #[serde(default="defaults::delay_attach")]
    pub attach: f32,
}


impl Config {
    pub fn load() -> Result<Config> {
        if Path::new(DEFAULT_CONFIG_PATH).exists() {
            Config::load_file(DEFAULT_CONFIG_PATH)
        } else {
            Ok(Config::default())
        }
    }

    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Config> {
        use std::io::Read;

        let mut buf = Vec::new();
        let mut file = std::fs::File::open(path.as_ref()).context(ErrorKind::Config)?;
        file.read_to_end(&mut buf).context(ErrorKind::Config)?;

        let mut config: Config = toml::from_slice(&buf).context(ErrorKind::Config)?;
        config.dir = path.as_ref().parent().unwrap().into();

        Ok(config)
    }
}


impl Default for LogLevel {
    fn default() -> LogLevel {
        LogLevel::Info
    }
}

impl Default for Delay {
    fn default() -> Delay {
        Delay {
            attach: defaults::delay_attach(),
        }
    }
}

mod defaults {
    pub fn delay_attach() -> f32 {
        5.0
    }
}


impl Into<slog::Level> for LogLevel {
    fn into(self) -> slog::Level {
        match self {
            LogLevel::Critical => slog::Level::Critical,
            LogLevel::Error    => slog::Level::Error,
            LogLevel::Warning  => slog::Level::Warning,
            LogLevel::Info     => slog::Level::Info,
            LogLevel::Debug    => slog::Level::Debug,
            LogLevel::Trace    => slog::Level::Trace,
        }
    }
}
