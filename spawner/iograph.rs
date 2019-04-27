use crate::pipe::{self, FileLock, ReadPipe, WritePipe};
use crate::rwhub::{ReadHub, ReadHubController, WriteHub};
use crate::Result;

use std::collections::HashMap;
use std::path::PathBuf;

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

pub enum IstreamDst {
    Pipe(WritePipe),
    File(PathBuf, FileLock),
    Ostream(OstreamId),
}

pub enum OstreamSrc {
    Pipe(ReadPipe),
    File(PathBuf, FileLock),
    Istream(IstreamId),
}

pub struct IoBuilder {
    iostreams: IoStreams,
    iograph: IoGraph,
    output_files: HashMap<PathBuf, OstreamId>,
}

impl IoGraph {
    pub fn istream_edges(&self, id: IstreamId) -> &Vec<OstreamId> {
        &self.istream_edges[id.0]
    }

    pub fn ostream_edges(&self, id: OstreamId) -> &Vec<IstreamId> {
        &self.ostream_edges[id.0]
    }
}

impl From<WritePipe> for IstreamDst {
    fn from(p: WritePipe) -> IstreamDst {
        IstreamDst::Pipe(p)
    }
}

impl From<OstreamId> for IstreamDst {
    fn from(id: OstreamId) -> IstreamDst {
        IstreamDst::Ostream(id)
    }
}

impl From<ReadPipe> for OstreamSrc {
    fn from(p: ReadPipe) -> OstreamSrc {
        OstreamSrc::Pipe(p)
    }
}

impl From<IstreamId> for OstreamSrc {
    fn from(id: IstreamId) -> OstreamSrc {
        OstreamSrc::Istream(id)
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
            output_files: HashMap::new(),
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

    pub fn add_istream_dst<D: Into<IstreamDst>>(
        &mut self,
        istream: IstreamId,
        dst: D,
    ) -> Result<()> {
        let ostream = match dst.into() {
            IstreamDst::Pipe(p) => self.add_ostream(Some(p))?,
            IstreamDst::File(file, mode) => match self.output_files.get(&file).map(|&id| id) {
                Some(id) => id,
                None => {
                    let pipe = WritePipe::open(&file, mode)?;
                    let id = self.add_ostream(Some(pipe))?;
                    self.output_files.insert(file, id);
                    id
                }
            },
            IstreamDst::Ostream(i) => i,
        };

        self.connect(istream, ostream);
        Ok(())
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

    pub fn add_ostream_src<S: Into<OstreamSrc>>(
        &mut self,
        ostream: OstreamId,
        src: S,
    ) -> Result<()> {
        let istream = match src.into() {
            OstreamSrc::Pipe(p) => self.add_istream(Some(p), None)?,
            OstreamSrc::File(file, mode) => {
                self.add_istream(Some(ReadPipe::open(file, mode)?), None)?
            }
            OstreamSrc::Istream(i) => i,
        };
        self.connect(istream, ostream);
        Ok(())
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
