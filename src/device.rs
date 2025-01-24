use anyhow::Result;
use log::{debug, error, trace};
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;
use std::fs;
use std::io::ErrorKind;
use std::marker::PhantomData;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const MTU: usize = 1500;

pub(crate) struct QemuDevice {
    target: PathBuf,
    tx: Rc<UnixDatagram>,
    rx: UnixDatagram,
}

pub(crate) struct QemuRxToken<'a> {
    buf: Vec<u8>,
    phantom: PhantomData<&'a ()>,
}

pub(crate) struct QemuTxToken<'a> {
    socket: Rc<UnixDatagram>,
    target: &'a Path,
}

impl QemuDevice {
    pub(crate) fn new<P: AsRef<Path>>(bind_path: P, connect_path: P) -> Result<Self> {
        match fs::remove_file(bind_path.as_ref()) {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return anyhow::Result::Err(e.into()),
        }

        let rx = UnixDatagram::bind(bind_path.as_ref())?;
        rx.set_nonblocking(true)?;

        let tx = UnixDatagram::unbound()?;
        tx.set_nonblocking(true)?;

        Ok(Self {
            rx,
            tx: Rc::new(tx),
            target: connect_path.as_ref().to_owned(),
        })
    }

    pub(crate) fn get_rx(&self) -> &UnixDatagram {
        &self.rx
    }
}

impl Device for QemuDevice {
    type RxToken<'a> = QemuRxToken<'a>;
    type TxToken<'a> = QemuTxToken<'a>;

    fn receive(&mut self, _: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let mut buf: Vec<u8> = vec![0; 2 * MTU];

        match self.rx.recv(&mut buf) {
            Ok(size) => buf.truncate(size),
            Err(e) if e.kind() == ErrorKind::WouldBlock => return None,
            Err(e) => {
                error!("rx {:?}", e);
                return None;
            }
        }

        trace!("rx {}", buf.len());

        Some((
            QemuRxToken {
                buf,
                phantom: Default::default(),
            },
            QemuTxToken {
                socket: Rc::clone(&self.tx),
                target: &self.target,
            },
        ))
    }

    fn transmit(&mut self, _: Instant) -> Option<Self::TxToken<'_>> {
        Some(QemuTxToken {
            socket: Rc::clone(&self.tx),
            target: &self.target,
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

        match self.socket.send_to(&buf, self.target) {
            Ok(written) => trace!("tx {}", written),
            Err(e) if e.kind() == ErrorKind::WouldBlock => debug!("dropped tx of length {}", len),
            Err(e) => error!("tx {:?}", e),
        }

        result
    }
}
