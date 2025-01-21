use std::collections::HashSet;

pub(crate) struct PortMapper {
    active: HashSet<u16>,
}

impl PortMapper {
    pub(crate) fn new() -> PortMapper {
        PortMapper {
            active: HashSet::new(),
        }
    }

    pub(crate) fn get(&mut self) -> u16 {
        todo!()
    }

    pub(crate) fn unget(&mut self, port: u16) {
        todo!()
    }
}
