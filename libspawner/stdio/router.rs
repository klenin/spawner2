use crate::{Error, Result};
use pipe::{self, ReadPipe, WritePipe};
use std::mem;
use std::thread::JoinHandle;
use stdio::hub::{ReadHub, WriteHub};

pub struct Router {
    read_hub_threads: Vec<JoinHandle<()>>,
    write_hub_threads: Vec<JoinHandle<()>>,
}

pub enum Istream {
    Pipe(ReadPipe),
    Empty,
}

pub enum Ostream {
    Pipe(WritePipe),
    Empty,
}

pub struct IoList {
    pub istreams: Vec<Istream>,
    pub ostreams: Vec<Ostream>,
}

/// This structure handles initialization and connection of given i\o streams with as little
/// read\write hubs as possible.
///
/// In some cases (e.g: one-to-one) it is possible to create a single (ReadPipe, WritePipe)
/// pair with zero overhead. If the ostream is file (ReadPipe) that goes to single istream,
/// then it is possible to inline file into that istream.
///
/// If the relation between i\o streams is one-to-many or many-to-one then we need to
/// initialize corresponding hubs.
pub struct RouterBuilder {
    istreams: Vec<IstreamInfo>,
    ostreams: Vec<OstreamInfo>,
}

struct IstreamInfo {
    incoming_ostreams: usize,
    src: Option<WritePipe>,
    kind: Option<IstreamKind>,
}

struct OstreamInfo {
    istreams: Vec<usize>,
    dst: Option<ReadPipe>,
    kind: Option<OstreamKind>,
}

enum IstreamKind {
    Pipe(ReadPipe),
    PipeHub(ReadPipe, WriteHub),
    File(WritePipe),
    FileHub(WriteHub),
    InlinedFile,
}

enum OstreamKind {
    Pipe(WritePipe),
    PipeHub(WritePipe, ReadHub),
    File(ReadPipe),
    FileHub(ReadHub),
    InlinedFile,
}

