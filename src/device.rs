use std::convert::TryFrom;
use std::{fs::File, path::Path, os::unix::io::AsRawFd};
use std::io::BufReader;

use tokio::prelude::*;
use tokio::reactor::PollEvented2;

use nix::{ioctl_none, ioctl_read};
use nix::{request_code_read, request_code_none, convert_ioctl_res, ioc};

use crate::error::{Result, ResultExt, Error, ErrorKind};


const DEFAULT_EVENT_FILE_PATH: &str = "/dev/surface_dtx";


#[derive(Debug)]
pub struct Device {
    file: File,
}

impl Device {
    pub fn open() -> Result<Self> {
        Device::open_path(DEFAULT_EVENT_FILE_PATH)
    }

    pub fn open_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path).context(ErrorKind::DeviceAccess)?;
        Ok(Device { file })
    }

    pub fn events(&self) -> Result<EventStream> {
        EventStream::from_file(self.file.try_clone().context(ErrorKind::DeviceAccess)?)
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
    reader: BufReader<PollEvented2<tokio_file_unix::File<File>>>,
}

impl EventStream {
    fn from_file(file: File) -> Result<Self> {
        let file = tokio_file_unix::File::new_nb(file).context(ErrorKind::DeviceAccess)?;
        let reader = file.into_reader(&Default::default()).context(ErrorKind::DeviceAccess)?;
        Ok(EventStream { reader })
    }
}

impl Stream for EventStream {
    type Item = RawEvent;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<RawEvent>, Error> {
        let mut buf = [0; 4];

        match self.reader.poll_read(&mut buf[..]) {
            Ok(Async::NotReady) => {
                Ok(Async::NotReady)
            },
            Ok(Async::Ready(4)) => {
                let evt = RawEvent {
                    typ:  buf[0],
                    code: buf[1],
                    arg0: buf[2],
                    arg1: buf[3],
                };
                Ok(Async::Ready(Some(evt)))
            },
            Ok(Async::Ready(_)) => {
                Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "incomplete event"))
                    .context(ErrorKind::DeviceIo)
                    .map_err(Into::into)
            },
            Err(e) => {
                Err(e)
                    .context(ErrorKind::DeviceIo)
                    .map_err(Into::into)
            },
        }
    }
}


#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OpMode {
    Tablet,
    Laptop,
    Studio,
}

impl TryFrom<u8> for OpMode {
    type Error = u8;

    fn try_from(val: u8) -> std::result::Result<Self, Self::Error> {
        match val {
            0 => Ok(OpMode::Tablet),
            1 => Ok(OpMode::Laptop),
            2 => Ok(OpMode::Studio),
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
    OpModeChange {
        mode: OpMode
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
                Event::OpModeChange { mode: OpMode::try_from(arg0).unwrap() }
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
    pub fn latch_open(&self) -> Result<()> {
        unsafe { dtx_latch_open(self.device.as_raw_fd()).context(ErrorKind::DeviceIo)? };
        Ok(())
    }

    #[allow(unused)]
    pub fn get_opmode(&self) -> Result<OpMode> {
        use std::io;

        let mut opmode: u32 = 0;
        unsafe {
            dtx_get_opmode(self.device.as_raw_fd(), &mut opmode as *mut u32)
                .context(ErrorKind::DeviceIo)?
        };

        match opmode {
            0 => Ok(OpMode::Tablet),
            1 => Ok(OpMode::Laptop),
            2 => Ok(OpMode::Studio),
            x => {
                Err(io::Error::new(io::ErrorKind::InvalidData, "invalid opmode"))
                    .context(ErrorKind::DeviceIo)
                    .map_err(Into::into)
            },
        }
    }
}


ioctl_none!(dtx_latch_lock,    0x11, 0x01);
ioctl_none!(dtx_latch_unlock,  0x11, 0x02);
ioctl_none!(dtx_latch_request, 0x11, 0x03);
ioctl_none!(dtx_latch_open,    0x11, 0x04);
ioctl_read!(dtx_get_opmode,    0x11, 0x05, u32);
