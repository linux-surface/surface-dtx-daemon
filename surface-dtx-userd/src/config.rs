use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::error::{ErrorKind, Result, ResultExt};


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
