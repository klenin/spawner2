use crate::dataflow::{DestinationId, Graph, SourceId};
use crate::pipe::{ReadPipe, WritePipe};
use crate::Result;

use std::collections::HashSet;

pub enum SourceOptimization {
    None,
    ReplaceWithNull,
    Inline(DestinationId),
}

pub enum DestinationOptimization {
    None,
    ReplaceWithNull,
    Inline(SourceId),
}

pub struct DataflowAnalyzer<'a>(&'a Graph);

pub struct DataflowOptimizer<'a, 'b, 'c> {
    graph: &'a mut Graph,
    ignored_srcs: &'b HashSet<SourceId>,
    ignored_dsts: &'c HashSet<DestinationId>,
}

impl<'a> DataflowAnalyzer<'a> {
    pub fn new(g: &'a Graph) -> Self {
        Self(g)
    }

    pub fn analyze_source(&self, id: SourceId) -> SourceOptimization {
        let src = match self.0.source(id) {
            Some(x) => x,
            None => return SourceOptimization::None,
        };
        if src.has_reader() {
            return SourceOptimization::None;
        }
        match src.edges().len() {
            0 => SourceOptimization::ReplaceWithNull,
            1 => {
                let dst_id = src.edges()[0];
                let dst = self.0.destination(dst_id).unwrap();
                if dst.edges().len() == 1 {
                    SourceOptimization::Inline(dst_id)
                } else {
                    SourceOptimization::None
                }
            }
            _ => SourceOptimization::None,
        }
    }

    pub fn analyze_destination(&self, id: DestinationId) -> DestinationOptimization {
        let dst = match self.0.destination(id) {
            Some(x) => x,
            None => return DestinationOptimization::None,
        };
        match dst.edges().len() {
            0 => DestinationOptimization::ReplaceWithNull,
            1 => {
                let src_id = dst.edges()[0];
                let src = self.0.source(src_id).unwrap();
                if src.edges().len() == 1 && !src.has_reader() {
                    DestinationOptimization::Inline(src_id)
                } else {
                    DestinationOptimization::None
                }
            }
            _ => DestinationOptimization::None,
        }
    }
}

impl<'a, 'b, 'c> DataflowOptimizer<'a, 'b, 'c> {
    pub fn new(
        graph: &'a mut Graph,
        ignored_srcs: &'b HashSet<SourceId>,
        ignored_dsts: &'c HashSet<DestinationId>,
    ) -> Self {
        Self {
            graph,
            ignored_srcs,
            ignored_dsts,
        }
    }

    fn is_src_ingored(&self, id: SourceId) -> bool {
        self.ignored_srcs.get(&id).is_some()
    }

    fn is_dst_ingored(&self, id: DestinationId) -> bool {
        self.ignored_dsts.get(&id).is_some()
    }

    pub fn optimize_source(&mut self, src_id: SourceId, src_writer: &mut WritePipe) -> Result<()> {
        if self.is_src_ingored(src_id) {
            return Ok(());
        }
        match DataflowAnalyzer::new(&self.graph).analyze_source(src_id) {
            SourceOptimization::None => Ok(()),
            SourceOptimization::ReplaceWithNull => {
                self.graph.remove_source(src_id);
                *src_writer = WritePipe::null()?;
                Ok(())
            }
            SourceOptimization::Inline(dst_id) => {
                if self.is_dst_ingored(dst_id) {
                    return Ok(());
                }
                self.graph.remove_source(src_id);
                *src_writer = self.graph.remove_destination(dst_id).unwrap();
                Ok(())
            }
        }
    }

    pub fn optimize_destination(
        &mut self,
        dst_id: DestinationId,
        dst_reader: &mut ReadPipe,
    ) -> Result<()> {
        if self.is_dst_ingored(dst_id) {
            return Ok(());
        }
        match DataflowAnalyzer::new(&self.graph).analyze_destination(dst_id) {
            DestinationOptimization::None => Ok(()),
            DestinationOptimization::ReplaceWithNull => {
                self.graph.remove_destination(dst_id);
                *dst_reader = ReadPipe::null()?;
                Ok(())
            }
            DestinationOptimization::Inline(src_id) => {
                if self.is_src_ingored(src_id) {
                    return Ok(());
                }
                self.graph.remove_destination(dst_id);
                *dst_reader = self.graph.remove_source(src_id).unwrap();
                Ok(())
            }
        }
    }
}
