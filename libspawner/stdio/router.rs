use crate::{Error, Result};
use pipe::{self, ReadPipe, WritePipe};
use std::mem;
use stdio::hub::{self, ReadHub, WriteHub};

pub struct Router {
    read_hubs: Vec<ReadHub>,
    write_hubs: Vec<WriteHub>,
}

pub struct StopHandle {
    read_hubs: Vec<hub::StopHandle>,
    write_hubs: Vec<hub::StopHandle>,
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
pub struct Builder {
    istreams: Vec<IstreamInfo>,
    ostreams: Vec<OstreamInfo>,
}

enum IstreamKind {
    Unknown,
    Pipe(ReadPipe),
    PipeHub(ReadPipe, WriteHub),
    File(WritePipe),
    FileHub(WriteHub),
    Inlined(usize),
}

struct IstreamInfo {
    incoming_ostreams: usize,
    kind: IstreamKind,
    is_hub_required: bool,
}

pub enum OstreamKind {
    Unknown,
    Pipe(WritePipe),
    PipeHub(WritePipe, ReadHub),
    File(ReadPipe),
    FileHub(ReadHub),
    Inlined(usize),
}

struct OstreamInfo {
    istreams: Vec<usize>,
    kind: OstreamKind,
    is_hub_required: bool,
}

impl Router {
    pub fn start(mut self) -> Result<StopHandle> {
        let mut sh = StopHandle {
            read_hubs: Vec::new(),
            write_hubs: Vec::new(),
        };
        for hub in self.write_hubs.drain(..) {
            sh.write_hubs.push(hub.start()?);
        }
        for hub in self.read_hubs.drain(..) {
            sh.read_hubs.push(hub.start()?);
        }
        Ok(sh)
    }
}

impl StopHandle {
    pub fn stop(&mut self) -> Result<()> {
        for hub in self.read_hubs.drain(..) {
            hub.stop()?;
        }
        for hub in self.write_hubs.drain(..) {
            hub.stop()?;
        }
        Ok(())
    }
}

impl Drop for StopHandle {
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

impl Builder {
    pub fn new() -> Self {
        Self {
            istreams: Vec::new(),
            ostreams: Vec::new(),
        }
    }

    fn add_ostream(&mut self, kind: OstreamKind) -> usize {
        kind.ensure_file_or_unknown();
        let ostream_no = self.ostreams.len();
        self.ostreams.push(OstreamInfo {
            kind: kind,
            istreams: Vec::new(),
            is_hub_required: false,
        });
        ostream_no
    }

    pub fn add_unknown_ostream(&mut self) -> usize {
        self.add_ostream(OstreamKind::Unknown)
    }

    pub fn add_file_ostream(&mut self, file: ReadPipe) -> usize {
        self.add_ostream(OstreamKind::File(file))
    }

    fn add_istream(&mut self, kind: IstreamKind) -> usize {
        kind.ensure_file_or_unknown();
        let istream_no = self.istreams.len();
        self.istreams.push(IstreamInfo {
            kind: kind,
            incoming_ostreams: 0,
            is_hub_required: false,
        });
        istream_no
    }

    pub fn add_unknown_istream(&mut self) -> usize {
        self.add_istream(IstreamKind::Unknown)
    }

