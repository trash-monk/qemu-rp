use std::collections::HashSet;

pub(crate) struct PortAlloc {
    active: HashSet<u16>,
}

impl PortAlloc {
    pub(crate) fn new() -> PortAlloc {
        PortAlloc {
            active: HashSet::new(),
        }
    }

    pub(crate) fn get(&mut self) -> u16 {
        loop {
            let port = rand::random::<u16>() % 16383 + 49152;
            if self.active.insert(port) {
                return port;
            }
        }
    }

    pub(crate) fn unget(&mut self, port: u16) {
        assert!(self.active.remove(&port))
    }
}
