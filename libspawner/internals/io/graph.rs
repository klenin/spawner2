use internals::io::mpsc::{Receiver, Sender};
use pipe::{self, ReadPipe, WritePipe};
use std::io;
use std::mem;

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
    PipeReceiver(ReadPipe, Receiver<WritePipe>),
    File(WritePipe),
    FileReceiver(Receiver<WritePipe>),
    Inlined(usize),
}

pub struct Istream {
    pub incoming_ostreams: usize,
    pub kind: IstreamKind,
    is_receiver: bool,
}

pub enum OstreamKind {
    Unknown,
    Pipe(WritePipe),
    PipeSender(WritePipe, Sender<ReadPipe>),
    File(ReadPipe),
    FileSender(Sender<ReadPipe>),
    Inlined(usize),
}

pub struct Ostream {
    pub istreams: Vec<usize>,
    pub kind: OstreamKind,
    is_sender: bool,
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
            is_sender: false,
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
            is_receiver: false,
        });
        istream_no
    }

    pub fn add_unknown_istream(&mut self) -> usize {
        self.add_istream(IstreamKind::Unknown)
    }

    pub fn add_file_istream(&mut self, file: WritePipe) -> usize {
        self.add_istream(IstreamKind::File(file))
    }

    pub fn connect(&mut self, istream_no: usize, ostream_no: usize) -> io::Result<()> {
        if istream_no > self.graph.istreams.len() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("istream index '{}' is out of range", istream_no),
            ));
        }
        if ostream_no > self.graph.ostreams.len() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("ostream index '{}' is out of range", ostream_no),
            ));
        }

        let istream = &mut self.graph.istreams[istream_no];
        let ostream = &mut self.graph.ostreams[ostream_no];
        if !ostream.istreams.iter().any(|x| *x == istream_no) {
            istream.incoming_ostreams += 1;
            ostream.istreams.push(istream_no);
        }

        Ok(())
    }

    pub fn build(mut self) -> io::Result<Graph> {
        self.detect_senders_receivers();
        self.upgrade_iostreams()?;
        self.connect_iostreams()?;
        Ok(self.graph)
    }

    fn detect_senders_receivers(&mut self) {
        let istreams = &mut self.graph.istreams;
        let ostreams = &mut self.graph.ostreams;

        for istream in istreams.iter_mut() {
            istream.kind.ensure_file_or_unknown();
            istream.is_receiver = istream.incoming_ostreams > 1;
        }

        for ostream in ostreams {
            ostream.kind.ensure_file_or_unknown();

            let is_sender = ostream.istreams.len() > 1
                || ostream
                    .istreams
                    .iter()
                    .any(|istream| istreams[*istream].is_receiver);

            if is_sender {
                for istream in ostream.istreams.iter() {
                    istreams[*istream].is_receiver = true;
                }
            }
            ostream.is_sender = is_sender;
        }
    }

    fn upgrade_iostreams(&mut self) -> io::Result<()> {
        for istream in self.graph.istreams.iter_mut() {
            if istream.is_receiver {
                istream.kind.upgrade_to_receiver()?;
            }
        }
        for ostream in self.graph.ostreams.iter_mut() {
            if ostream.is_sender {
                ostream.kind.upgrade_to_sender()?;
            }
        }
        Ok(())
    }

    fn connect_iostreams(&mut self) -> io::Result<()> {
        for (index, ostream) in self.graph.ostreams.iter_mut().enumerate() {
            let istreams_len = ostream.istreams.len();
            if ostream.kind.is_unknown() && istreams_len == 0 {
                // nobody reads from this ostream, so initialize it with null
                ostream.kind = OstreamKind::Pipe(WritePipe::null()?);
            } else if ostream.kind.is_unknown() && istreams_len == 1 {
                // there are one reader, if the reader is file then inline it into ostream
                // otherwise connect ostream and reader through pipe
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
                // there are one reader, if it reads file then inline file into it
                let istream_no = ostream.istreams[0];
                let istream = &mut self.graph.istreams[istream_no];
                istream.take_file_from(ostream);
            } else if let Some(sender) = ostream.kind.sender_mut() {
                for istream in ostream.istreams.iter() {
                    self.graph.istreams[*istream]
                        .kind
                        .receiver()
                        .unwrap()
                        .receive_from(sender);
                }
            }
        }

        for istream in self
            .graph
            .istreams
            .iter_mut()
            .filter(|x| x.kind.is_unknown())
        {
            // at this point only unused istreams are left
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

    fn receiver(&self) -> Option<&Receiver<WritePipe>> {
        match self {
            IstreamKind::PipeReceiver(_, ref r) => Some(r),
            IstreamKind::FileReceiver(ref r) => Some(r),
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

    fn upgrade_to_receiver(&mut self) -> io::Result<()> {
        self.ensure_file_or_unknown();

        let placeholder = IstreamKind::Unknown;
        *self = match mem::replace(self, placeholder) {
            IstreamKind::File(f) => IstreamKind::FileReceiver(Receiver::new(f)),
            IstreamKind::Unknown => {
                let (r, w) = pipe::create()?;
                IstreamKind::PipeReceiver(r, Receiver::new(w))
            }

            _ => unreachable!(),
        };

        Ok(())
    }

    pub fn take_pipe(&mut self) -> (ReadPipe, Option<Receiver<WritePipe>>) {
        match mem::replace(self, IstreamKind::Unknown) {
            IstreamKind::Pipe(p) => (p, None),
            IstreamKind::PipeReceiver(p, r) => (p, Some(r)),
            _ => unreachable!(),
        }
    }

    pub fn is_file_receiver(&self) -> bool {
        match self {
            IstreamKind::FileReceiver(_) => true,
            _ => false,
        }
    }

    pub fn take_file_receiver(&mut self) -> Receiver<WritePipe> {
        match mem::replace(self, IstreamKind::Unknown) {
            IstreamKind::FileReceiver(r) => r,
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

    fn sender_mut(&mut self) -> Option<&mut Sender<ReadPipe>> {
        match self {
            OstreamKind::PipeSender(_, ref mut s) => Some(s),
            OstreamKind::FileSender(ref mut s) => Some(s),
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

    fn upgrade_to_sender(&mut self) -> io::Result<()> {
        self.ensure_file_or_unknown();

        let placeholder = OstreamKind::Unknown;
        *self = match mem::replace(self, placeholder) {
            OstreamKind::File(f) => OstreamKind::FileSender(Sender::new(f)),
            OstreamKind::Unknown => {
                let (r, w) = pipe::create()?;
                OstreamKind::PipeSender(w, Sender::new(r))
            }

            _ => unreachable!(),
        };

        Ok(())
    }

    pub fn take_pipe(&mut self) -> (WritePipe, Option<Sender<ReadPipe>>) {
        match mem::replace(self, OstreamKind::Unknown) {
            OstreamKind::Pipe(p) => (p, None),
            OstreamKind::PipeSender(p, s) => (p, Some(s)),
            _ => unreachable!(),
        }
    }

    pub fn is_file_sender(&self) -> bool {
        match self {
            OstreamKind::FileSender(_) => true,
            _ => false,
        }
    }

    pub fn take_file_sender(&mut self) -> Sender<ReadPipe> {
        match mem::replace(self, OstreamKind::Unknown) {
            OstreamKind::FileSender(s) => s,
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
