use crate::Connection;
use smoltcp::socket::tcp::Socket;

pub(crate) fn proxy(local: &mut Socket, remote: &mut Connection) -> anyhow::Result<bool> {
    todo!()
}
