use anyhow::Result;
use clap::{command, Arg};
use log::error;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::phy::wait as phy_wait;
use smoltcp::phy::{Loopback, Medium, PcapMode, PcapWriter};
use smoltcp::socket::tcp::{Socket, SocketBuffer};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr};
use std::collections::HashMap;
use std::fs;
use std::io::{stdout, ErrorKind};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::str::FromStr;

struct Connection {
    remote: TcpStream,
}

fn main() -> ! {
    env_logger::init();

    let args = command!()
        .arg(
            Arg::new("hostport")
                .required(true)
                .value_parser(clap::builder::ValueParser::new(SocketAddr::from_str))
                .help("hostport to listen on"),
        )
        .arg(
            Arg::new("socket")
                .required(true)
                .help("path to server socket for QEMU to connect to"),
        )
        .get_matches();

    let socket_path = args.get_one::<String>("socket").unwrap();
    let hostport = args.get_one::<SocketAddr>("hostport").unwrap();

    let listener = TcpListener::bind(hostport).unwrap();
    listener.set_nonblocking(true).unwrap();

    match fs::remove_file(socket_path) {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => Err(e).unwrap(),
    }
    let uds = UnixDatagram::bind(socket_path).unwrap();

    let mut device = PcapWriter::new(Loopback::new(Medium::Ethernet), stdout(), PcapMode::Both);

    let mut config = Config::new(HardwareAddress::Ethernet(EthernetAddress([
        42, 0, 0, 0, 0, 69,
    ])));
    config.random_seed = rand::random();

    let mut iface = Interface::new(config, &mut device, Instant::now());

    iface.update_ip_addrs(|ip_addrs| {
        ip_addrs
            .push(IpCidr::new(IpAddress::v4(192, 168, 69, 1), 24))
            .unwrap();
    });

    let mut sockets = SocketSet::new(vec![]);
    let mut connections = HashMap::new();
    let mut dead = Vec::new();

    loop {
        match listener.accept() {
            Ok((remote, _)) => {
                let handle = sockets.add(Socket::new(
                    SocketBuffer::new(vec![0; 65535]),
                    SocketBuffer::new(vec![0; 65535]),
                ));
                connections.insert(handle, Connection { remote });
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
            connections.remove(&handle);
            sockets.remove(handle);
        }

        phy_wait(
            listener.as_raw_fd(),
            iface.poll_delay(Instant::now(), &sockets),
        )
        .unwrap();
    }
}

fn proxy(local: &mut Socket, remote: &mut Connection) -> Result<bool> {
    todo!()
}
