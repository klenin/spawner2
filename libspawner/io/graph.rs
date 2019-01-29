use crate::{Error, Result};
use io::split_combine::{Combiner, Splitter};
use pipe::{self, ReadPipe, WritePipe};
use std::mem;

/// This structure handles initialization and connection of given i\o streams with as little
/// splitters\combiners as possible.
///
/// In some cases (e.g: one-to-one) it is possible to create
/// a single (ReadPipe, WritePipe) pair with zero overhead. If the ostream is file (ReadPipe)
/// that goes to single istream, then it is possible to inline file into that istream.
///
/// If the relation between i\o streams is one-to-many or many-to-one then we need to initialize
/// corresponding pipe splitters and combiners.
pub struct Builder {
    graph: Graph,
}

pub struct Graph {
    pub istreams: Vec<Istream>,
    pub ostreams: Vec<Ostream>,
}

pub enum IstreamKind {
    Unknown,
    Pipe(ReadPipe),
    PipeCombiner(ReadPipe, Combiner),
    File(WritePipe),
    FileCombiner(Combiner),
    Inlined(usize),
}

pub struct Istream {
    pub incoming_ostreams: usize,
    pub kind: IstreamKind,
    is_combiner_required: bool,
}

pub enum OstreamKind {
    Unknown,
    Pipe(WritePipe),
    PipeSplitter(WritePipe, Splitter),
    File(ReadPipe),
    FileSplitter(Splitter),
    Inlined(usize),
}

