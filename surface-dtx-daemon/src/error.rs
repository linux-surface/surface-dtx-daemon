use failure::{Backtrace, Context, Fail};

pub type Result<T> = std::result::Result<T, Error>;
pub use failure::ResultExt;

#[derive(Debug)]
pub struct Error {
    inner: Context<ErrorKind>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Fail)]
pub enum ErrorKind {
    #[fail(display = "Invalid configuration")]
    Config,

    #[fail(display = "Device access failure")]
    DeviceAccess,

    #[fail(display = "Device I/O failure")]
    DeviceIo,

    #[fail(display = "Failed to run external process")]
    Process,

    #[fail(display = "DBus service failure")]
    DBusService,
}


impl Error {
    pub fn with<F: Fail>(cause: F, context: ErrorKind) -> Error {
        Error::from(cause.context(context))
    }

    pub fn kind(&self) -> ErrorKind {
        *self.inner.get_context()
    }

    pub fn iter_causes(&self) -> failure::Causes {
        ((&self.inner) as &dyn Fail).iter_causes()
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Error {
        Error { inner: Context::new(kind) }
    }
}

impl From<Context<ErrorKind>> for Error {
    fn from(inner: Context<ErrorKind>) -> Error {
        Error { inner }
    }
}

impl Fail for Error {
    fn cause(&self) -> Option<&dyn Fail> {
        self.inner.cause()
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        self.inner.backtrace()
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.inner, f)
    }
}


pub type CliResult = std::result::Result<(), CliError>;

pub struct CliError {
    error: Error,
}

impl From<Error> for CliError {
    fn from(error: Error) -> Self {
        CliError { error }
    }
}

impl From<Context<ErrorKind>> for CliError {
    fn from(error: Context<ErrorKind>) -> Self {
        CliError { error: error.into() }
    }
}

impl std::fmt::Debug for CliError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{}.", self.error.kind())?;
        for cause in self.error.iter_causes() {
            write!(fmt, "\n       {}.", cause)?;
        }

        Ok(())
    }
}


#[derive(Debug)]
pub struct ErrorStr {
    message: &'static str,
}

impl From<&'static str> for ErrorStr {
    fn from(message: &'static str) -> Self {
        ErrorStr { message }
    }
}

impl Fail for ErrorStr {
    fn cause(&self) -> Option<&dyn Fail> {
        None
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        None
    }
}

impl std::fmt::Display for ErrorStr {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.message, f)
    }
}
