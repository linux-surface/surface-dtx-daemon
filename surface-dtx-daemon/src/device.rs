use std::convert::{TryFrom, TryInto};
use std::path::Path;
use std::pin::Pin;
use std::os::unix::io::AsRawFd;
use std::task::{Context, Poll};

use smallvec::SmallVec;

use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufRead, BufReader};
use tokio_stream::Stream;

use nix::{ioctl_none, ioctl_read};

use crate::error::{Result, ResultExt, Error, ErrorKind};


const DEFAULT_EVENT_FILE_PATH: &str = "/dev/surface/dtx";


#[derive(Debug)]
pub struct Device {
    file: File,
}

impl Device {
    pub async fn open() -> Result<Self> {
        Device::open_path(DEFAULT_EVENT_FILE_PATH).await
    }

    pub async fn open_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(false)
                .open(path).await
                .context(ErrorKind::DeviceAccess)?;

        Ok(Device { file })
    }

    pub async fn events(&self) -> Result<EventStream> {
        let file = self.file
                .try_clone().await
                .context(ErrorKind::DeviceAccess)?;

        self.commands().events_enable()?;
        Ok(EventStream::from_file(file))
    }

    pub fn commands(&self) -> Commands {
        Commands { device: &self }
    }
}

impl std::os::unix::io::AsRawFd for Device {
    fn as_raw_fd(&self) -> std::os::unix::io::RawFd {
        self.file.as_raw_fd()
    }
}


pub struct EventStream {
    reader: BufReader<File>,
}

impl EventStream {
    fn from_file(file: File) -> Self {
        EventStream { reader: BufReader::with_capacity(128, file) }
    }
}

impl Stream for EventStream {
    type Item = Result<RawEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let poll = Pin::new(&mut self.reader)
                .poll_fill_buf(cx)
                .map_err(|e| Error::with(e, ErrorKind::DeviceIo))?;

        let buf = match poll {
            Poll::Ready(buf) if buf.len() >= 4 => buf,
            _ => return Poll::Pending,
        };

        let hdr_len = std::mem::size_of::<EventHeader>();
        let hdr = EventHeader::from_bytes(buf[0..hdr_len].try_into().unwrap());
        let len = hdr_len + hdr.len as usize;

        if buf.len() < len {
            return Poll::Pending;
        }

        let evt = RawEvent {
            code: hdr.code,
            data: SmallVec::from_slice(&buf[hdr_len..len])
        };

        Pin::new(&mut self.reader).consume(len);
        Poll::Ready(Some(Ok(evt)))
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct EventHeader {
    len: u16,
    code: u16,
}

impl EventHeader {
    fn from_bytes(bytes: [u8; std::mem::size_of::<Self>()]) -> Self {
        unsafe { std::mem::transmute(bytes) }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RawEvent {
    pub code: u16,
    pub data: SmallVec<[u8; 4]>,
}


#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DeviceMode {
    Tablet,
    Laptop,
    Studio,
    Unknown(u16),
}

impl DeviceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceMode::Tablet => "tablet",
            DeviceMode::Laptop => "laptop",
            DeviceMode::Studio => "studio",
            DeviceMode::Unknown(_) => "<unknown>",
        }
    }
}

