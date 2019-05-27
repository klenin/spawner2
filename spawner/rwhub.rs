use crate::pipe::{ReadPipe, WritePipe};

use std::io::{self, BufWriter, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};

/// Splits the [`ReadPipe`] allowing multiple readers to receive data from it.
///
/// [`ReadPipe`]: struct.ReadPipe.html
pub struct ReadHub {
    pipe: ReadPipe,
    connections: Vec<Connection>,
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

impl ReadHub {
    pub fn new(pipe: ReadPipe) -> Self {
        Self {
            pipe: pipe,
            connections: Vec::new(),
        }
    }

    pub fn connect(&mut self, wh: &WriteHub) {
        self.connections.push(Connection {
            wh: wh.clone(),
            is_dead: false,
        });
    }

    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    pub fn connections_mut(&mut self) -> &mut [Connection] {
        &mut self.connections
    }

    pub fn transmit(&mut self, data: &[u8]) {
        for c in self.connections.iter_mut() {
            c.send(data);
        }
    }

    pub fn into_inner(self) -> ReadPipe {
        self.pipe
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