    pub fn add_file_istream(&mut self, file: WritePipe) -> usize {
        self.add_istream(IstreamKind::File(file))
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

    pub fn build(mut self) -> Result<(Router, IoList)> {
        self.detect_hubs();
        self.upgrade_iostreams()?;
        self.connect_iostreams()?;

        let mut router = Router {
            read_hubs: Vec::new(),
            write_hubs: Vec::new(),
        };
        let mut list = IoList {
            istreams: Vec::new(),
            ostreams: Vec::new(),
        };

        for istream in self.istreams.drain(..) {
            match istream.kind {
                IstreamKind::Unknown => unreachable!(),
                IstreamKind::Pipe(p) => {
                    list.istreams.push(Istream::Pipe(p));
                }
                IstreamKind::PipeHub(p, h) => {
                    router.write_hubs.push(h);
                    list.istreams.push(Istream::Pipe(p));
                }
                IstreamKind::File(_) | IstreamKind::Inlined(_) => {
                    list.istreams.push(Istream::Empty);
                }
                IstreamKind::FileHub(h) => {
                    router.write_hubs.push(h);
                }
            };
        }

        for ostream in self.ostreams.drain(..) {
            match ostream.kind {
                OstreamKind::Unknown => unreachable!(),
                OstreamKind::Pipe(p) => {
                    list.ostreams.push(Ostream::Pipe(p));
                }
                OstreamKind::PipeHub(p, h) => {
                    router.read_hubs.push(h);
                    list.ostreams.push(Ostream::Pipe(p));
                }
                OstreamKind::File(_) | OstreamKind::Inlined(_) => {
                    list.ostreams.push(Ostream::Empty);
                }
                OstreamKind::FileHub(h) => {
                    router.read_hubs.push(h);
                }
            };
        }

        Ok((router, list))
    }

    fn detect_hubs(&mut self) {
        let istreams = &mut self.istreams;
        let ostreams = &mut self.ostreams;

        for istream in istreams.iter_mut() {
            istream.kind.ensure_file_or_unknown();
            istream.is_hub_required = istream.incoming_ostreams > 1;
        }

        for ostream in ostreams {
            ostream.kind.ensure_file_or_unknown();

            let istreams_len = ostream.istreams.len();
            let is_hub_required = istreams_len > 1
                || (istreams_len == 1 && istreams[ostream.istreams[0]].is_hub_required);

            if is_hub_required {
                for istream in ostream.istreams.iter() {
                    istreams[*istream].is_hub_required = true;
                }
            }
            ostream.is_hub_required = is_hub_required;
        }
    }

    fn upgrade_iostreams(&mut self) -> Result<()> {
        for istream in self.istreams.iter_mut() {
            if istream.is_hub_required {
                istream.kind.upgrade_to_hub()?;
            }
        }
        for ostream in self.ostreams.iter_mut() {
            if ostream.is_hub_required {
                ostream.kind.upgrade_to_hub()?;
            }
        }
        Ok(())
    }

    fn connect_iostreams(&mut self) -> Result<()> {
        for (index, ostream) in self.ostreams.iter_mut().enumerate() {
            let istreams_len = ostream.istreams.len();
            if ostream.kind.is_unknown() && istreams_len == 0 {
                // Nobody reads from this ostream, so initialize it with null.
                ostream.kind = OstreamKind::Pipe(WritePipe::null()?);
            } else if ostream.kind.is_unknown() && istreams_len == 1 {
                // There is one istream, if it is file then inline it into ostream,
                // otherwise connect ostream and istream through pipe.
                let istream = &mut self.istreams[ostream.istreams[0]];
                istream.kind.ensure_file_or_unknown();
                if istream.kind.is_unknown() {
                    let (r, w) = pipe::create()?;
                    istream.kind = IstreamKind::Pipe(r);
                    ostream.kind = OstreamKind::Pipe(w);
                } else {
                    ostream.take_file_from(index, istream);
                }
            } else if ostream.kind.is_file() && istreams_len == 1 {
                // There is one istream, if it reads file then inline file into it.
                let istream_no = ostream.istreams[0];
                let istream = &mut self.istreams[istream_no];
                istream.take_file_from(ostream);
            } else if let Some(read_hub) = ostream.kind.hub_opt() {
                for istream in ostream.istreams.iter() {
                    self.istreams[*istream]
                        .kind
                        .hub_opt()
                        .unwrap()
                        .connect(read_hub);
                }
            }
        }

        for istream in self.istreams.iter_mut().filter(|x| x.kind.is_unknown()) {
            // At this point only unused istreams are left.
            assert!(istream.incoming_ostreams == 0);
            istream.kind = IstreamKind::Pipe(ReadPipe::null()?);
        }

        Ok(())
    }
}

impl IstreamKind {
    fn ensure_file_or_unknown(&self) {
        match self {
            IstreamKind::File(_) | IstreamKind::Unknown => {}
            _ => assert!(false, "invalid initial state"),
        }
    }

