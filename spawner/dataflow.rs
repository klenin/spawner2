use crate::pipe::{ReadPipe, WritePipe};
use crate::{Error, Result};

use std::collections::HashMap;
use std::fmt;
use std::io::{BufWriter, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct SourceId(usize);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct DestinationId(usize);

pub trait OnRead: Send {
    fn on_read(&mut self, data: &[u8], dsts: &mut [Connection]) -> Result<()>;
}

#[derive(Debug)]
enum ConnectionKind {
    Pipe(WritePipe),
    File(BufWriter<WritePipe>),
}

pub struct Connection {
    kind: Arc<Mutex<ConnectionKind>>,
    is_dead: bool,
    src_id: SourceId,
    dst_id: DestinationId,
}

pub struct Destination {
    connection_kind: Arc<Mutex<ConnectionKind>>,
    edges: Vec<SourceId>,
}

pub struct Source {
    pipe: ReadPipe,
    connections: Vec<Connection>,
    edges: Vec<DestinationId>,
    on_read: Option<Box<OnRead>>,
}

pub struct Graph {
    srcs: HashMap<SourceId, Source>,
    dsts: HashMap<DestinationId, Destination>,
    src_id_generator: usize,
    dst_id_generator: usize,
}

#[derive(Debug)]
pub struct Errors {
    pub errors: HashMap<SourceId, Error>,
}

pub struct Transmitter {
    readers: Vec<(SourceId, JoinHandle<Result<ReadPipe>>)>,
    _file_dsts: Vec<Destination>,
}

impl ConnectionKind {
    fn is_file(&self) -> bool {
        match self {
            ConnectionKind::File(_) => true,
            _ => false,
        }
    }
}

impl Connection {
    pub fn destination_id(&self) -> DestinationId {
        self.dst_id
    }

    pub fn source_id(&self) -> SourceId {
        self.src_id
    }

    pub fn send(&mut self, data: &[u8]) {
        if self.is_dead {
            return;
        }
        let result = match *self.kind.lock().unwrap() {
            ConnectionKind::Pipe(ref mut p) => p.write_all(data),
            ConnectionKind::File(ref mut f) => f.write_all(data),
        };
        if result.is_err() {
            self.is_dead = true;
        }
    }

    fn is_dead(&self) -> bool {
        self.is_dead
    }
}

impl Source {
    pub fn edges(&self) -> &[DestinationId] {
        &self.edges
    }

    pub fn has_handler(&self) -> bool {
        self.on_read.is_some()
    }

    pub fn set_handler<T>(&mut self, on_read: T)
    where
        T: OnRead + 'static,
    {
        self.on_read = Some(Box::new(on_read));
    }
}

impl Destination {
    pub fn edges(&self) -> &[SourceId] {
        &self.edges
    }
}

impl Graph {
    pub fn new() -> Self {
        Self {
            srcs: HashMap::new(),
            dsts: HashMap::new(),
            src_id_generator: 0,
            dst_id_generator: 0,
        }
    }

    pub fn add_source(&mut self, src: ReadPipe) -> SourceId {
        let id = self.generate_src_id();
        self.srcs.insert(
            id,
            Source {
                pipe: src,
                connections: Vec::new(),
                edges: Vec::new(),
                on_read: None,
            },
        );
        id
    }

    pub fn source(&mut self, id: SourceId) -> Option<&Source> {
        self.srcs.get(&id)
    }

    pub fn source_mut(&mut self, id: SourceId) -> Option<&mut Source> {
        self.srcs.get_mut(&id)
    }

    pub fn remove_source(&mut self, id: SourceId) -> Option<ReadPipe> {
        self.srcs.remove(&id).map(|src| {
            for edge in src.edges.iter() {
                let dst_edges = &mut self.dsts.get_mut(edge).unwrap().edges;
                let src_idx = dst_edges.iter().position(|&i| i == id).unwrap();
                dst_edges.swap_remove(src_idx);
            }
            src.pipe
        })
    }

    fn add_dst_impl(&mut self, kind: ConnectionKind) -> DestinationId {
        let id = self.generate_dst_id();
        self.dsts.insert(
            id,
            Destination {
                connection_kind: Arc::new(Mutex::new(kind)),
                edges: Vec::new(),
            },
        );
        id
    }

    pub fn add_destination(&mut self, dst: WritePipe) -> DestinationId {
        self.add_dst_impl(ConnectionKind::Pipe(dst))
    }

    pub fn add_file_destination(&mut self, file: WritePipe) -> DestinationId {
        self.add_dst_impl(ConnectionKind::File(BufWriter::new(file)))
    }

    pub fn destination(&self, id: DestinationId) -> Option<&Destination> {
        self.dsts.get(&id)
    }

    pub fn remove_destination(&mut self, id: DestinationId) -> Option<WritePipe> {
        self.dsts.remove(&id).map(|dst| {
            for edge in dst.edges.iter() {
                let src = self.srcs.get_mut(edge).unwrap();

                let dst_idx = src.edges.iter().position(|&i| i == id).unwrap();
                src.edges.swap_remove(dst_idx);

                let dst_idx = src.connections.iter().position(|c| c.dst_id == id).unwrap();
                src.connections.swap_remove(dst_idx);
            }
            match Arc::try_unwrap(dst.connection_kind)
                .unwrap()
                .into_inner()
                .unwrap()
            {
                ConnectionKind::Pipe(p) => p,
                ConnectionKind::File(f) => f.into_inner().unwrap(),
            }
        })
    }

    pub fn connect(&mut self, src_id: SourceId, dst_id: DestinationId) {
        let src = self.srcs.get_mut(&src_id).unwrap();
        let dst = self.dsts.get_mut(&dst_id).unwrap();
        if src.edges.iter().any(|&id| id == dst_id) {
            return;
        }
        dst.edges.push(src_id);
        src.edges.push(dst_id);
        src.connections.push(Connection {
            kind: dst.connection_kind.clone(),
            src_id: src_id,
            dst_id: dst_id,
            is_dead: false,
        })
    }

    pub fn transmit_data(self) -> Transmitter {
        let file_dsts = self
            .dsts
            .into_iter()
            .filter_map(|(_, dst)| {
                if dst.connection_kind.lock().unwrap().is_file() {
                    Some(dst)
                } else {
                    None
                }
            })
            .collect();
        Transmitter {
            readers: self
                .srcs
                .into_iter()
                .map(|(id, src)| (id, thread::spawn(move || read_source(src))))
                .collect(),
            _file_dsts: file_dsts,
        }
    }

    fn generate_src_id(&mut self) -> SourceId {
        self.src_id_generator += 1;
        SourceId(self.src_id_generator)
    }

    fn generate_dst_id(&mut self) -> DestinationId {
        self.dst_id_generator += 1;
        DestinationId(self.dst_id_generator)
    }
}

impl std::error::Error for Errors {}

impl fmt::Display for Errors {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for e in self.errors.values() {
            writeln!(f, "{}", e)?;
        }
        Ok(())
    }
}

