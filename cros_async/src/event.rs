use crate::{new, AsyncResult, IoSourceExt};
use std::os::unix::io::AsRawFd;

/// An async version of sys_util::EventFd.
pub struct EventAsync<'a, F: AsRawFd + 'a> {
    io_source: Box<dyn IoSourceExt<F> + 'a>,
}

impl<'a, F: AsRawFd + 'a> EventAsync<'a, F> {
    #[allow(dead_code)]
    pub fn new(f: F) -> AsyncResult<EventAsync<'a, F>> {
        Ok(EventAsync { io_source: new(f)? })
    }

    #[allow(dead_code)]
    pub async fn next_val(&mut self) -> AsyncResult<u64> {
        self.io_source.read_u64().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn next_val_reads_value() {
        let f = File::open("/dev/zero").unwrap();
        let event = EventAsync::new(f);
        // TODO(nkgold): implement this test.
    }
}
