use crate::logic::{
    BaseInfo,
    BaseState,
    DeviceMode,
    DeviceType,
    HardwareError,
    LatchStatus,
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
                HardwareError::FailedToOpen       => "error:failed-to-open".into(),
                HardwareError::FailedToRemainOpen => "error:failed-to-remain-open".into(),
                HardwareError::FailedToClose      => "error:failed-to-close".into(),
                HardwareError::Unknown(x) => format!("error:unknown:{}", x),
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