impl Transmitter {
    /// Transmitter keeps pipes open as long as possible,
    /// so make sure it is destroyed before `Spawner`.
    ///
    /// [`Spawner`]: struct.Spawner.html
    pub fn wait(self) -> std::result::Result<(), Errors> {
        let errors = self
            .readers
            .into_iter()
            .filter_map(|(id, reader)| {
                match reader
                    .join()
                    .unwrap_or(Err(Error::from("Source reader panicked")))
                {
                    Ok(_) => None,
                    Err(e) => Some((id, e)),
                }
            })
            .collect::<HashMap<_, _>>();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(Errors { errors: errors })
        }
    }
}

fn read_source(mut src: Source) -> Result<ReadPipe> {
    let mut buffer: Vec<u8> = Vec::new();
    buffer.resize(8192, 0);

    loop {
        let data = match src.pipe.read(buffer.as_mut_slice()) {
            Ok(bytes_read) => &buffer[..bytes_read],
            Err(_) => break,
        };
        if data.is_empty() {
            break;
        }

        match src.on_read {
            Some(ref mut handler) => handler.on_read(data, &mut src.connections)?,
            None => src
                .connections
                .iter_mut()
                .for_each(|connection| connection.send(data)),
        }
        if src.connections.iter().all(Connection::is_dead) {
            break;
        }
    }

    Ok(src.pipe)
}