impl From<u16> for DeviceMode {
    fn from(val: u16) -> Self {
        match val {
            0 => DeviceMode::Tablet,
            1 => DeviceMode::Laptop,
            2 => DeviceMode::Studio,
            x => DeviceMode::Unknown(x),
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connected,
    NotFeasible,
    Unknown(u16)
}

impl From<u16> for ConnectionState {
    fn from(val: u16) -> Self {
        match val {
            0x0000 => ConnectionState::Disconnected,
            0x0001 => ConnectionState::Connected,
            0x1001 => ConnectionState::NotFeasible,
            x      => ConnectionState::Unknown(x),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseInfo {
    state: ConnectionState,
    base_id: u16,
}



#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    NotFeasible,
    Timedout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareError {
    FailedToOpen,
    FailedToRemainOpen,
    FailedToClose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatchStatus {
    Closed,
    Open,
    HwError(HardwareError),
    Unknown(u16),
}

impl From<u16> for LatchStatus {
    fn from(val: u16) -> Self {
        match val {
            0x0000 => LatchStatus::Closed,
            0x0001 => LatchStatus::Open,
            0x2001 => LatchStatus::HwError(HardwareError::FailedToOpen),
            0x2002 => LatchStatus::HwError(HardwareError::FailedToRemainOpen),
            0x2003 => LatchStatus::HwError(HardwareError::FailedToClose),
            x      => LatchStatus::Unknown(x),
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetachError {
    RtError(RuntimeError),
    HwError(HardwareError),
    Unknown(u16),
}

impl From<u16> for DetachError {
    fn from(val: u16) -> Self {
        match val {
            0x1001 => DetachError::RtError(RuntimeError::NotFeasible),
            0x1002 => DetachError::RtError(RuntimeError::Timedout),
            0x2001 => DetachError::HwError(HardwareError::FailedToOpen),
            0x2002 => DetachError::HwError(HardwareError::FailedToRemainOpen),
            0x2003 => DetachError::HwError(HardwareError::FailedToClose),
            x      => DetachError::Unknown(x),
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    DeviceModeChange {
        mode: DeviceMode
    },

    ConectionChange {
        state: ConnectionState,
        base_id: u16
    },

    LatchStatusChange {
        state: LatchStatus,
    },

    DetachError {
        err: DetachError,
    },

    DetachRequest,
}

impl TryFrom<RawEvent> for Event {
    type Error = RawEvent;

    fn try_from(evt: RawEvent) -> std::result::Result<Self, Self::Error> {
        let evt = match evt {
            RawEvent { code: 1, data } if data.is_empty() => {
                Event::DetachRequest
            },
            RawEvent { code: 2, data } if data.len() == 2 => {
                let err = u16::from_ne_bytes(data[0..2].try_into().unwrap()).into();
                Event::DetachError { err }
            },
            RawEvent { code: 3, data } if data.len() == 4 => {
                let state = u16::from_ne_bytes(data[0..2].try_into().unwrap()).into();

                Event::ConectionChange {
                    state,
                    base_id: u16::from_ne_bytes(data[2..4].try_into().unwrap()),
                }
            },
            RawEvent { code: 4, data } if data.len() == 2 => {
                let state = u16::from_ne_bytes(data[0..2].try_into().unwrap()).into();
                Event::LatchStatusChange { state }
            },
            RawEvent { code: 5, data } if data.len() == 2 => {
                let mode = u16::from_ne_bytes(data[0..2].try_into().unwrap()).into();
                Event::DeviceModeChange { mode }
            },
            _ => return Err(evt)
        };

        Ok(evt)
    }
}


pub struct Commands<'a> {
    device: &'a Device,
}

impl<'a> Commands<'a> {
    pub fn events_enable(&self) -> Result<()> {
        unsafe { dtx_events_enable(self.device.as_raw_fd()).context(ErrorKind::DeviceIo)? };
        Ok(())
    }

    #[allow(unused)]
    pub fn events_disable(&self) -> Result<()> {
        unsafe { dtx_events_disable(self.device.as_raw_fd()).context(ErrorKind::DeviceIo)? };
        Ok(())
    }

    #[allow(unused)]
    pub fn latch_lock(&self) -> Result<()> {
        unsafe { dtx_latch_lock(self.device.as_raw_fd()).context(ErrorKind::DeviceIo)? };
        Ok(())
    }

    #[allow(unused)]
    pub fn latch_unlock(&self) -> Result<()> {
        unsafe { dtx_latch_unlock(self.device.as_raw_fd()).context(ErrorKind::DeviceIo)? };
        Ok(())
    }

    pub fn latch_request(&self) -> Result<()> {
        unsafe { dtx_latch_request(self.device.as_raw_fd()).context(ErrorKind::DeviceIo)? };
        Ok(())
    }

    pub fn latch_confirm(&self) -> Result<()> {
        unsafe { dtx_latch_confirm(self.device.as_raw_fd()).context(ErrorKind::DeviceIo)? };
        Ok(())
    }

    #[allow(unused)]
    pub fn latch_heartbeat(&self) -> Result<()> {
        unsafe { dtx_latch_heartbeat(self.device.as_raw_fd()).context(ErrorKind::DeviceIo)? };
        Ok(())
    }

    pub fn latch_cancel(&self) -> Result<()> {
        unsafe { dtx_latch_cancel(self.device.as_raw_fd()).context(ErrorKind::DeviceIo)? };
        Ok(())
    }

    #[allow(unused)]
    pub fn get_base_info(&self) -> Result<BaseInfo> {
        use std::io;

        let mut info = RawBaseInfo { state: 0, base_id: 0 };
        unsafe {
            dtx_get_base_info(self.device.as_raw_fd(), &mut info as *mut RawBaseInfo)
                    .context(ErrorKind::DeviceIo)?
        };

        let state = ConnectionState::try_from(info.state)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData,
                        format!("invalid connection state: {}", e)))
                .context(ErrorKind::DeviceIo)?;

        Ok(BaseInfo { state, base_id: info.base_id })
    }

    pub fn get_device_mode(&self) -> Result<DeviceMode> {
        use std::io;

        let mut mode: u16 = 0;
        unsafe {
            dtx_get_device_mode(self.device.as_raw_fd(), &mut mode as *mut u16)
                    .context(ErrorKind::DeviceIo)?
        };

        let mode = DeviceMode::try_from(mode)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData,
                        format!("invalid device mode: {}", e)))
                .context(ErrorKind::DeviceIo)?;

        Ok(mode)
    }

    #[allow(unused)]
    pub fn get_latch_status(&self) -> Result<LatchStatus> {
        use std::io;

        let mut status: u16 = 0;
        unsafe {
            dtx_get_latch_status(self.device.as_raw_fd(), &mut status as *mut u16)
                    .context(ErrorKind::DeviceIo)?
        };

        let status = LatchStatus::try_from(status)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData,
                        format!("invalid latch status: {}", e)))
                .context(ErrorKind::DeviceIo)?;

        Ok(status)
    }
}


#[repr(C)]
pub struct RawBaseInfo {
    state: u16,
    base_id: u16,
}

ioctl_none!(dtx_events_enable,    0xa5, 0x21);
ioctl_none!(dtx_events_disable,   0xa5, 0x22);

ioctl_none!(dtx_latch_lock,       0xa5, 0x23);
ioctl_none!(dtx_latch_unlock,     0xa5, 0x24);
ioctl_none!(dtx_latch_request,    0xa5, 0x25);
ioctl_none!(dtx_latch_confirm,    0xa5, 0x26);
ioctl_none!(dtx_latch_heartbeat,  0xa5, 0x27);
ioctl_none!(dtx_latch_cancel,     0xa5, 0x28);

ioctl_read!(dtx_get_base_info,    0xa5, 0x29, RawBaseInfo);
ioctl_read!(dtx_get_device_mode,  0xa5, 0x2a, u16);
ioctl_read!(dtx_get_latch_status, 0xa5, 0x2b, u16);