    fn hub_opt(&self) -> Option<&WriteHub> {
        match self {
            IstreamKind::PipeHub(_, ref h) => Some(h),
            IstreamKind::FileHub(ref h) => Some(h),
            _ => None,
        }
    }

    fn is_file(&self) -> bool {
        match self {
            IstreamKind::File(_) => true,
            _ => false,
        }
    }

    fn is_unknown(&self) -> bool {
        match self {
            IstreamKind::Unknown => true,
            _ => false,
        }
    }

    fn upgrade_to_hub(&mut self) -> Result<()> {
        self.ensure_file_or_unknown();

        let placeholder = IstreamKind::Unknown;
        *self = match mem::replace(self, placeholder) {
            IstreamKind::File(f) => IstreamKind::FileHub(WriteHub::new(f)),
            IstreamKind::Unknown => {
                let (r, w) = pipe::create()?;
                IstreamKind::PipeHub(r, WriteHub::new(w))
            }
            _ => unreachable!(),
        };

        Ok(())
    }
}

impl IstreamInfo {
    fn take_file_from(&mut self, ostream: &mut OstreamInfo) {
        assert!(self.kind.is_unknown() && self.incoming_ostreams == 1);
        assert!(ostream.kind.is_file() && ostream.istreams.len() == 1);

        let istream_no = ostream.istreams[0];
        let file = match mem::replace(&mut ostream.kind, OstreamKind::Inlined(istream_no)) {
            OstreamKind::File(f) => f,
            _ => unreachable!(),
        };

        self.kind = IstreamKind::Pipe(file);
        self.incoming_ostreams = 0;
        ostream.istreams.clear();
    }
}

impl OstreamKind {
    fn ensure_file_or_unknown(&self) {
        match self {
            OstreamKind::File(_) | OstreamKind::Unknown => {}
            _ => assert!(false, "invalid initial state"),
        }
    }

    fn hub_opt(&mut self) -> Option<&mut ReadHub> {
        match self {
            OstreamKind::PipeHub(_, ref mut h) => Some(h),
            OstreamKind::FileHub(ref mut h) => Some(h),
            _ => None,
        }
    }

    fn is_file(&self) -> bool {
        match self {
            OstreamKind::File(_) => true,
            _ => false,
        }
    }

    fn is_unknown(&self) -> bool {
        match self {
            OstreamKind::Unknown => true,
            _ => false,
        }
    }

    fn upgrade_to_hub(&mut self) -> Result<()> {
        self.ensure_file_or_unknown();

        let placeholder = OstreamKind::Unknown;
        *self = match mem::replace(self, placeholder) {
            OstreamKind::File(f) => OstreamKind::FileHub(ReadHub::new(f)),
            OstreamKind::Unknown => {
                let (r, w) = pipe::create()?;
                OstreamKind::PipeHub(w, ReadHub::new(r))
            }

            _ => unreachable!(),
        };

        Ok(())
    }
}

impl OstreamInfo {
    fn take_file_from(&mut self, self_no: usize, istream: &mut IstreamInfo) {
        assert!(self.kind.is_unknown() && self.istreams.len() == 1);
        assert!(istream.kind.is_file() && istream.incoming_ostreams == 1);

        let file = match mem::replace(&mut istream.kind, IstreamKind::Inlined(self_no)) {
            IstreamKind::File(f) => f,
            _ => unreachable!(),
        };

        self.kind = OstreamKind::Pipe(file);
        istream.incoming_ostreams = 0;
        self.istreams.clear();
    }
}
