mod hub;
pub(crate) mod router;

use crate::Result;
use std::io::Write;
use stdio::hub::WriteHub;

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct IstreamIdx(pub usize);
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct OstreamIdx(pub usize);

pub struct Ostreams<'a>(&'a mut [WriteHub]);
pub struct Ostream<'a>(&'a mut WriteHub);
pub struct OstreamsIterMut<'a>(std::slice::IterMut<'a, WriteHub>);

pub trait IstreamController: Send {
    fn handle_data(&mut self, data: &[u8], ostreams: Ostreams) -> Result<()>;
}

impl<'a> Ostreams<'a> {
    pub fn iter_mut(&mut self) -> OstreamsIterMut {
        OstreamsIterMut(self.0.iter_mut())
    }
}

impl<'a> Ostream<'a> {
    pub fn write(&mut self, data: &[u8]) {
        let _ = self.0.write_all(data);
    }

    pub fn idx(&self) -> OstreamIdx {
        self.0.ostream_idx()
    }
}

impl<'a> Iterator for OstreamsIterMut<'a> {
    type Item = Ostream<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.0.next() {
            Some(hub) => Some(Ostream(hub)),
            None => None,
        }
    }
}
