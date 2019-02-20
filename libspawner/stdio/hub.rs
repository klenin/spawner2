use crate::{Error, Result};
use pipe::{ReadPipe, WritePipe};
use std::io;
use std::io::{BufWriter, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

/// Splits the `ReadPipe` allowing multiple readers to receive data from it.
pub struct ReadHub {
    src: ReadPipe,
    write_hubs: Vec<WriteHub>,
    buffer_size: usize,
}

enum WriteHubKind {
    Pipe(WritePipe),
    File(BufWriter<WritePipe>),
}

/// Allows multiple writers to send data to the `WritePipe`.
#[derive(Clone)]
pub struct WriteHub {
    kind: Arc<Mutex<WriteHubKind>>,
}

impl ReadHub {
    pub fn new(src: ReadPipe) -> Self {
        Self {
            src: src,
            write_hubs: Vec::new(),
            buffer_size: 8192,
        }
    }

    pub fn connect(&mut self, wh: &WriteHub) {
        self.write_hubs.push(wh.clone());
    }

    pub fn spawn(self) -> Result<JoinHandle<()>> {
        thread::Builder::new()
            .spawn(move || Self::main_loop(self))
            .map_err(|e| Error::from(e))
    }

    fn main_loop(mut self) {
        let mut buffer: Vec<u8> = Vec::new();
        buffer.resize(self.buffer_size, 0);

        loop {
            let bytes_read = match self.src.read(buffer.as_mut_slice()) {
                Ok(x) => x,
                Err(_) => return,
            };

            if bytes_read == 0 {
                // Assuming pipes are blocking.
                // So we waited on the pipe, got 0 bytes and no errors.
                // That case is (probably) unreachable.
                return;
            }

            let mut errors = 0;
            for wh in &mut self.write_hubs {
                if wh.write_all(&buffer[..bytes_read]).is_err() {
                    errors += 1;
                }
            }
            if errors == self.write_hubs.len() {
                // All receivers are dead.
                return;
            }
        }
    }
}

impl WriteHub {
    pub fn new(dst: WritePipe) -> Self {
        Self {
            kind: Arc::new(Mutex::new(match dst.is_file() {
                true => WriteHubKind::File(BufWriter::new(dst)),
                false => WriteHubKind::Pipe(dst),
            })),
        }
    }

    fn lock(&self) -> io::Result<MutexGuard<WriteHubKind>> {
        self.kind
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "WriteHub mutex was poisoned"))
    }
}

impl Write for WriteHub {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self.lock()? {
            WriteHubKind::Pipe(ref mut p) => p.write(buf),
            WriteHubKind::File(ref mut f) => f.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self.lock()? {
            WriteHubKind::Pipe(ref mut p) => p.flush(),
            WriteHubKind::File(ref mut f) => f.flush(),
        }
    }
}
