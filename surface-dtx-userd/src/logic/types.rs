use std::collections::HashMap;
use std::convert::TryFrom;
use std::str::FromStr;

use anyhow::{Context, Error, Result};

use dbus::{Message, MessageType};
use dbus::arg::{Variant, RefArg};


#[derive(Debug, Clone, Copy)]
pub enum Event {
    DetachmentInhibited { reason: CancelReason },
    DetachmentStart,
    DetachmentComplete,
    DetachmentTimeout,
    DetachmentCancelStart { reason: CancelReason },
    DetachmentCancelComplete,
    DetachmentCancelTimeout,
    DetachmentUnexpected,
    AttachmentStart,
    AttachmentComplete,
    AttachmentTimeout,
}

impl Event {
    pub fn match_message(msg: &Message) -> bool {
        msg.msg_type() == MessageType::Signal
            && msg.path() == Some("/org/surface/dtx".into())
            && msg.interface() == Some("org.surface.dtx".into())
            && msg.member() == Some("Event".into())
    }

    pub fn try_from_message(msg: &Message) -> Result<Option<Self>> {
        if Self::match_message(msg) {
            Self::from_message(msg).map(Some)
        } else {
            Ok(None)
        }
    }

    #[allow(clippy::type_complexity)]
    pub fn from_message(msg: &Message) -> Result<Self> {
        let (ty, args): (&str, HashMap<&str, Variant<Box<dyn RefArg>>>) = msg.read2()
            .context("Protocol error")?;

        let event = match ty {
            "detachment:inhibited" => {
                let reason = args.get("reason")
                    .ok_or_else(|| anyhow::anyhow!("Missing argument: reason"))
                    .and_then(CancelReason::try_from)
                    .context("Protocol error")?;

                Event::DetachmentInhibited { reason }
            },
            "detachment:start" => {
                Event::DetachmentStart
            },
            "detachment:complete" => {
                Event::DetachmentComplete
            },
            "detachment:timeout" => {
                Event::DetachmentTimeout
            },
            "detachment:cancel:start" => {
                let reason = args.get("reason")
                    .ok_or_else(|| anyhow::anyhow!("Missing argument: reason"))
                    .and_then(CancelReason::try_from)
                    .context("Protocol error")?;

                Event::DetachmentCancelStart { reason }
            },
            "detachment:cancel:complete" => {
                Event::DetachmentCancelComplete
            },
            "detachment:cancel:timeout" => {
                Event::DetachmentCancelTimeout
            },
            "detachment:unexpected" => {
                Event::DetachmentUnexpected
            },
            "attachment:start" => {
                Event::AttachmentStart
            },
            "attachment:complete" => {
                Event::AttachmentComplete
            },
            "attachment:timeout" => {
                Event::AttachmentTimeout
            },
            _ => {
                Err(anyhow::anyhow!("Unsupported event type: {}", ty))
                    .context("Protocol error")?
            },
        };

        Ok(event)
    }
}

impl TryFrom<&Message> for Event {
    type Error = Error;

    fn try_from(msg: &Message) -> Result<Self> {
        Self::from_message(msg)
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    UserRequest,
    Runtime(RuntimeError),
    Hardware(HardwareError),
    Unknown(u16),
}

impl FromStr for CancelReason {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "request" => Ok(Self::UserRequest),
            _ if s.starts_with("error:runtime") => Ok(Self::Runtime(RuntimeError::from_str(s)?)),
            _ if s.starts_with("error:hardware") => Ok(Self::Hardware(HardwareError::from_str(s)?)),
            _ if s.starts_with("unknown:") => {
                let value = s.strip_prefix("unknown:")
                    .unwrap_or("")
                    .parse()
                    .context("Failed to parse unknown cancel reason")
                    .context("Protocol error")?;

                Ok(Self::Unknown(value))
            },
            _ => {
                Err(anyhow::anyhow!("Unknown cancel reason: {}", s))
                    .context("Protocol error")
            },
        }
    }
}

impl TryFrom<&Variant<Box<dyn RefArg>>> for CancelReason {
    type Error = Error;

    fn try_from(value: &Variant<Box<dyn RefArg>>) -> Result<Self> {
        let value = value.as_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid value type: {:?}", value))
            .context("Protocol error")?;

        Self::from_str(value)
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    NotAttached,
    NotFeasible,
    Timeout,
    Unknown(u8),
}

impl FromStr for RuntimeError {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "error:runtime:not-attached" => Ok(Self::NotAttached),
            "error:runtime:not-feasible" => Ok(Self::NotFeasible),
            "error:runtime:timeout"      => Ok(Self::Timeout),
            _ if s.starts_with("error:runtime:unknown:") => {
                let value = s.strip_prefix("error:runtime:unknown:")
                    .unwrap_or("")
                    .parse()
                    .context("Failed to parse unknown runtime error value")
                    .context("Protocol error")?;

                Ok(Self::Unknown(value))
            },
            _ => {
                Err(anyhow::anyhow!("Unknown runtime error value: {}", s))
                    .context("Protocol error")
            },
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareError {
    FailedToOpen,
    FailedToRemainOpen,
    FailedToClose,
    Unknown(u8),
}

impl FromStr for HardwareError {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "error:hardware:failed-to-open" => Ok(Self::FailedToOpen),
            "error:hardware:failed-to-remain-open" => Ok(Self::FailedToRemainOpen),
            "error:hardware:failed-to-close" => Ok(Self::FailedToClose),
            _ if s.starts_with("error:hardware:unknown:") => {
                let value = s.strip_prefix("error:hardware:unknown:")
                    .unwrap_or("")
                    .parse()
                    .context("Failed to parse unknown hardware error value")
                    .context("Protocol error")?;

                Ok(Self::Unknown(value))
            },
            _ => {
                Err(anyhow::anyhow!("Unknown hardware error value: {}", s))
                    .context("Protocol error")
            },
        }
    }
}
