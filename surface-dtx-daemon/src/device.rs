use std::convert::TryFrom;
use std::path::Path;
use std::pin::Pin;
use std::os::unix::io::AsRawFd;
use std::task::{Context, Poll};

use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufRead, BufReader};
use tokio::stream::Stream;

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

        Ok(EventStream::from_file(file))
    }

    #[allow(unused)]
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

        match poll {
            Poll::Ready(buf) if buf.len() >= 4 => {
                let evt = RawEvent {
                    typ:  buf[0],
                    code: buf[1],
                    arg0: buf[2],
                    arg1: buf[3],
                };

                Pin::new(&mut self.reader).consume(4);
                Poll::Ready(Some(Ok(evt)))
            },
            _ => Poll::Pending,
        }
    }
}


#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DeviceMode {
    Tablet,
    Laptop,
    Studio,
}

impl DeviceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceMode::Tablet => "tablet",
            DeviceMode::Laptop => "laptop",
            DeviceMode::Studio => "studio",
        }
    }
}

impl TryFrom<u8> for DeviceMode {
    type Error = u8;

    fn try_from(val: u8) -> std::result::Result<Self, Self::Error> {
        match val {
            0 => Ok(DeviceMode::Tablet),
            1 => Ok(DeviceMode::Laptop),
            2 => Ok(DeviceMode::Studio),
            x => Err(x),
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connected,
}

impl TryFrom<u8> for ConnectionState {
    type Error = u8;

    fn try_from(val: u8) -> std::result::Result<Self, Self::Error> {
        match val {
            0 => Ok(ConnectionState::Disconnected),
            1 => Ok(ConnectionState::Connected),
            x => Err(x),
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatchState {
    Closed,
    Open,
}

impl TryFrom<u8> for LatchState {
    type Error = u8;

    fn try_from(val: u8) -> std::result::Result<Self, Self::Error> {
        match val {
            0 => Ok(LatchState::Closed),
            1 => Ok(LatchState::Open),
            x => Err(x),
        }
    }
}


#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RawEvent {
    pub typ:  u8,
    pub code: u8,
    pub arg0: u8,
    pub arg1: u8,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    DeviceModeChange {
        mode: DeviceMode
    },

    ConectionChange {
        state: ConnectionState,
        arg1:  u8
    },

    LatchStateChange {
        state: LatchState
    },

    DetachError {
        err: u8
    },

    DetachRequest,
}

impl TryFrom<RawEvent> for Event {
    type Error = RawEvent;

    fn try_from(evt: RawEvent) -> std::result::Result<Self, Self::Error> {
        let evt = match evt {
            RawEvent { typ: 0x11, code: 0x0c, arg0, arg1 } if arg0 <= 1 => {
                Event::ConectionChange { state: ConnectionState::try_from(arg0).unwrap(), arg1 }
            },
            RawEvent { typ: 0x11, code: 0x0d, arg0, .. } if arg0 <= 2 => {
                Event::DeviceModeChange { mode: DeviceMode::try_from(arg0).unwrap() }
            },
            RawEvent { typ: 0x11, code: 0x0e, .. } => {
                Event::DetachRequest
            },
            RawEvent { typ: 0x11, code: 0x0f, arg0, .. } => {
                Event::DetachError { err: arg0 }
            },
            RawEvent { typ: 0x11, code: 0x11, arg0, .. } if arg0 <= 1 => {
                Event::LatchStateChange { state: LatchState::try_from(arg0).unwrap() }
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

    #[allow(unused)]
    pub fn latch_request(&self) -> Result<()> {
        unsafe { dtx_latch_request(self.device.as_raw_fd()).context(ErrorKind::DeviceIo)? };
        Ok(())
    }

    #[allow(unused)]
    pub fn latch_confirm(&self) -> Result<()> {
        unsafe { dtx_latch_confirm(self.device.as_raw_fd()).context(ErrorKind::DeviceIo)? };
        Ok(())
    }

    #[allow(unused)]
    pub fn get_device_mode(&self) -> Result<DeviceMode> {
        use std::io;

        let mut mode: u32 = 0;
        unsafe {
            dtx_get_device_mode(self.device.as_raw_fd(), &mut mode as *mut u32)
                .context(ErrorKind::DeviceIo)?
        };

        match mode {
            0 => Ok(DeviceMode::Tablet),
            1 => Ok(DeviceMode::Laptop),
            2 => Ok(DeviceMode::Studio),
            x => {
                Err(io::Error::new(io::ErrorKind::InvalidData, "invalid device mode"))
                    .context(ErrorKind::DeviceIo)
                    .map_err(Into::into)
            },
        }
    }
}


ioctl_none!(dtx_latch_lock,      0x11, 0x01);
ioctl_none!(dtx_latch_unlock,    0x11, 0x02);
ioctl_none!(dtx_latch_request,   0x11, 0x03);
ioctl_none!(dtx_latch_confirm,   0x11, 0x04);
ioctl_read!(dtx_get_device_mode, 0x11, 0x05, u32);
