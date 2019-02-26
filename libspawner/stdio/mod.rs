mod hub;
pub(crate) mod router;

use crate::Result;
use pipe::{ReadPipe, WritePipe};
use std::io::Write;
use stdio::hub::WriteHub;

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct IstreamIdx(pub usize);
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct OstreamIdx(pub usize);

pub struct Istream {
    pipe: ReadPipe,
    controller: Option<Box<IstreamController>>,
}

pub struct IstreamListeners<'a> {
    write_hubs: &'a mut [WriteHub],
    num_errors: usize,
}

pub struct IstreamListener<'a, 'b> {
    listeners: &'a mut IstreamListeners<'b>,
    idx: usize,
}

pub trait IstreamController: Send {
    fn handle_data(&mut self, data: &[u8], listeners: &mut IstreamListeners) -> Result<()>;
}

pub struct Ostream {
    pipe: WritePipe,
}

impl Istream {
    pub fn new(pipe: ReadPipe, ctl: Option<Box<IstreamController>>) -> Self {
        Self {
            pipe: pipe,
            controller: ctl,
        }
    }
}

impl Ostream {
    pub fn new(pipe: WritePipe) -> Self {
        Self { pipe: pipe }
    }
}

impl<'a> IstreamListeners<'a> {
    pub fn len(&self) -> usize {
        self.write_hubs.len()
    }

    pub fn at<'b>(&'b mut self, i: usize) -> IstreamListener<'b, 'a> {
        IstreamListener {
            listeners: self,
            idx: i,
        }
    }
}

impl<'a, 'b> IstreamListener<'a, 'b> {
    pub fn ostream_idx(&self) -> OstreamIdx {
        self.listeners.write_hubs[self.idx].ostream_idx()
    }

    pub fn write(&mut self, data: &[u8]) {
        let wh = &mut self.listeners.write_hubs[self.idx];
        if wh.write_all(data).is_err() {
            self.listeners.num_errors += 1;
        }
    }
}
