use crate::pipe::{ReadPipe, WritePipe};
use crate::{Error, Result};

use std::collections::HashMap;
use std::fmt;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct SourceId(usize);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct DestinationId(usize);

pub trait SourceReader: Send {
    fn read(&mut self, src: &mut ReadPipe, connections: &mut [Connection]) -> Result<()>;
}

#[derive(Debug)]
enum ConnectionKind {
    Pipe(WritePipe),
    File(BufWriter<WritePipe>),
}

enum ConnectionState {
    Alive(Arc<Mutex<ConnectionKind>>),
    Dead,
}

pub struct Connection {
    state: ConnectionState,
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
    reader: Option<Box<dyn SourceReader>>,
}

#[derive(Default)]
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
        let result = match self.state {
            ConnectionState::Alive(ref mut kind) => match *kind.lock().unwrap() {
                ConnectionKind::Pipe(ref mut p) => p.write_all(data),
                ConnectionKind::File(ref mut f) => f.write_all(data),
            },
            ConnectionState::Dead => return,
        };
        if result.is_err() {
            self.state = ConnectionState::Dead;
        }
    }

    fn is_dead(&self) -> bool {
        match self.state {
            ConnectionState::Dead => true,
            _ => false,
        }
    }
}

impl Source {
    pub fn edges(&self) -> &[DestinationId] {
        &self.edges
    }

    pub fn is_connected_to(&self, dst_id: DestinationId) -> bool {
        self.edges().iter().any(|&dst| dst == dst_id)
    }

    pub fn has_reader(&self) -> bool {
        self.reader.is_some()
    }

    pub fn set_reader<T>(&mut self, reader: T)
    where
        T: SourceReader + 'static,
    {
        self.reader = Some(Box::new(reader));
    }
}

impl Destination {
    pub fn edges(&self) -> &[SourceId] {
        &self.edges
    }
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_source(&mut self, src: ReadPipe) -> SourceId {
        let id = self.generate_src_id();
        self.srcs.insert(
            id,
            Source {
                pipe: src,
                connections: Vec::new(),
                edges: Vec::new(),
                reader: None,
            },
        );
        id
    }

    pub fn source(&self, id: SourceId) -> Option<&Source> {
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
            state: ConnectionState::Alive(dst.connection_kind.clone()),
            src_id,
            dst_id,
        })
    }

    pub fn has_connection(&self, src_id: SourceId, dst_id: DestinationId) -> bool {
        if let Some(src) = self.source(src_id) {
            src.is_connected_to(dst_id)
        } else {
            false
        }
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
    pub fn wait(self) -> std::result::Result<(), Errors> {
        let errors = self
            .readers
            .into_iter()
            .filter_map(|(id, reader)| {
                match reader
                    .join()
                    .unwrap_or_else(|_| Err(Error::from("Source reader panicked")))
                {
                    Ok(_) => None,
                    Err(e) => Some((id, e)),
                }
            })
            .collect::<HashMap<_, _>>();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(Errors { errors })
        }
    }
}

fn read_source(src: Source) -> Result<ReadPipe> {
    let reader = src.reader;
    let mut pipe = src.pipe;
    let mut connections = src.connections;

    if let Some(mut reader) = reader {
        return reader.read(&mut pipe, &mut connections).map(|_| pipe);
    }

    let mut reader = BufReader::new(pipe);
    loop {
        let data_len = {
            let data = reader.fill_buf().unwrap_or(&[]);
            if data.is_empty() {
                break;
            }
            for c in connections.iter_mut() {
                c.send(data);
            }
            data.len()
        };
        reader.consume(data_len);

        if connections.iter().all(Connection::is_dead) {
            break;
        }
    }

    Ok(reader.into_inner())
}