impl Router {
    pub fn stop(&mut self) -> Result<()> {
        for thread in self
            .read_hub_threads
            .drain(..)
            .chain(self.write_hub_threads.drain(..))
        {
            thread
                .join()
                .map_err(|_| Error::from("unexpected panic!(...) in thread"))?;
        }
        Ok(())
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl Istream {
    pub fn take(&mut self) -> ReadPipe {
        match mem::replace(self, Istream::Empty) {
            Istream::Pipe(p) => p,
            Istream::Empty => unreachable!(),
        }
    }
}

impl Ostream {
    pub fn take(&mut self) -> WritePipe {
        match mem::replace(self, Ostream::Empty) {
            Ostream::Pipe(p) => p,
            Ostream::Empty => unreachable!(),
        }
    }
}

impl RouterBuilder {
    pub fn new() -> Self {
        Self {
            istreams: Vec::new(),
            ostreams: Vec::new(),
        }
    }

    pub fn add_istream(&mut self, src: Option<WritePipe>) -> usize {
        let idx = self.istreams.len();
        self.istreams.push(IstreamInfo {
            incoming_ostreams: 0,
            src: src,
            kind: None,
        });
        idx
    }

    pub fn add_ostream(&mut self, dst: Option<ReadPipe>) -> usize {
        let idx = self.ostreams.len();
        self.ostreams.push(OstreamInfo {
            istreams: Vec::new(),
            dst: dst,
            kind: None,
        });
        idx
    }

    pub fn connect(&mut self, istream_no: usize, ostream_no: usize) -> Result<()> {
        if istream_no > self.istreams.len() {
            return Err(Error::from(format!(
                "istream index '{}' is out of range",
                istream_no
            )));
        }
        if ostream_no > self.ostreams.len() {
            return Err(Error::from(format!(
                "ostream index '{}' is out of range",
                ostream_no
            )));
        }

        let istream = &mut self.istreams[istream_no];
        let ostream = &mut self.ostreams[ostream_no];
        if !ostream.istreams.iter().any(|x| *x == istream_no) {
            istream.incoming_ostreams += 1;
            ostream.istreams.push(istream_no);
        }

        Ok(())
    }

    pub fn spawn(mut self) -> Result<(Router, IoList)> {
        self.init_hubs()?;
        self.connect_iostreams()?;

        let mut router = Router {
            read_hub_threads: Vec::new(),
            write_hub_threads: Vec::new(),
        };
        let mut iolist = IoList {
            istreams: Vec::new(),
            ostreams: Vec::new(),
        };

        for istream_kind in self.istreams.drain(..).map(|x| x.kind.unwrap()) {
            match istream_kind {
                IstreamKind::Pipe(rp) => {
                    iolist.istreams.push(Istream::Pipe(rp));
                }
                IstreamKind::PipeHub(rp, wh) => {
                    router.write_hub_threads.push(wh.spawn()?);
                    iolist.istreams.push(Istream::Pipe(rp));
                }
                IstreamKind::File(_) | IstreamKind::InlinedFile => {
                    iolist.istreams.push(Istream::Empty);
                }
                IstreamKind::FileHub(wh) => {
                    router.write_hub_threads.push(wh.spawn()?);
                }
            }
        }

        for ostream_kind in self.ostreams.drain(..).map(|x| x.kind.unwrap()) {
            match ostream_kind {
                OstreamKind::Pipe(wp) => {
                    iolist.ostreams.push(Ostream::Pipe(wp));
                }
                OstreamKind::PipeHub(wp, rh) => {
                    router.read_hub_threads.push(rh.spawn()?);
                    iolist.ostreams.push(Ostream::Pipe(wp));
                }
                OstreamKind::File(_) | OstreamKind::InlinedFile => {
                    iolist.ostreams.push(Ostream::Empty);
                }
                OstreamKind::FileHub(rh) => {
                    router.read_hub_threads.push(rh.spawn()?);
                }
            }
        }

        Ok((router, iolist))
    }

    fn init_hubs(&mut self) -> Result<()> {
        let istreams = &mut self.istreams;
        let ostreams = &mut self.ostreams;

        for istream in istreams.iter_mut() {
            if istream.incoming_ostreams > 1 {
                istream.into_hub()?;
            }
        }

        for ostream in ostreams {
            let istreams_len = ostream.istreams.len();
            let is_hub_required =
                istreams_len > 1 || (istreams_len == 1 && istreams[ostream.istreams[0]].is_hub());

            if is_hub_required {
                ostream.into_hub()?;
                for idx in ostream.istreams.iter() {
                    if !istreams[*idx].is_hub() {
                        istreams[*idx].into_hub()?;
                    }
                }
            }
        }

        Ok(())
    }

    fn connect_iostreams(&mut self) -> Result<()> {
        for ostream in self.ostreams.iter_mut() {
            let istreams_len = ostream.istreams.len();
            if ostream.is_hub() {
                // The ostream is hub, connect it to the other hubs.
                for istream in ostream.istreams.iter() {
                    self.istreams[*istream]
                        .kind
                        .as_ref()
                        .unwrap()
                        .hub()
                        .connect(ostream.kind.as_mut().unwrap().hub());
                }
            } else if ostream.dst.is_some() {
                if istreams_len == 1 {
                    // There is only one pipe reader. Inline pipe into it.
                    let istream = &mut self.istreams[ostream.istreams[0]];
                    istream.inline_ostream(ostream);
                } else {
                    // There is 0 file readers, just do initialization.
                    ostream.into_file();
                }
            } else if istreams_len == 1 {
                // There is one reader, if it has pipe then we are able to inline this pipe.
                // Otherwise connect streams through (ReadPipe, WritePipe) pair.
                let istream = &mut self.istreams[ostream.istreams[0]];
                if istream.src.is_some() {
                    ostream.inline_istream(istream);
                } else {
                    let (r, w) = pipe::create()?;
                    istream.into_pipe(r);
                    ostream.into_pipe(w);
                }
            } else {
                // So this stream doesn't have its own pipe and nobody reads from it.
                // Just initialize it with null.
                assert!(istreams_len == 0 && ostream.dst.is_none());
                ostream.into_null()?;
            }
        }

        for istream in self.istreams.iter_mut().filter(|x| x.kind.is_none()) {
            // At this point only unused istreams are left.
            assert!(istream.incoming_ostreams == 0);
            if istream.src.is_some() {
                istream.into_file();
            } else {
                istream.into_null()?;
            }
        }

        Ok(())
    }
}

impl IstreamInfo {
    fn into_hub(&mut self) -> Result<()> {
        assert!(self.kind.is_none());
        self.kind = if let Some(pipe) = self.src.take() {
            Some(IstreamKind::FileHub(WriteHub::new(pipe)))
        } else {
            let (r, w) = pipe::create()?;
            Some(IstreamKind::PipeHub(r, WriteHub::new(w)))
        };
        Ok(())
    }

    fn into_pipe(&mut self, pipe: ReadPipe) {
        assert!(self.kind.is_none() && self.src.is_none());
        self.kind = Some(IstreamKind::Pipe(pipe));
    }

    fn into_file(&mut self) {
        assert!(self.kind.is_none() && self.src.is_some());
        self.kind = Some(IstreamKind::File(self.src.take().unwrap()));
    }

    fn into_null(&mut self) -> Result<()> {
        assert!(self.kind.is_none() && self.src.is_none());
        self.kind = Some(IstreamKind::Pipe(ReadPipe::null()?));
        Ok(())
    }

    fn inline_ostream(&mut self, ostream: &mut OstreamInfo) {
        assert!(self.kind.is_none() && self.src.is_none() && self.incoming_ostreams == 1);
        assert!(ostream.kind.is_none() && ostream.dst.is_some() && ostream.istreams.len() == 1);

        self.into_pipe(ostream.dst.take().unwrap());
        self.incoming_ostreams = 0;

        ostream.kind = Some(OstreamKind::InlinedFile);
        ostream.istreams.clear();
    }

    fn is_hub(&self) -> bool {
        self.kind.is_some() && self.kind.as_ref().unwrap().is_hub()
    }
}

impl IstreamKind {
    fn is_hub(&self) -> bool {
        match self {
            IstreamKind::FileHub(_) | IstreamKind::PipeHub(..) => true,
            _ => false,
        }
    }

    fn hub(&self) -> &WriteHub {
        assert!(self.is_hub());
        match self {
            IstreamKind::FileHub(ref h) | IstreamKind::PipeHub(_, ref h) => h,
            _ => unreachable!(),
        }
    }
}

impl OstreamInfo {
    fn into_hub(&mut self) -> Result<()> {
        assert!(self.kind.is_none());
        self.kind = if let Some(pipe) = self.dst.take() {
            Some(OstreamKind::FileHub(ReadHub::new(pipe)))
        } else {
            let (r, w) = pipe::create()?;
            Some(OstreamKind::PipeHub(w, ReadHub::new(r)))
        };
        Ok(())
    }

    fn into_pipe(&mut self, pipe: WritePipe) {
        assert!(self.kind.is_none() && self.dst.is_none());
        self.kind = Some(OstreamKind::Pipe(pipe));
    }

    fn into_file(&mut self) {
        assert!(self.kind.is_none() && self.dst.is_some());
        self.kind = Some(OstreamKind::File(self.dst.take().unwrap()));
    }

    fn into_null(&mut self) -> Result<()> {
        assert!(self.kind.is_none() && self.dst.is_none());
        self.kind = Some(OstreamKind::Pipe(WritePipe::null()?));
        Ok(())
    }

    fn inline_istream(&mut self, istream: &mut IstreamInfo) {
        assert!(self.kind.is_none() && self.dst.is_none() && self.istreams.len() == 1);
        assert!(istream.kind.is_none() && istream.src.is_some() && istream.incoming_ostreams == 1);

        self.into_pipe(istream.src.take().unwrap());
        self.istreams.clear();

        istream.kind = Some(IstreamKind::InlinedFile);
        istream.incoming_ostreams = 0;
    }

    fn is_hub(&self) -> bool {
        self.kind.is_some() && self.kind.as_ref().unwrap().is_hub()
    }
}

impl OstreamKind {
    fn is_hub(&self) -> bool {
        match self {
            OstreamKind::FileHub(_) | OstreamKind::PipeHub(..) => true,
            _ => false,
        }
    }

    fn hub(&mut self) -> &mut ReadHub {
        assert!(self.is_hub());
        match self {
            OstreamKind::FileHub(ref mut h) | OstreamKind::PipeHub(_, ref mut h) => h,
            _ => unreachable!(),
        }
    }
}
