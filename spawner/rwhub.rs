use crate::pipe::{ReadPipe, WritePipe};
use crate::{Error, Result};

use std::io::{self, BufWriter, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

pub trait OnRead: Send {
    fn on_read(&mut self, data: &[u8], connections: &mut [Connection]) -> Result<()>;
}

pub struct ReaderThread(JoinHandle<Result<ReadPipe>>);

/// Splits the [`ReadPipe`] allowing multiple readers to receive data from it.
///
/// [`ReadPipe`]: struct.ReadPipe.html
pub struct ReadHub {
    pipe: ReadPipe,
    connections: Vec<Connection>,
    on_read: Option<Box<OnRead>>,
}

/// Represents connection between [`ReadHub`] and [`WriteHub`].
///
/// [`ReadHub`]: struct.ReadHub.html
/// [`WriteHub`]: struct.WriteHub.html
pub struct Connection {
    wh: WriteHub,
    is_dead: bool,
}

enum WriteHubDst {
    Pipe(WritePipe),
    File(BufWriter<WritePipe>),
}

/// Allows multiple writers to send data to the [`WritePipe`].
///
/// [`WritePipe`]: struct.WritePipe.html
#[derive(Clone)]
pub struct WriteHub(Arc<Mutex<WriteHubDst>>);

impl ReaderThread {
    pub fn join(self) -> Result<ReadPipe> {
        self.0
            .join()
            .unwrap_or(Err(Error::from("ReaderThread panicked")))
    }
}

impl ReadHub {
    pub fn new(pipe: ReadPipe) -> Self {
        Self {
            pipe: pipe,
            connections: Vec::new(),
            on_read: None,
        }
    }

    pub fn set_on_read<T>(&mut self, on_read: T)
    where
        T: OnRead + 'static,
    {
        self.on_read = Some(Box::new(on_read));
    }

    pub fn connect(&mut self, wh: &WriteHub) {
        self.connections.push(Connection {
            wh: wh.clone(),
            is_dead: false,
        });
    }

    pub fn start_reading(mut self) -> ReaderThread {
        ReaderThread(thread::spawn(move || {
            let mut buffer: Vec<u8> = Vec::new();
            buffer.resize(8192, 0);

            loop {
                let bytes_read = match self.read(buffer.as_mut_slice()) {
                    Ok(x) => x,
                    Err(_) => break,
                };
                if bytes_read == 0 {
                    break;
                }

                self.transmit(&buffer[..bytes_read])?;
                if self.connections.iter().all(Connection::is_dead) {
                    break;
                }
            }
            Ok(self.pipe)
        }))
    }

    fn transmit(&mut self, data: &[u8]) -> Result<()> {
        let connections = &mut self.connections;
        match self.on_read {
            Some(ref mut handler) => handler.on_read(data, connections),
            None => {
                for c in connections {
                    c.send(data);
                }
                Ok(())
            }
        }
    }
}

impl Read for ReadHub {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.pipe.read(buf)
    }
}

impl Connection {
    pub fn send(&mut self, data: &[u8]) {
        if self.wh.write_all(data).is_err() {
            self.is_dead = true;
        }
    }

    pub fn is_dead(&self) -> bool {
        self.is_dead
    }
}

impl WriteHub {
    pub fn from_pipe(pipe: WritePipe) -> Self {
        Self(Arc::new(Mutex::new(WriteHubDst::Pipe(pipe))))
    }

    pub fn from_file(file: WritePipe) -> Self {
        Self(Arc::new(Mutex::new(WriteHubDst::File(BufWriter::new(
            file,
        )))))
    }

    fn lock(&self) -> io::Result<MutexGuard<WriteHubDst>> {
        self.0
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "WriteHub mutex was poisoned"))
    }
}

impl Write for WriteHub {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self.lock()? {
            WriteHubDst::Pipe(ref mut p) => p.write(buf),
            WriteHubDst::File(ref mut f) => f.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self.lock()? {
            WriteHubDst::Pipe(ref mut p) => p.flush(),
            WriteHubDst::File(ref mut f) => f.flush(),
        }
    }
}
