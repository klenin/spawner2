use crate::{Error, Result};
use pipe::{ReadPipe, WritePipe};
use std::io::{self, Read, Write};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct Splitter {
    src: ReadPipe,
    channels: Vec<Sender<Message>>,
    buffer_size: usize,
}

struct CombinerInner {
    dst: WritePipe,
    receiver: Receiver<Message>,
    buffer: Vec<u8>,
    buffer_size: usize,
}

pub struct Combiner {
    inner: CombinerInner,
    sender: Sender<Message>,
}

pub struct StopHandle {
    thread: JoinHandle<()>,
}

#[derive(Clone)]
struct Message {
    content: Arc<Vec<u8>>,
}

impl Splitter {
    pub fn new(src: ReadPipe) -> Self {
        Self {
            src: src,
            channels: Vec::new(),
            buffer_size: 4096,
        }
    }

    pub fn connect(&mut self, combiner: &Combiner) {
        self.channels.push(combiner.sender.clone());
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

impl Combiner {
    pub fn new(dst: WritePipe) -> Self {
        let (s, r) = channel::<Message>();
        Self {
            inner: CombinerInner {
                dst: dst,
                receiver: r,
                buffer: Vec::new(),
                buffer_size: 4096,
            },
            sender: s,
        }
    }

    pub fn connect(&self, splitter: &mut Splitter) {
        splitter.connect(self);
    }

    pub fn start(self) -> io::Result<StopHandle> {
        Ok(StopHandle {
            thread: thread::Builder::new().spawn(move || CombinerInner::main_loop(self.inner))?,
        })
    }
}

impl CombinerInner {
    fn finish_buffer(&mut self) -> bool {
        let mut written = 0;
        while written != self.buffer.len() {
            let remaining_data = &self.buffer[written..];
            written += match write_and_flush(&mut self.dst, remaining_data) {
                Ok(bytes) => bytes,
                Err(_) => return false,
            };
        }
        self.buffer.clear();
        true
    }

    fn bufferize(&mut self, msg: Message) {
        self.buffer.extend_from_slice(msg.get());
    }

    fn try_fill_buffer(&mut self) -> bool {
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
                        return false;
                    }
                }
            }
        }
        true
    }

    fn write(&mut self, msg: Message) -> bool {
        let data = msg.get();
        let bytes_written = match write_and_flush(&mut self.dst, data) {
            Ok(x) => x,
            Err(_) => return false,
        };
        if bytes_written != data.len() {
            self.buffer.extend_from_slice(&data[bytes_written..]);
        }
        true
    }

    fn main_loop(mut self) {
        self.buffer = Vec::with_capacity(self.buffer_size);
        loop {
            if !self.finish_buffer() {
                return;
            }

            let msg = match self.receiver.recv() {
                Ok(x) => x,
                Err(_) => return,
            };

            let succeeded = if msg.get().len() == self.buffer_size {
                self.write(msg)
            } else {
                self.bufferize(msg);
                self.try_fill_buffer()
            };

            if !succeeded {
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

fn write_and_flush(pipe: &mut WritePipe, data: &[u8]) -> io::Result<usize> {
    let bytes = pipe.write(data)?;
    pipe.flush()?;
    Ok(bytes)
}
