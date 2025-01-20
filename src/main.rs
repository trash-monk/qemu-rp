use clap::{command, Arg};
use smoltcp::iface::{Config, Interface};
use smoltcp::phy::{Loopback, Medium, PcapMode, PcapWriter};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr};
use std::fs;
use std::io::{stdout, ErrorKind};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::net::UnixDatagram;
use std::str::FromStr;

fn main() -> ! {
    env_logger::init();

    let mut device = PcapWriter::new(Loopback::new(Medium::Ethernet), stdout(), PcapMode::RxOnly);

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

    match fs::remove_file(socket_path) {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => Err(e).unwrap(),
    }
    let uds = UnixDatagram::bind(socket_path).unwrap();

    loop {
        let (stream, addr) = listener.accept().unwrap();
    }
}
