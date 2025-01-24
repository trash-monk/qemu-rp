mod device;
mod port_alloc;
mod proxy;

use crate::device::*;
use crate::port_alloc::*;
use crate::proxy::*;
use clap::{command, Arg};
use log::{debug, error, info, trace};
use nix::libc::{suseconds_t, time_t};
use nix::sys::select::{select, FdSet};
use nix::sys::time::TimeVal;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp::{Socket, SocketBuffer};
use smoltcp::time::{Duration, Instant};
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener};
use std::os::fd::AsFd;
use std::os::unix::net::UnixDatagram;
use std::str::FromStr;

fn parse_cidr(s: &str) -> Result<IpCidr, &'static str> {
    IpCidr::from_str(s).map_err(|_| "invalid CIDR")
}

fn parse_mac(s: &str) -> Result<EthernetAddress, &'static str> {
    EthernetAddress::from_str(s).map_err(|_| "invalid MAC address")
}

fn main() -> ! {
    env_logger::init();

    let mut ports = PortAlloc::new();

    let args = command!()
        .arg(
            Arg::new("bind-path")
                .long("bind-path")
                .required(true)
                .help("path to server socket to listen on"),
        )
        .arg(
            Arg::new("connect-path")
                .long("connect-path")
                .required(true)
                .help("path to QEMU's listening socket"),
        )
        .arg(
            Arg::new("listen")
                .long("listen")
                .required(true)
                .value_parser(clap::builder::ValueParser::new(SocketAddr::from_str))
                .help("local address to listen on"),
        )
        .arg(
            Arg::new("forward")
                .long("forward")
                .required(true)
                .value_parser(clap::builder::ValueParser::new(SocketAddr::from_str))
                .help("peer address in the VM to forward connection to"),
        )
        .arg(
            Arg::new("local")
                .long("local")
                .required(true)
                .value_parser(clap::builder::ValueParser::new(parse_cidr))
                .help("local CIDR in the VM"),
        )
        .arg(
            Arg::new("mac")
                .long("mac")
                .value_parser(clap::builder::ValueParser::new(parse_mac))
                .default_value("42:00:00:00:00:69")
                .help("local MAC address in the VM"),
        )
        .get_matches();

    let bind_path = args.get_one::<String>("bind-path").unwrap();
    let connect_path = args.get_one::<String>("connect-path").unwrap();
    let listen_addr = args.get_one::<SocketAddr>("listen").unwrap();
    let forward_addr = args.get_one::<SocketAddr>("forward").unwrap();
    let local_addr = args.get_one::<IpCidr>("local").unwrap();
    let mac_addr = args.get_one::<EthernetAddress>("mac").unwrap();

    let listener = TcpListener::bind(listen_addr).unwrap();
    listener.set_nonblocking(true).unwrap();

    let mut device = QemuDevice::new(bind_path, connect_path).unwrap();

    let mut config = Config::new(HardwareAddress::Ethernet(*mac_addr));
    config.random_seed = rand::random();

    let mut iface = Interface::new(config, &mut device, Instant::now());

    iface.update_ip_addrs(|ip_addrs| {
        ip_addrs.push(*local_addr).unwrap();
    });

    let mut sockets = SocketSet::new(vec![]);
    let mut connections = HashMap::new();
    let mut dead = Vec::new();

    loop {
        match listener.accept() {
            Ok((remote, _)) => {
                info!("connection from {:?}", remote.peer_addr());
                debug!("local address {:?}", remote.local_addr());

                remote.set_nonblocking(true).unwrap();

                let port = ports.get();
                debug!("allocated port {}", port);

                let mut sock = Socket::new(
                    SocketBuffer::new(vec![0; 65535]),
                    SocketBuffer::new(vec![0; 65535]),
                );

                sock.connect(iface.context(), *forward_addr, port).unwrap();

                let handle = sockets.add(sock);

                connections.insert(handle, Connection::new(remote, port));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => Err(e).unwrap(),
        }

        for (handle, sock) in sockets.iter_mut() {
            let sock = match sock {
                smoltcp::socket::Socket::Tcp(s) => s,
                _ => unreachable!(),
            };
            match proxy(sock, connections.get_mut(&handle).unwrap()) {
                Err(e) => error!("{:?}", e),
                Ok(true) => continue,
                Ok(false) => {}
            }
            dead.push(handle);
        }

        iface.poll(Instant::now(), &mut device, &mut sockets);

        for handle in dead.drain(..) {
            sockets.remove(handle);
            let old = connections.remove(&handle).unwrap();
            ports.unget(old.port);
            debug!("{} cleaning up", old.port);
        }

        let delay = iface.poll_delay(Instant::now(), &sockets);
        trace!("waiting {:?}", delay);
        wait(
            &listener,
            device.get_rx(),
            &connections,
            delay.unwrap_or(Duration::from_secs(1)),
        )
        .unwrap()
    }
}

fn wait(
    listener: &TcpListener,
    rx: &UnixDatagram,
    connections: &HashMap<SocketHandle, Connection>,
    delay: Duration,
) -> nix::Result<()> {
    let mut timeout = TimeVal::new(delay.secs() as time_t, delay.micros() as suseconds_t);
    let mut fds = FdSet::new();
    fds.insert(listener.as_fd());
    fds.insert(rx.as_fd());
    for conn in connections.values() {
        fds.insert(conn.socket.as_fd())
    }
    select(None, &mut fds, None, None, Some(&mut timeout)).map(|_| ())
}
