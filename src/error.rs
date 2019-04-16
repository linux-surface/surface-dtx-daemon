use toml;

use std::borrow::Cow;

use failure::Fail;


#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "{}", message)]
    Message {
        message: Cow<'static, str>,
    },
    #[fail(display = "{}", cause)]
    Io {
        cause: std::io::Error,
    },
    #[fail(display = "{}", cause)]
    System {
        cause: nix::Error,
    },
    #[fail(display = "{}", cause)]
    ConfigSyntax {
        cause: toml::de::Error,
    },
}

impl From<String> for Error {
    fn from(msg: String) -> Error {
        Error::Message { message: Cow::Owned(msg) }
    }
}

impl From<&'static str> for Error {
    fn from(msg: &'static str) -> Error {
        Error::Message { message: Cow::Borrowed(msg) }
    }
}

impl From<std::io::Error> for Error {
    fn from(cause: std::io::Error) -> Error {
        Error::Io { cause }
    }
}

impl From<nix::Error> for Error {
    fn from(cause: nix::Error) -> Error {
        Error::System { cause }
    }
}

impl From<toml::de::Error> for Error {
    fn from(cause: toml::de::Error) -> Error {
        Error::ConfigSyntax { cause }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
