mod core;
pub use self::core::{Adapter, AtHandle, Core, DtHandle, DtcHandle};

mod proc;
pub use self::proc::ProcessAdapter;

mod srvc;
pub use self::srvc::ServiceAdapter;


use sdtx::event;
pub use sdtx::{BaseInfo, BaseState, DeviceMode, DeviceType, HardwareError, LatchStatus};


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    NotAttached,
    NotFeasible,
    Timeout,
    Unknown(u8),
}

impl From<sdtx::RuntimeError> for RuntimeError {
    fn from(err: sdtx::RuntimeError) -> Self {
        match err {
            sdtx::RuntimeError::NotFeasible => Self::NotFeasible,
            sdtx::RuntimeError::Timeout     => Self::Timeout,
            sdtx::RuntimeError::Unknown(x)  => Self::Unknown(x),
        }
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAttached => write!(f, "no base attached"),
            Self::NotFeasible => write!(f, "not feasible"),
            Self::Timeout     => write!(f, "timeout"),
            Self::Unknown(x)  => write!(f, "unknown: {:#04x}", x),
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    UserRequest,    // user or higher layer requested cancelation, or user did not act
    Runtime(RuntimeError),
    Hardware(HardwareError),
    Unknown(u16),
}

impl From<event::CancelReason> for CancelReason {
    fn from(reason: event::CancelReason) -> Self {
        match reason {
            event::CancelReason::Runtime(e)  => Self::Runtime(RuntimeError::from(e)),
            event::CancelReason::Hardware(e) => Self::Hardware(e),
            event::CancelReason::Unknown(x)  => Self::Unknown(x),
        }
    }
}

impl std::fmt::Display for CancelReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserRequest   => write!(f, "user request"),
            Self::Runtime(err)  => write!(f, "runtime error: {}", err),
            Self::Hardware(err) => write!(f, "hardware error: {}", err),
            Self::Unknown(x)    => write!(f, "unknown: {:#04x}", x),
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatchState {
    Closed,
    Opened,
}
