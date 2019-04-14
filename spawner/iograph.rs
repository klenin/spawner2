use crate::pipe::{self, ReadPipe, WritePipe};
use crate::rwhub::{ReadHub, ReadHubController, WriteHub};
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

pub struct IoStreams {
    pub istreams: HashMap<IstreamId, Istream>,
    pub ostreams: HashMap<OstreamId, Ostream>,
}

#[derive(Clone)]
pub struct IoGraph {
    istream_edges: Vec<Vec<OstreamId>>,
    ostream_edges: Vec<Vec<IstreamId>>,
}

pub struct IoBuilder {
    iostreams: IoStreams,
    iograph: IoGraph,
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
        Self {
            iostreams: IoStreams {
                istreams: HashMap::new(),
                ostreams: HashMap::new(),
            },
            iograph: IoGraph {
                istream_edges: Vec::new(),
                ostream_edges: Vec::new(),
            },
        }
    }

    pub fn add_istream(
        &mut self,
        pipe: Option<ReadPipe>,
        controller: Option<Box<ReadHubController>>,
    ) -> Result<IstreamId> {
        let id = IstreamId(self.iostreams.istreams.len());
        self.iostreams.istreams.insert(
            id,
            match pipe {
                Some(p) => Istream {
                    src: None,
                    dst: ReadHub::new(p, controller),
                },
                None => {
                    let (r, w) = pipe::create()?;
                    Istream {
                        src: Some(w),
                        dst: ReadHub::new(r, controller),
                    }
                }
            },
        );
        self.iograph.istream_edges.push(Vec::new());
        Ok(id)
    }

    pub fn add_ostream(&mut self, pipe: Option<WritePipe>) -> Result<OstreamId> {
        let id = OstreamId(self.iostreams.ostreams.len());
        self.iostreams.ostreams.insert(
            id,
            match pipe {
                Some(p) => Ostream {
                    src: WriteHub::new(p),
                    dst: None,
                },
                None => {
                    let (r, w) = pipe::create()?;
                    Ostream {
                        src: WriteHub::new(w),
                        dst: Some(r),
                    }
                }
            },
        );
        self.iograph.ostream_edges.push(Vec::new());
        Ok(id)
    }

    pub fn connect(&mut self, istream_id: IstreamId, ostream_id: OstreamId) {
        let istream = self.iostreams.istreams.get_mut(&istream_id).unwrap();
        let ostream = self.iostreams.ostreams.get_mut(&ostream_id).unwrap();
        if self
            .iograph
            .istream_edges(istream_id)
            .iter()
            .any(|&id| id == ostream_id)
        {
            return;
        }

        istream.dst.connect(&ostream.src);
        self.iograph.istream_edges[istream_id.0].push(ostream_id);
        self.iograph.ostream_edges[ostream_id.0].push(istream_id);
    }

    pub fn build(self) -> (IoStreams, IoGraph) {
        (self.iostreams, self.iograph)
    }
}
