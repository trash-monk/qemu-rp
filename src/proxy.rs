use anyhow::{Ok, Result};
use log::{debug, trace};
use smoltcp::socket::tcp::Socket;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;

pub(crate) struct Connection {
    socket: TcpStream,
    port: u16,
}

impl Connection {
    pub(crate) fn new(client: TcpStream, port: u16) -> Self {
        Self {
            socket: client,
            port,
        }
    }

    pub(crate) fn get_port(&self) -> u16 {
        self.port
    }
}

fn proxy_inner(vm_client: &mut Socket, remote: &mut Connection) -> Result<()> {
    let mut rc = Ok(());

    if vm_client.can_recv() {
        rc = vm_client.recv(|data| match remote.socket.write(data) {
            Result::Ok(written) => {
                trace!("{} recv from vm {}", remote.get_port(), written);
                (written, Ok(()))
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => (0, Ok(())),
            Err(e) => (0, Err(e.into())),
        })?;
    }

    if rc.is_err() {
        return rc;
    }

    if vm_client.can_send() {
        let got = vm_client.send(|data| match remote.socket.read(data) {
            Result::Ok(0) => (0, Ok(true)),
            Result::Ok(written) => {
                trace!("{} send to vm {}", remote.get_port(), written);
                (written, Ok(false))
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => (0, Ok(false)),
            Err(e) => (0, Err(e.into())),
        })?;
        rc = got.map(|closed| {
            if closed {
                debug!("{} close vm connection", remote.get_port());
                vm_client.close()
            }
        })
    }

    rc
}

pub(crate) fn proxy(vm_client: &mut Socket, remote: &mut Connection) -> Result<bool> {
    let out = proxy_inner(vm_client, remote);

    if let Err(e) = out {
        vm_client.abort();
        return Err(e);
    }

    if vm_client.is_active() || vm_client.recv_queue() > 0 {
        return Ok(true);
    }

    debug!("{} vm connection closed", remote.get_port());
    Ok(false)
}
