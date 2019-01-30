use crate::{Error, Result};
use pipe::{ReadPipe, WritePipe};
use std::io::{self, Read, Write};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// This structure splits the `ReadPipe` allowing multiple readers to receive data from it.
pub struct ReadHub {
    src: ReadPipe,
    channels: Vec<Sender<Message>>,
    buffer_size: usize,
}

struct WriteHubInner {
    dst: WritePipe,
    receiver: Receiver<Message>,
    buffer: Vec<u8>,
    buffer_size: usize,
}

/// This structure splits the `WritePipe` allowing multiple writers to send data to it.
pub struct WriteHub {
    inner: WriteHubInner,
    sender: Sender<Message>,
}

pub struct StopHandle {
    thread: JoinHandle<()>,
}

#[derive(Clone)]
struct Message {
    content: Arc<Vec<u8>>,
}

impl ReadHub {
    pub fn new(src: ReadPipe) -> Self {
        Self {
            src: src,
            channels: Vec::new(),
            buffer_size: 4096,
        }
    }

    pub fn connect(&mut self, wh: &WriteHub) {
        self.channels.push(wh.sender.clone());
    }

    pub fn start(self) -> Result<StopHandle> {
        Ok(StopHandle {
            thread: thread::Builder::new().spawn(move || Self::main_loop(self))?,
        })
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

            let message = Message::new(&buffer[..bytes_read]);
            let mut errors = 0;
            for channel in &self.channels {
                if let Err(_) = channel.send(message.clone()) {
                    errors += 1;
                }
            }

            if errors == self.channels.len() {
                // All receivers are dead.
                return;
            }
        }
    }
}

impl WriteHub {
    pub fn new(dst: WritePipe) -> Self {
        let (s, r) = channel::<Message>();
        Self {
            inner: WriteHubInner {
                dst: dst,
                receiver: r,
                buffer: Vec::new(),
                buffer_size: 4096,
            },
            sender: s,
        }
    }

    pub fn connect(&self, rh: &mut ReadHub) {
        rh.connect(self);
    }

    pub fn start(self) -> io::Result<StopHandle> {
        // Split self into inner and _sender, so we can drop the sender.
        // If we don't do that we'll hang on recv() call since there will be always one sender left.
        let inner = self.inner;
        let _sender = self.sender;
        Ok(StopHandle {
            thread: thread::Builder::new().spawn(move || WriteHubInner::main_loop(inner))?,
        })
    }
}

impl WriteHubInner {
    fn bufferize(&mut self, msg: Message) {
        self.buffer.extend_from_slice(msg.get());
    }

    fn try_fill_buffer(&mut self) {
        for _ in 0..50 {
            match self.receiver.recv_timeout(Duration::from_millis(1)) {
                Ok(msg) => {
                    self.bufferize(msg);
                    if self.buffer.len() > self.buffer_size {
                        break;
                    }
                }
                Err(e) => {
                    if e == RecvTimeoutError::Disconnected {
                        break;
                    }
                }
            }
        }
    }

    fn main_loop(mut self) {
        self.buffer = Vec::with_capacity(self.buffer_size);
        loop {
            self.buffer.clear();
            let msg = match self.receiver.recv() {
                Ok(x) => x,
                Err(_) => return,
            };

            let data = if msg.get().len() == self.buffer_size {
                msg.get()
            } else {
                self.bufferize(msg);
                self.try_fill_buffer();
                &self.buffer
            };

            if self.dst.write_all(data).is_err() || self.dst.flush().is_err() {
                return;
            }
        }
    }
}

impl StopHandle {
    pub fn stop(self) -> Result<()> {
        self.thread
            .join()
            .map_err(|_| Error::from("monitoring thread panicked"))
    }
}

impl Message {
    pub fn new(content: &[u8]) -> Self {
        Self {
            content: Arc::new(content.to_vec()),
        }
    }

    pub fn get(&self) -> &[u8] {
        self.content.as_slice()
    }
}
