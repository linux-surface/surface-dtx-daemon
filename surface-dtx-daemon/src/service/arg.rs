use crate::logic::{
    BaseInfo,
    BaseState,
    CancelReason,
    DeviceMode,
    DeviceType,
    HardwareError,
    LatchStatus,
    RuntimeError,
};

use dbus::arg::Variant;


pub trait DbusArg {
    type Arg: dbus::arg::RefArg + 'static;

    fn as_arg(&self) -> Self::Arg;

    fn as_variant(&self) -> Variant<Box<dyn dbus::arg::RefArg>> {
        Variant(Box::new(self.as_arg()))
    }
}

impl DbusArg for DeviceMode {
    type Arg = String;

    fn as_arg(&self) -> String {
        match self {
            DeviceMode::Tablet => "tablet",
            DeviceMode::Laptop => "laptop",
            DeviceMode::Studio => "studio",
        }.into()
    }
}

impl DbusArg for LatchStatus {
    type Arg = String;

    fn as_arg(&self) -> String {
        match self {
            LatchStatus::Closed => "closed".into(),
            LatchStatus::Opened => "opened".into(),
            LatchStatus::Error(error) => match error {
                HardwareError::FailedToOpen       => "error:hardware:failed-to-open".into(),
                HardwareError::FailedToRemainOpen => "error:hardware:failed-to-remain-open".into(),
                HardwareError::FailedToClose      => "error:hardware:failed-to-close".into(),
                HardwareError::Unknown(x) => format!("error:hardware:unknown:{}", x),
            },
        }
    }
}

impl DbusArg for BaseInfo {
    type Arg = (String, String, u8);

    fn as_arg(&self) -> Self::Arg {
        (self.state.as_arg(), self.device_type.as_arg(), self.id)
    }
}

impl DbusArg for BaseState {
    type Arg = String;

    fn as_arg(&self) -> Self::Arg {
        match self {
            BaseState::Detached    => "detached",
            BaseState::Attached    => "attached",
            BaseState::NotFeasible => "not-feasible",
        }.into()
    }
}

impl DbusArg for DeviceType {
    type Arg = String;

    fn as_arg(&self) -> Self::Arg {
        match self {
            DeviceType::Hid => "hid".into(),
            DeviceType::Ssh => "ssh".into(),
            DeviceType::Unknown(x) => format!("unknown:{}", x),
        }
    }
}

impl DbusArg for CancelReason {
    type Arg = String;

    fn as_arg(&self) -> Self::Arg {
        match self {
            CancelReason::UserRequest             => "request".into(),
            CancelReason::HandlerTimeout          => "timeout:handler".into(),
            CancelReason::DisconnectTimeout       => "timeout:disconnect".into(),
            CancelReason::Runtime(rt) => match rt {
                RuntimeError::NotAttached         => "error:runtime:not-attached".into(),
                RuntimeError::NotFeasible         => "error:runtime:not-feasible".into(),
                RuntimeError::Timeout             => "error:runtime:timeout".into(),
                RuntimeError::Unknown(x)  => format!("error:runtime:unknown:{}", x),
            },
            CancelReason::Hardware(hw) => match hw {
                HardwareError::FailedToOpen       => "error:hardware:failedt-to-open".into(),
                HardwareError::FailedToRemainOpen => "error:hardware:failed-to-remain-open".into(),
                HardwareError::FailedToClose      => "error:hardware:failed-to-close".into(),
                HardwareError::Unknown(x) => format!("error:hardware:unknown:{}", x),
            },
            CancelReason::Unknown(x) => format!("unknown:{}", x),
        }
    }
}
