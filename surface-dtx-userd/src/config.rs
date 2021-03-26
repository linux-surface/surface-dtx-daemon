use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};


const SYSTEM_CONFIG_PATH: &str = "/etc/surface-dtx/surface-dtx-userd.conf";
const USER_CONFIG_LOCAL_PATH: &str = "surface-dtx/surface-dtx-userd.conf";


#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Config {
    #[serde(skip)]
    pub dir: PathBuf,

    #[serde(default)]
    pub log: Log,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Log {
    #[serde(default)]
    pub level: LogLevel,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all="lowercase")]
pub enum LogLevel {
    Critical,
    Error,
    Warning,
    Info,
    Debug,
    Trace,
}


impl Config {
    pub fn load() -> Result<Config> {
        let mut user_config = std::env::var_os("XDG_CONFIG_HOME")
            .and_then(|d| if d != "" { Some(d) } else { None })
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("~/.config"));
        user_config.push(USER_CONFIG_LOCAL_PATH);

        if user_config.exists() {
            Config::load_file(user_config)
        } else if Path::new(SYSTEM_CONFIG_PATH).exists() {
            Config::load_file(SYSTEM_CONFIG_PATH)
        } else {
            Ok(Config::default())
        }
    }

    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Config> {
        use std::io::Read;

        let mut buf = Vec::new();
        let mut file = std::fs::File::open(path.as_ref())
            .context("Failed to open config file")?;

        file.read_to_end(&mut buf)
            .with_context(|| format!("Failed to read config file (path: {:?})", path.as_ref()))?;

        let mut config: Config = toml::from_slice(&buf)
            .with_context(|| format!("Failed to read config file (path: {:?})", path.as_ref()))?;

        config.dir = path.as_ref().parent().unwrap().into();
        Ok(config)
    }
}


impl Default for LogLevel {
    fn default() -> LogLevel {
        LogLevel::Info
    }
}

impl From<LogLevel> for slog::Level {
    fn from(value: LogLevel) -> slog::Level {
        match value {
            LogLevel::Critical => slog::Level::Critical,
            LogLevel::Error    => slog::Level::Error,
            LogLevel::Warning  => slog::Level::Warning,
            LogLevel::Info     => slog::Level::Info,
            LogLevel::Debug    => slog::Level::Debug,
            LogLevel::Trace    => slog::Level::Trace,
        }
    }
}