pub struct Ostream {
    pub istreams: Vec<usize>,
    pub kind: OstreamKind,
    is_splitter_required: bool,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            graph: Graph {
                istreams: Vec::new(),
                ostreams: Vec::new(),
            },
        }
    }

    fn add_ostream(&mut self, kind: OstreamKind) -> usize {
        kind.ensure_file_or_unknown();
        let ostream_no = self.graph.ostreams.len();
        self.graph.ostreams.push(Ostream {
            kind: kind,
            istreams: Vec::new(),
            is_splitter_required: false,
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
        let istream_no = self.graph.istreams.len();
        self.graph.istreams.push(Istream {
            kind: kind,
            incoming_ostreams: 0,
            is_combiner_required: false,
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
        if istream_no > self.graph.istreams.len() {
            return Err(Error::from(format!(
                "istream index '{}' is out of range",
                istream_no
            )));
        }
        if ostream_no > self.graph.ostreams.len() {
            return Err(Error::from(format!(
                "ostream index '{}' is out of range",
                ostream_no
            )));
        }

        let istream = &mut self.graph.istreams[istream_no];
        let ostream = &mut self.graph.ostreams[ostream_no];
        if !ostream.istreams.iter().any(|x| *x == istream_no) {
            istream.incoming_ostreams += 1;
            ostream.istreams.push(istream_no);
        }

        Ok(())
    }

    pub fn build(mut self) -> Result<Graph> {
        self.detect_splitters_combiners();
        self.upgrade_iostreams()?;
        self.connect_iostreams()?;
        Ok(self.graph)
    }

    fn detect_splitters_combiners(&mut self) {
        let istreams = &mut self.graph.istreams;
        let ostreams = &mut self.graph.ostreams;

        for istream in istreams.iter_mut() {
            istream.kind.ensure_file_or_unknown();
            istream.is_combiner_required = istream.incoming_ostreams > 1;
        }

        for ostream in ostreams {
            ostream.kind.ensure_file_or_unknown();

            let istreams_len = ostream.istreams.len();
            let is_splitter_required = istreams_len > 1
                || (istreams_len == 1 && istreams[ostream.istreams[0]].is_combiner_required);

            if is_splitter_required {
                for istream in ostream.istreams.iter() {
                    istreams[*istream].is_combiner_required = true;
                }
            }
            ostream.is_splitter_required = is_splitter_required;
        }
    }

    fn upgrade_iostreams(&mut self) -> Result<()> {
        for istream in self.graph.istreams.iter_mut() {
            if istream.is_combiner_required {
                istream.kind.upgrade_to_combiner()?;
            }
        }
        for ostream in self.graph.ostreams.iter_mut() {
            if ostream.is_splitter_required {
                ostream.kind.upgrade_to_splitter()?;
            }
        }
        Ok(())
    }

    fn connect_iostreams(&mut self) -> Result<()> {
        for (index, ostream) in self.graph.ostreams.iter_mut().enumerate() {
            let istreams_len = ostream.istreams.len();
            if ostream.kind.is_unknown() && istreams_len == 0 {
                // Nobody reads from this ostream, so initialize it with null.
                ostream.kind = OstreamKind::Pipe(WritePipe::null()?);
            } else if ostream.kind.is_unknown() && istreams_len == 1 {
                // There are one reader, if the reader is file then inline it into ostream,
                // otherwise connect ostream and reader through pipe.
                let istream = &mut self.graph.istreams[ostream.istreams[0]];
                istream.kind.ensure_file_or_unknown();
                if istream.kind.is_unknown() {
                    let (r, w) = pipe::create()?;
                    istream.kind = IstreamKind::Pipe(r);
                    ostream.kind = OstreamKind::Pipe(w);
                } else {
                    ostream.take_file_from(index, istream);
                }
            } else if ostream.kind.is_file() && istreams_len == 1 {
                // There are one reader, if it reads file then inline file into it.
                let istream_no = ostream.istreams[0];
                let istream = &mut self.graph.istreams[istream_no];
                istream.take_file_from(ostream);
            } else if let Some(splitter) = ostream.kind.splitter_opt() {
                for istream in ostream.istreams.iter() {
                    self.graph.istreams[*istream]
                        .kind
                        .combiner_opt()
                        .unwrap()
                        .connect(splitter);
                }
            }
        }

        for istream in self
            .graph
            .istreams
            .iter_mut()
            .filter(|x| x.kind.is_unknown())
        {
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

    fn combiner_opt(&self) -> Option<&Combiner> {
        match self {
            IstreamKind::PipeCombiner(_, ref c) => Some(c),
            IstreamKind::FileCombiner(ref c) => Some(c),
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

    fn upgrade_to_combiner(&mut self) -> Result<()> {
        self.ensure_file_or_unknown();

        let placeholder = IstreamKind::Unknown;
        *self = match mem::replace(self, placeholder) {
            IstreamKind::File(f) => IstreamKind::FileCombiner(Combiner::new(f)),
            IstreamKind::Unknown => {
                let (r, w) = pipe::create()?;
                IstreamKind::PipeCombiner(r, Combiner::new(w))
            }
            _ => unreachable!(),
        };

        Ok(())
    }

    pub fn take_pipe(&mut self) -> (ReadPipe, Option<Combiner>) {
        match mem::replace(self, IstreamKind::Unknown) {
            IstreamKind::Pipe(p) => (p, None),
            IstreamKind::PipeCombiner(p, c) => (p, Some(c)),
            _ => unreachable!(),
        }
    }

    pub fn is_file_combiner(&self) -> bool {
        match self {
            IstreamKind::FileCombiner(_) => true,
            _ => false,
        }
    }

    pub fn take_file_combiner(&mut self) -> Combiner {
        match mem::replace(self, IstreamKind::Unknown) {
            IstreamKind::FileCombiner(r) => r,
            _ => unreachable!(),
        }
    }
}

impl Istream {
    fn take_file_from(&mut self, ostream: &mut Ostream) {
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

    fn splitter_opt(&mut self) -> Option<&mut Splitter> {
        match self {
            OstreamKind::PipeSplitter(_, ref mut s) => Some(s),
            OstreamKind::FileSplitter(ref mut s) => Some(s),
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

    fn upgrade_to_splitter(&mut self) -> Result<()> {
        self.ensure_file_or_unknown();

        let placeholder = OstreamKind::Unknown;
        *self = match mem::replace(self, placeholder) {
            OstreamKind::File(f) => OstreamKind::FileSplitter(Splitter::new(f)),
            OstreamKind::Unknown => {
                let (r, w) = pipe::create()?;
                OstreamKind::PipeSplitter(w, Splitter::new(r))
            }

            _ => unreachable!(),
        };

        Ok(())
    }

    pub fn take_pipe(&mut self) -> (WritePipe, Option<Splitter>) {
        match mem::replace(self, OstreamKind::Unknown) {
            OstreamKind::Pipe(p) => (p, None),
            OstreamKind::PipeSplitter(p, s) => (p, Some(s)),
            _ => unreachable!(),
        }
    }

    pub fn is_file_splitter(&self) -> bool {
        match self {
            OstreamKind::FileSplitter(_) => true,
            _ => false,
        }
    }

    pub fn take_file_splitter(&mut self) -> Splitter {
        match mem::replace(self, OstreamKind::Unknown) {
            OstreamKind::FileSplitter(s) => s,
            _ => unreachable!(),
        }
    }
}

impl Ostream {
    fn take_file_from(&mut self, self_no: usize, istream: &mut Istream) {
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
