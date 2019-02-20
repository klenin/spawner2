use crate::{Error, Result};
use pipe::{self, ReadPipe, WritePipe};
use std::thread::JoinHandle;
use stdio::hub::{ReadHub, WriteHub};

#[derive(Copy, Clone)]
pub struct IstreamIdx(pub usize);
#[derive(Copy, Clone)]
pub struct OstreamIdx(pub usize);

pub struct Router {
    read_hub_threads: Vec<JoinHandle<()>>,
}

pub struct Istream {
    pipe: Option<WritePipe>,
    hub: Option<ReadHub>,
    listeners: Vec<OstreamIdx>,
}

pub struct Ostream {
    pipe: Option<ReadPipe>,
    hub: Option<WriteHub>,
}

pub struct RouterBuilder {
    pub istreams: Vec<Istream>,
    pub ostreams: Vec<Ostream>,
}

pub struct IoList {
    pub istream_srcs: Vec<Option<WritePipe>>,
    pub ostream_dsts: Vec<Option<ReadPipe>>,
}

impl Router {
    pub fn stop(&mut self) -> Result<()> {
        for thread in self.read_hub_threads.drain(..) {
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

impl RouterBuilder {
    pub fn new() -> Self {
        Self {
            istreams: Vec::new(),
            ostreams: Vec::new(),
        }
    }

    pub fn add_istream(&mut self, stream: Option<ReadPipe>) -> IstreamIdx {
        let idx = IstreamIdx(self.istreams.len());
        self.istreams.push(Istream {
            pipe: None,
            hub: stream.map(|s| ReadHub::new(s)),
            listeners: Vec::new(),
        });
        idx
    }

    pub fn add_ostream(&mut self, stream: Option<WritePipe>) -> OstreamIdx {
        let idx = OstreamIdx(self.ostreams.len());
        self.ostreams.push(Ostream {
            pipe: None,
            hub: stream.map(|s| WriteHub::new(s)),
        });
        idx
    }

    pub fn connect(&mut self, istream_idx: IstreamIdx, ostream_idx: OstreamIdx) -> Result<()> {
        let istream = &mut self.istreams[istream_idx.0];
        let ostream = &mut self.ostreams[ostream_idx.0];
        if istream.listeners.iter().any(|x| x.0 == ostream_idx.0) {
            return Ok(());
        }

        istream.listeners.push(ostream_idx);
        if istream.hub.is_none() {
            assert!(istream.pipe.is_none());
            let (r, w) = pipe::create()?;
            istream.pipe = Some(w);
            istream.hub = Some(ReadHub::new(r));
        }
        if ostream.hub.is_none() {
            assert!(ostream.pipe.is_none());
            let (r, w) = pipe::create()?;
            ostream.pipe = Some(r);
            ostream.hub = Some(WriteHub::new(w));
        }

        istream
            .hub
            .as_mut()
            .unwrap()
            .connect(ostream.hub.as_ref().unwrap());
        Ok(())
    }

    pub fn spawn(self) -> Result<(IoList, Router)> {
        let mut router = Router {
            read_hub_threads: Vec::new(),
        };
        let mut list = IoList {
            istream_srcs: Vec::new(),
            ostream_dsts: Vec::new(),
        };

        for istream in self.istreams.into_iter() {
            let is_predefined = istream.pipe.is_none() && istream.hub.is_some();
            list.istream_srcs.push(match is_predefined {
                true => None,
                false => Some(istream.pipe.unwrap_or(WritePipe::null()?)),
            });
            if let Some(hub) = istream.hub {
                router.read_hub_threads.push(hub.spawn()?);
            }
        }

        for ostream in self.ostreams.into_iter() {
            let is_predefined = ostream.pipe.is_none() && ostream.hub.is_some();
            list.ostream_dsts.push(match is_predefined {
                true => None,
                false => Some(ostream.pipe.unwrap_or(ReadPipe::null()?)),
            });
        }

        Ok((list, router))
    }
}
