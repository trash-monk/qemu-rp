use anyhow::Result;
use log::debug;
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
use std::fs;
use std::io::ErrorKind;
use std::marker::PhantomData;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::rc::Rc;

const MTU: usize = 1500;

pub(crate) struct QemuDevice {
    socket: Rc<UnixDatagram>,
}

pub(crate) struct QemuRxToken<'a> {
    buf: Vec<u8>,
    phantom: PhantomData<&'a ()>,
}
pub(crate) struct QemuTxToken<'a> {
    socket: Rc<UnixDatagram>,
    phantom: PhantomData<&'a ()>,
}

impl QemuDevice {
    pub(crate) fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        match fs::remove_file(path.as_ref()) {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return anyhow::Result::Err(e.into()),
        }

        let uds = UnixDatagram::bind(path.as_ref())?;
        uds.set_nonblocking(true)?;

        Ok(Self {
            socket: Rc::new(uds),
        })
    }
}

impl Device for QemuDevice {
    type RxToken<'a> = QemuRxToken<'a>;
    type TxToken<'a> = QemuTxToken<'a>;

    fn receive(&mut self, _: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let mut buf: Vec<u8> = vec![0; 2 * MTU];

        match self.socket.recv(&mut buf) {
            Ok(size) => buf.truncate(size),
            Err(e) if e.kind() == ErrorKind::WouldBlock => return None,
            Err(e) => Err(e).unwrap(),
        }

        Some((
            QemuRxToken {
                buf,
                phantom: Default::default(),
            },
            QemuTxToken {
                socket: Rc::clone(&self.socket),
                phantom: Default::default(),
            },
        ))
    }

    fn transmit(&mut self, _: Instant) -> Option<Self::TxToken<'_>> {
        Some(QemuTxToken {
            socket: Rc::clone(&self.socket),
            phantom: Default::default(),
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut out = DeviceCapabilities::default();
        out.medium = Medium::Ethernet;
        out.max_transmission_unit = MTU;
        out
    }
}

impl<'a> RxToken for QemuRxToken<'a> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.buf)
    }
}

impl<'a> TxToken for QemuTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf: Vec<u8> = vec![0; len];
        let result = f(&mut buf);

        match self.socket.send(&buf) {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => debug!("dropped tx of length {}", len),
            Err(e) => Err(e).unwrap(),
        }

        result
    }
}
