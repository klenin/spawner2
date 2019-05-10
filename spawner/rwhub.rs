use crate::pipe::{ReadPipe, WritePipe};
use crate::{Error, Result};

use std::io::{self, BufWriter, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

/// Splits the [`ReadPipe`] allowing multiple readers to receive data from it.
///
/// [`ReadPipe`]: struct.ReadPipe.html
pub struct ReadHub {
    pipe: ReadPipe,
    controller: Option<Box<(ReadHubController)>>,
    write_hubs: Vec<WriteHub>,
    buffer_size: usize,
}

/// Allows dynamic control over data flow in [`ReadHub`].
///
/// [`ReadHub`]: struct.ReadHub.html
pub trait ReadHubController: Send {
    fn handle_data(&mut self, data: &[u8], write_hubs: &mut [WriteHub]) -> Result<()>;
}

/// Continiously reads data from [`ReadHub`] sending it to corresponding receivers.
/// Exits if one of the following conditions is met:
/// * Error reading from [`ReadHub`].
/// * EOF encountered.
/// * [`ReadHubController::handle_data`] returned error.
/// * All receivers are dead.
///
/// [`ReadHub`]: struct.ReadHub.html
/// [`ReadHubController::handle_data`]: trait.ReadHubController.html#method.handle_data
pub struct ReadHubThread(JoinHandle<Result<ReadPipe>>);

enum WriteHubDst {
    Pipe(WritePipe),
    File(BufWriter<WritePipe>),
}

/// Allows multiple writers to send data to the [`WritePipe`].
///
/// [`WritePipe`]: struct.WritePipe.html
#[derive(Clone)]
pub struct WriteHub {
    dst: Arc<Mutex<WriteHubDst>>,
    error_encountered: bool,
}

impl ReadHub {
    pub fn new(pipe: ReadPipe, controller: Option<Box<ReadHubController>>) -> Self {
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

    pub fn write_hubs(&self) -> &Vec<WriteHub> {
        &self.write_hubs
    }
}

impl ReadHubThread {
    pub fn spawn(rh: ReadHub) -> Result<Self> {
        thread::Builder::new()
            .spawn(move || ReadHubThread::entry(rh))
            .map_err(|_| Error::from("Cannot spawn ReadHubThread"))
            .map(Self)
    }

    pub fn join(self) -> Result<ReadPipe> {
        self.0
            .join()
            .unwrap_or(Err(Error::from("ReadHub thread panicked")))
    }

    fn entry(mut rh: ReadHub) -> Result<ReadPipe> {
        let mut buffer: Vec<u8> = Vec::new();
        buffer.resize(rh.buffer_size, 0);

        loop {
            let bytes_read = match rh.pipe.read(buffer.as_mut_slice()) {
                Ok(x) => x,
                Err(_) => break,
            };
            if bytes_read == 0 {
                break;
            }

            let data = &buffer[..bytes_read];
            if let Some(ctl) = &mut rh.controller {
                ctl.handle_data(data, rh.write_hubs.as_mut_slice())?;
            } else {
                for wh in rh.write_hubs.iter_mut() {
                    wh.write_all(data);
                }
            }

            if rh.write_hubs.iter().all(|wh| wh.error_encountered) {
                break;
            }
        }

        Ok(rh.pipe)
    }
}

impl Write for WriteHubDst {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            WriteHubDst::Pipe(p) => p.write(buf),
            WriteHubDst::File(f) => f.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            WriteHubDst::Pipe(p) => p.flush(),
            WriteHubDst::File(f) => f.flush(),
        }
    }
}

impl WriteHub {
    pub fn new(pipe: WritePipe) -> Self {
        Self {
            dst: Arc::new(Mutex::new(match pipe.is_file() {
                true => WriteHubDst::File(BufWriter::new(pipe)),
                false => WriteHubDst::Pipe(pipe),
            })),
            error_encountered: false,
        }
    }

    pub fn write_all(&mut self, data: &[u8]) {
        if self.lock().and_then(|mut dst| dst.write_all(data)).is_err() {
            self.error_encountered = true;
        }
    }

    fn lock(&self) -> io::Result<MutexGuard<WriteHubDst>> {
        self.dst
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "WriteHub mutex was poisoned"))
    }
}
