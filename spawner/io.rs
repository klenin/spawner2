use crate::pipe::{self, ReadPipe, WritePipe};
use crate::rwhub::{ReadHub, WriteHub};
use crate::Result;

use std::collections::HashMap;

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct IstreamId(usize);

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct OstreamId(usize);

pub struct Istream {
    pub src: Option<WritePipe>,
    pub dst: ReadHub,
}

pub struct Ostream {
    pub src: WriteHub,
    pub dst: Option<ReadPipe>,
}

#[derive(Copy, Clone)]
pub struct StdioMapping {
    pub stdin: OstreamId,
    pub stdout: IstreamId,
    pub stderr: IstreamId,
}

pub struct Stdio {
    pub stdin: Ostream,
    pub stdout: Istream,
    pub stderr: Istream,
}

pub struct IoStreams {
    istreams: HashMap<IstreamId, Istream>,
    ostreams: HashMap<OstreamId, Ostream>,
    graph: IoGraph,
}

#[derive(Clone)]
pub struct IoGraph {
    istream_edges: Vec<Vec<OstreamId>>,
    ostream_edges: Vec<Vec<IstreamId>>,
}

pub enum IstreamDst {
    Pipe(WritePipe),
    File(WritePipe),
    Ostream(OstreamId),
}

pub enum OstreamSrc {
    Pipe(ReadPipe),
    File(ReadPipe),
    Istream(IstreamId),
}

pub struct IoBuilder(IoStreams);

impl IoStreams {
    pub fn take_istream(&mut self, id: IstreamId) -> Istream {
        self.istreams.remove(&id).unwrap()
    }

    pub fn take_ostream(&mut self, id: OstreamId) -> Ostream {
        self.ostreams.remove(&id).unwrap()
    }

    pub fn take_stdio(&mut self, stdio: StdioMapping) -> Stdio {
        Stdio {
            stdin: self.take_ostream(stdio.stdin),
            stdout: self.take_istream(stdio.stdout),
            stderr: self.take_istream(stdio.stderr),
        }
    }

    pub fn take_remaining_istreams<'a>(
        &'a mut self,
    ) -> impl Iterator<Item = (IstreamId, Istream)> + 'a {
        self.istreams.drain()
    }

    pub fn take_remaining_ostreams<'a>(
        &'a mut self,
    ) -> impl Iterator<Item = (OstreamId, Ostream)> + 'a {
        self.ostreams.drain()
    }

    pub fn graph(&self) -> &IoGraph {
        &self.graph
    }
}

impl IoGraph {
    pub fn istream_edges(&self, id: IstreamId) -> &Vec<OstreamId> {
        &self.istream_edges[id.0]
    }

    pub fn ostream_edges(&self, id: OstreamId) -> &Vec<IstreamId> {
        &self.ostream_edges[id.0]
    }
}

impl IoBuilder {
    pub fn new() -> Self {
        Self(IoStreams {
            istreams: HashMap::new(),
            ostreams: HashMap::new(),
            graph: IoGraph {
                istream_edges: Vec::new(),
                ostream_edges: Vec::new(),
            },
        })
    }

    pub fn add_stdio(&mut self) -> Result<StdioMapping> {
        Ok(StdioMapping {
            stdin: self.add_ostream(None)?,
            stdout: self.add_istream(None)?,
            stderr: self.add_istream(None)?,
        })
    }

    pub fn add_istream(&mut self, pipe: Option<ReadPipe>) -> Result<IstreamId> {
        let id = IstreamId(self.0.istreams.len());
        self.0.istreams.insert(
            id,
            match pipe {
                Some(p) => Istream {
                    src: None,
                    dst: ReadHub::new(p),
                },
                None => {
                    let (r, w) = pipe::create()?;
                    Istream {
                        src: Some(w),
                        dst: ReadHub::new(r),
                    }
                }
            },
        );
        self.0.graph.istream_edges.push(Vec::new());
        Ok(id)
    }

    pub fn add_file_istream(&mut self, file: ReadPipe) -> Result<IstreamId> {
        self.add_istream(Some(file))
    }

    pub fn add_istream_dst<D: Into<IstreamDst>>(
        &mut self,
        istream: IstreamId,
        dst: D,
    ) -> Result<()> {
        let ostream = match dst.into() {
            IstreamDst::Pipe(p) => self.add_ostream(Some(p))?,
            IstreamDst::File(f) => self.add_file_ostream(f)?,
            IstreamDst::Ostream(i) => i,
        };

        self.connect(istream, ostream);
        Ok(())
    }

    pub fn add_ostream(&mut self, pipe: Option<WritePipe>) -> Result<OstreamId> {
        let id = OstreamId(self.0.ostreams.len());
        self.0.ostreams.insert(
            id,
            match pipe {
                Some(p) => Ostream {
                    src: WriteHub::from_pipe(p),
                    dst: None,
                },
                None => {
                    let (r, w) = pipe::create()?;
                    Ostream {
                        src: WriteHub::from_pipe(w),
                        dst: Some(r),
                    }
                }
            },
        );
        self.0.graph.ostream_edges.push(Vec::new());
        Ok(id)
    }

    pub fn add_file_ostream(&mut self, file: WritePipe) -> Result<OstreamId> {
        let id = OstreamId(self.0.ostreams.len());
        self.0.ostreams.insert(
            id,
            Ostream {
                src: WriteHub::from_file(file),
                dst: None,
            },
        );
        self.0.graph.ostream_edges.push(Vec::new());
        Ok(id)
    }

    pub fn add_ostream_src<S: Into<OstreamSrc>>(
        &mut self,
        ostream: OstreamId,
        src: S,
    ) -> Result<()> {
        let istream = match src.into() {
            OstreamSrc::Pipe(p) => self.add_istream(Some(p))?,
            OstreamSrc::File(f) => self.add_file_istream(f)?,
            OstreamSrc::Istream(i) => i,
        };
        self.connect(istream, ostream);
        Ok(())
    }

    pub fn connect(&mut self, istream_id: IstreamId, ostream_id: OstreamId) {
        let istream = self.0.istreams.get_mut(&istream_id).unwrap();
        let ostream = self.0.ostreams.get_mut(&ostream_id).unwrap();
        if self
            .0
            .graph
            .istream_edges(istream_id)
            .iter()
            .any(|&id| id == ostream_id)
        {
            return;
        }

        istream.dst.connect(&ostream.src);
        self.0.graph.istream_edges[istream_id.0].push(ostream_id);
        self.0.graph.ostream_edges[ostream_id.0].push(istream_id);
    }

    pub fn build(self) -> IoStreams {
        self.0
    }
}
