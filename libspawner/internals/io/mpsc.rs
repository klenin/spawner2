use std::io::BufWriter;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone)]
struct Message {
    content: Arc<Vec<u8>>,
}

pub struct Sender<R>
where
    R: Read + Send + 'static,
{
    source: R,
    senders: Vec<mpsc::Sender<Message>>,
    stopped: Arc<AtomicBool>,
    buffer_size: usize,
}

pub struct Receiver<W>
where
    W: Write + Send + 'static,
{
    destination: W,
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    stopped: Arc<AtomicBool>,
    buffer_size: usize,
}

pub struct StopHandle {
    thread: JoinHandle<()>,
    stopped: Weak<AtomicBool>,
}

impl<'a> Message {
    pub fn new(content: &[u8]) -> Self {
        Self {
            content: Arc::new(content.to_vec()),
        }
    }

    pub fn get(&self) -> &[u8] {
        self.content.as_slice()
    }
}

impl<R> Sender<R>
where
    R: Read + Send + 'static,
{
    pub fn new(source: R) -> Self {
        Self {
            source: source,
            senders: Vec::new(),
            stopped: Arc::new(AtomicBool::new(false)),
            buffer_size: 8096,
        }
    }

    pub fn send_to<W>(&mut self, receiver: &Receiver<W>)
    where
        W: Write + Send + 'static,
    {
        self.senders.push(receiver.sender.clone())
    }

    pub fn start(self) -> io::Result<StopHandle> {
        let stopped = Arc::downgrade(&self.stopped);
        Ok(StopHandle {
            thread: thread::Builder::new().spawn(move || Self::main_loop(self))?,
            stopped: stopped,
        })
    }

    fn main_loop(mut self) {
        let mut buffer: Vec<u8> = Vec::new();
        buffer.resize(self.buffer_size, 0);

        while !self.stopped.load(Ordering::SeqCst) {
            if let Ok(bytes_read) = self.source.read(buffer.as_mut_slice()) {
                if bytes_read != 0 {
                    let message = Message::new(&buffer[..bytes_read]);
                    for sender in &self.senders {
                        let _ = sender.send(message.clone());
                    }
                }
            } else {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}

impl<W> Receiver<W>
where
    W: Write + Send + 'static,
{
    pub fn new(destination: W) -> Self {
        let (s, r) = mpsc::channel::<Message>();
        Self {
            destination: destination,
            sender: s,
            receiver: r,
            stopped: Arc::new(AtomicBool::new(false)),
            buffer_size: 8096,
        }
    }

    pub fn receive_from<R>(&self, sender: &mut Sender<R>)
    where
        R: Read + Send + 'static,
    {
        sender.send_to(self);
    }

    pub fn start(self) -> io::Result<StopHandle> {
        let stopped = Arc::downgrade(&self.stopped);
        Ok(StopHandle {
            thread: thread::Builder::new().spawn(move || Self::main_loop(self))?,
            stopped: stopped,
        })
    }

    fn main_loop(self) {
        let mut buf = BufWriter::with_capacity(self.buffer_size, self.destination);
        while !self.stopped.load(Ordering::SeqCst) {
            while let Some(msg) = self.receiver.try_iter().take(10).next() {
                if let Err(_) = buf.write(msg.get()).and(buf.flush()) {
                    return;
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}

impl StopHandle {
    pub fn stop(self) -> io::Result<()> {
        if let Some(stopped) = self.stopped.upgrade() {
            stopped.store(true, Ordering::SeqCst);
        }
        self.thread
            .join()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "monitoring thread panicked"))
    }
}
