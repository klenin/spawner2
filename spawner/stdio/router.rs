use crate::pipe::{self, ReadPipe, WritePipe};
use crate::stdio::hub::{ReadHub, ReadHubResult, WriteHub};
use crate::stdio::{IstreamController, IstreamIdx, OstreamIdx};
use crate::{Error, Result};

use std::collections::HashMap;
use std::thread::JoinHandle;

pub struct Router {
    readhub_threads: Vec<(IstreamIdx, JoinHandle<ReadHubResult>)>,
    // Some of these files are exclusively opened, so they are stored here
    // to keep them exclusive as long as possible.
    output_files: Vec<WriteHub>,
}

pub struct StopErrors {
    pub istream_errors: HashMap<IstreamIdx, Error>,
}

struct IstreamInfo {
    src: Option<WritePipe>,
    controller: Option<Box<IstreamController>>,
    hub: Option<ReadHub>,
    listeners: Vec<OstreamIdx>,
}

struct OstreamInfo {
    dst: Option<ReadPipe>,
    hub: Option<WriteHub>,
}

pub struct RouterBuilder {
    istream_info: Vec<IstreamInfo>,
    ostream_info: Vec<OstreamInfo>,
}

pub struct IoList {
    pub istream_srcs: Vec<Option<WritePipe>>,
    pub ostream_dsts: Vec<Option<ReadPipe>>,
}

impl Router {
    pub fn stop(self) -> StopErrors {
        StopErrors {
            istream_errors: self
                .readhub_threads
                .into_iter()
                .filter_map(|(idx, thread)| match thread.join() {
                    Ok(result) => match result {
                        Ok(_) => None,
                        Err(e) => Some((idx, e.error)),
                    },
                    Err(_) => Some((idx, Error::from("unexpected panic!(...) in thread"))),
                })
                .collect(),
        }
    }
}

impl RouterBuilder {
    pub fn new() -> Self {
        Self {
            istream_info: Vec::new(),
            ostream_info: Vec::new(),
        }
    }

    pub fn add_istream(
        &mut self,
        stream: Option<ReadPipe>,
        controller: Option<Box<IstreamController>>,
    ) -> IstreamIdx {
        let (hub, controller) = match stream {
            Some(stream) => (Some(ReadHub::new(stream, controller)), None),
            None => (None, controller),
        };

        let idx = IstreamIdx(self.istream_info.len());
        self.istream_info.push(IstreamInfo {
            src: None,
            controller: controller,
            hub: hub,
            listeners: Vec::new(),
        });
        idx
    }

    pub fn add_ostream(&mut self, stream: Option<WritePipe>) -> OstreamIdx {
        let idx = OstreamIdx(self.ostream_info.len());
        self.ostream_info.push(OstreamInfo {
            dst: None,
            hub: stream.map(|p| WriteHub::new(p, idx)),
        });
        idx
    }

    pub fn connect(&mut self, istream_idx: IstreamIdx, ostream_idx: OstreamIdx) -> Result<()> {
        let istream = &mut self.istream_info[istream_idx.0];
        let ostream = &mut self.ostream_info[ostream_idx.0];
        if istream.listeners.iter().any(|x| x.0 == ostream_idx.0) {
            return Ok(());
        }

        istream.listeners.push(ostream_idx);
        if istream.hub.is_none() {
            assert!(istream.src.is_none());
            let (r, w) = pipe::create()?;
            istream.src = Some(w);
            istream.hub = Some(ReadHub::new(r, istream.controller.take()));
        }
        if ostream.hub.is_none() {
            assert!(ostream.dst.is_none());
            let (r, w) = pipe::create()?;
            ostream.dst = Some(r);
            ostream.hub = Some(WriteHub::new(w, ostream_idx));
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
            readhub_threads: Vec::new(),
            output_files: Vec::new(),
        };
        let mut list = IoList {
            istream_srcs: Vec::new(),
            ostream_dsts: Vec::new(),
        };

        for (idx, istream) in self.istream_info.into_iter().enumerate() {
            let is_predefined = istream.src.is_none() && istream.hub.is_some();
            list.istream_srcs.push(match is_predefined {
                true => None,
                false => Some(istream.src.unwrap_or(WritePipe::null()?)),
            });
            if let Some(hub) = istream.hub {
                router.readhub_threads.push((IstreamIdx(idx), hub.spawn()?));
            }
        }

        for ostream in self.ostream_info.into_iter() {
            let is_predefined = ostream.dst.is_none() && ostream.hub.is_some();
            list.ostream_dsts.push(match is_predefined {
                true => None,
                false => Some(ostream.dst.unwrap_or(ReadPipe::null()?)),
            });
            if let Some(hub) = ostream.hub {
                if hub.is_file() {
                    router.output_files.push(hub);
                }
            }
        }

        Ok((list, router))
    }
}
