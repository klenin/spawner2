use crate::{Error, Result};
use pipe::WritePipe;
use std::io::{self, BufWriter, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use stdio::{Istream, IstreamListeners, Ostream, OstreamIdx};

/// Splits the `ReadPipe` allowing multiple readers to receive data from it.
pub struct ReadHub {
    istream: Istream,
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
    ostream_idx: OstreamIdx,
}

impl ReadHub {
    pub fn new(istream: Istream) -> Self {
        Self {
            istream: istream,
            write_hubs: Vec::new(),
            buffer_size: 8192,
        }
    }

    pub fn connect(&mut self, wh: &WriteHub) {
        self.write_hubs.push(wh.clone());
    }

    pub fn spawn(self) -> Result<JoinHandle<Result<()>>> {
        thread::Builder::new()
            .spawn(move || Self::main_loop(self))
            .map_err(|e| Error::from(e))
    }

    fn main_loop(mut self) -> Result<()> {
        let mut buffer: Vec<u8> = Vec::new();
        buffer.resize(self.buffer_size, 0);

        loop {
            let bytes_read = match self.istream.pipe.read(buffer.as_mut_slice()) {
                Ok(x) => x,
                Err(_) => return Ok(()),
            };
            if bytes_read == 0 {
                return Ok(());
            }
            let data = &buffer[..bytes_read];
            let num_errors = match &mut self.istream.controller {
                Some(ctl) => {
                    let mut listeners = IstreamListeners {
                        write_hubs: &mut self.write_hubs,
                        num_errors: 0,
                    };
                    ctl.handle_data(data, &mut listeners)?;
                    listeners.num_errors
                }
                None => {
                    let mut num_errors = 0;
                    for wh in self.write_hubs.iter_mut() {
                        if wh.write_all(data).is_err() {
                            num_errors += 1;
                        }
                    }
                    num_errors
                }
            };

            if num_errors == self.write_hubs.len() {
                // All pipes are dead.
                return Ok(());
            }
        }
    }
}

impl WriteHub {
    pub fn new(ostream: Ostream, idx: OstreamIdx) -> Self {
        let pipe = ostream.pipe;
        Self {
            kind: Arc::new(Mutex::new(match pipe.is_file() {
                true => WriteHubKind::File(BufWriter::new(pipe)),
                false => WriteHubKind::Pipe(pipe),
            })),
            ostream_idx: idx,
        }
    }

    pub fn ostream_idx(&self) -> OstreamIdx {
        self.ostream_idx
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
