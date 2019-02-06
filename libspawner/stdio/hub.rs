use crate::{Error, Result};
use pipe::{ReadPipe, WritePipe};
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// This structure splits the `ReadPipe` allowing multiple readers to receive data from it.
pub struct ReadHub {
    src: ReadPipe,
    channels: Vec<Sender<Message>>,
    buffer_size: usize,
}

struct WriteHubInner {
    dst: WritePipe,
    receiver: Receiver<Message>,
}

/// This structure splits the `WritePipe` allowing multiple writers to send data to it.
pub struct WriteHub {
    inner: WriteHubInner,
    sender: Sender<Message>,
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
            },
            sender: s,
        }
    }

    pub fn connect(&self, rh: &mut ReadHub) {
        rh.connect(self);
    }

    pub fn spawn(self) -> Result<JoinHandle<()>> {
        // Split self into inner and _sender, so we can drop the sender.
        // If we don't do that we'll hang on recv() call since there will be always one sender left.
        let inner = self.inner;
        let _sender = self.sender;

        thread::Builder::new()
            .spawn(move || WriteHubInner::main_loop(inner))
            .map_err(|e| Error::from(e))
    }
}

impl WriteHubInner {
    fn main_loop(mut self) {
        loop {
            let msg = match self.receiver.recv() {
                Ok(x) => x,
                Err(_) => return,
            };
            if self.dst.write_all(msg.get()).is_err() {
                return;
            }
        }
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
