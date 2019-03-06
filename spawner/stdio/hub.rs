use crate::{Error, Result};
use pipe::{ReadPipe, WritePipe};
use std::io::{self, BufWriter, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use stdio::{IstreamController, OstreamIdx, Ostreams};

/// Splits the `ReadPipe` allowing multiple readers to receive data from it.
pub struct ReadHub {
    pipe: ReadPipe,
    controller: Option<Box<IstreamController>>,
    write_hubs: Vec<WriteHub>,
    buffer_size: usize,
}

enum WriteHubKind {
    Pipe(WritePipe),
    File(BufWriter<WritePipe>),
}

pub struct ReadHubError {
    pub error: Error,
    pub pipe: ReadPipe,
}

pub type ReadHubResult = std::result::Result<ReadPipe, ReadHubError>;

/// Allows multiple writers to send data to the `WritePipe`.
#[derive(Clone)]
pub struct WriteHub {
    kind: Arc<Mutex<WriteHubKind>>,
    ostream_idx: OstreamIdx,
    error_encountered: bool,
}

impl ReadHub {
    pub fn new(pipe: ReadPipe, controller: Option<Box<IstreamController>>) -> Self {
        Self {
            pipe: pipe,
            controller: controller,
            write_hubs: Vec::new(),
            buffer_size: 8192,
        }
    }

    pub fn connect(&mut self, wh: &WriteHub) {
        self.write_hubs.push(wh.clone());
    }

    pub fn spawn(self) -> Result<JoinHandle<ReadHubResult>> {
        thread::Builder::new()
            .spawn(move || Self::main_loop(self))
            .map_err(|e| Error::from(e))
    }

    fn main_loop(mut self) -> ReadHubResult {
        let mut buffer: Vec<u8> = Vec::new();
        buffer.resize(self.buffer_size, 0);

        loop {
            let bytes_read = match self.pipe.read(buffer.as_mut_slice()) {
                Ok(x) => x,
                Err(_) => break,
            };
            if bytes_read == 0 {
                break;
            }

            let data = &buffer[..bytes_read];
            if let Some(ctl) = &mut self.controller {
                if let Err(e) = ctl.handle_data(data, Ostreams(self.write_hubs.as_mut_slice())) {
                    return Err(ReadHubError {
                        error: e,
                        pipe: self.pipe,
                    });
                }
            } else {
                for wh in self.write_hubs.iter_mut() {
                    let _ = wh.write_all(data);
                }
            }

            if self.write_hubs.iter().all(|wh| wh.error_encountered) {
                break;
            }
        }

        Ok(self.pipe)
    }
}

impl WriteHub {
    pub fn new(pipe: WritePipe, idx: OstreamIdx) -> Self {
        Self {
            kind: Arc::new(Mutex::new(match pipe.is_file() {
                true => WriteHubKind::File(BufWriter::new(pipe)),
                false => WriteHubKind::Pipe(pipe),
            })),
            ostream_idx: idx,
            error_encountered: false,
        }
    }

    pub fn ostream_idx(&self) -> OstreamIdx {
        self.ostream_idx
    }

    pub fn is_file(&self) -> bool {
        match *self.lock().unwrap() {
            WriteHubKind::File(_) => true,
            _ => false,
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
        let result = self.lock().and_then(|mut guard| match *guard {
            WriteHubKind::Pipe(ref mut p) => p.write(buf),
            WriteHubKind::File(ref mut f) => f.write(buf),
        });
        if result.is_err() {
            self.error_encountered = true;
        }
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        let result = self.lock().and_then(|mut guard| match *guard {
            WriteHubKind::Pipe(ref mut p) => p.flush(),
            WriteHubKind::File(ref mut f) => f.flush(),
        });
        if result.is_err() {
            self.error_encountered = true;
        }
        result
    }
}
