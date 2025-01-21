mod port_alloc;
mod proxy;

use clap::{command, Arg};
use log::error;
use port_alloc::*;
use proxy::*;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::phy::wait as phy_wait;
use smoltcp::phy::{Loopback, Medium, PcapMode, PcapWriter};
use smoltcp::socket::tcp::{Socket, SocketBuffer};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr};
use std::collections::HashMap;
use std::fs;
use std::io::{stdout, ErrorKind};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::str::FromStr;

struct Connection {
    socket: TcpStream,
    port: u16,
}

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
            Arg::new("socket")
                .long("socket")
                .required(true)
                .help("path to server socket for QEMU to connect to"),
        )
        .arg(
            Arg::new("listen")
                .long("listen")
                .required(true)
                .value_parser(clap::builder::ValueParser::new(SocketAddr::from_str))
                .help("local address to listen on"),
        )
        .arg(
            Arg::new("remote")
                .long("remote")
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

    let socket_path = args.get_one::<String>("socket").unwrap();
    let listen_addr = args.get_one::<SocketAddr>("listen").unwrap();
    let remote_addr = args.get_one::<SocketAddr>("remote").unwrap();
    let local_addr = args.get_one::<IpCidr>("local").unwrap();
    let mac_addr = args.get_one::<EthernetAddress>("mac").unwrap();

    let listener = TcpListener::bind(listen_addr).unwrap();
    listener.set_nonblocking(true).unwrap();

    match fs::remove_file(socket_path) {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => Err(e).unwrap(),
    }
    let uds = UnixDatagram::bind(socket_path).unwrap();

    // TODO device
    let mut device = PcapWriter::new(Loopback::new(Medium::Ethernet), stdout(), PcapMode::Both);

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
                remote.set_nonblocking(true).unwrap();

                let port = ports.get();

                let mut sock = Socket::new(
                    SocketBuffer::new(vec![0; 65535]),
                    SocketBuffer::new(vec![0; 65535]),
                );

                sock.connect(iface.context(), *remote_addr, port).unwrap();

                let handle = sockets.add(sock);

                connections.insert(
                    handle,
                    Connection {
                        socket: remote,
                        port,
                    },
                );
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => Err(e).unwrap(),
        }

        iface.poll(Instant::now(), &mut device, &mut sockets);

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

        for handle in dead.drain(..) {
            let old = connections.remove(&handle).unwrap();
            ports.unget(old.port);
            sockets.remove(handle);
        }

        phy_wait(
            listener.as_raw_fd(),
            iface.poll_delay(Instant::now(), &sockets),
        )
        .unwrap();
    }
}
