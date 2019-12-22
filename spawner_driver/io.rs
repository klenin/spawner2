use crate::cmd::{Command, RedirectFlags, RedirectKind, RedirectList};
use crate::driver::Warnings;
use crate::sys::{open_input_file, open_output_file};

use spawner::dataflow::{DestinationId, Graph, SourceId};
use spawner::pipe::{self, ReadPipe, WritePipe};
use spawner::process::Stdio;
use spawner::{Error, Result};

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Copy, Clone)]
pub struct StdioMapping {
    pub stdin: DestinationId,
    pub stdout: SourceId,
    pub stderr: SourceId,
}

pub struct IoStreams {
    pub graph: Graph,
    pub stdio_list: Vec<Stdio>,
    pub warnings: Warnings,
    pub mappings: Vec<StdioMapping>,

    output_files: HashMap<PathBuf, DestinationId>,
    exclusive_input_files: HashMap<PathBuf, SourceId>,
}

impl IoStreams {
    pub fn new(cmds: &Vec<Command>) -> Result<Self> {
        let mut data = Self {
            graph: Graph::new(),
            stdio_list: Vec::new(),
            mappings: Vec::new(),
            warnings: Warnings::new(),
            output_files: HashMap::new(),
            exclusive_input_files: HashMap::new(),
        };

        for _ in cmds {
            data.add_stdio()?;
        }

        for (idx, cmd) in cmds.iter().enumerate() {
            let mapping = data.mappings[idx];
            data.redirect_destination(mapping.stdin, &cmd.stdin_redirect)?;
            data.redirect_source(mapping.stdout, &cmd.stdout_redirect)?;
            data.redirect_source(mapping.stderr, &cmd.stderr_redirect)?;
        }

        Ok(data)
    }

    pub fn optimize(&mut self) -> Result<()> {
        for (mapping, stdio) in self.mappings.iter().zip(self.stdio_list.iter_mut()) {
            optimize_istream(&mut self.graph, mapping.stdin, &mut stdio.stdin)?;
            optimize_ostream(&mut self.graph, mapping.stdout, &mut stdio.stdout)?;
            optimize_ostream(&mut self.graph, mapping.stderr, &mut stdio.stderr)?;
        }
        Ok(())
    }

    fn add_stdio(&mut self) -> Result<()> {
        let (stdin_r, stdin_w) = pipe::create()?;
        let (stdout_r, stdout_w) = pipe::create()?;
        let (stderr_r, stderr_w) = pipe::create()?;
        self.stdio_list.push(Stdio {
            stdin: stdin_r,
            stdout: stdout_w,
            stderr: stderr_w,
        });
        self.mappings.push(StdioMapping {
            stdin: self.graph.add_destination(stdin_w),
            stdout: self.graph.add_source(stdout_r),
            stderr: self.graph.add_source(stderr_r),
        });
        Ok(())
    }

    fn redirect_destination(
        &mut self,
        dst: DestinationId,
        redirect_list: &RedirectList,
    ) -> Result<()> {
        for redirect in redirect_list.items.iter() {
            let src = match &redirect.kind {
                RedirectKind::File(f) => self.open_input_file(f, redirect.flags)?,
                RedirectKind::Stdout(i) => self.get_mapping("Stdout", *i)?.stdout,
                _ => continue,
            };
            self.graph.connect(src, dst);
        }
        Ok(())
    }

    fn redirect_source(&mut self, src: SourceId, redirect_list: &RedirectList) -> Result<()> {
        for redirect in redirect_list.items.iter() {
            let dst = match &redirect.kind {
                RedirectKind::File(f) => self.open_output_file(f, redirect.flags)?,
                RedirectKind::Stdin(i) => self.get_mapping("Stdin", *i)?.stdin,
                _ => continue,
            };
            self.graph.connect(src, dst);
        }
        Ok(())
    }

    fn open_input_file(&mut self, path: &String, flags: RedirectFlags) -> Result<SourceId> {
        let path = canonicalize(path)?;
        match self.exclusive_input_files.get(&path).map(|&id| id) {
            Some(id) => Ok(id),
            None => {
                let pipe = open_input_file(&path, flags, &self.warnings)?;
                let id = self.graph.add_source(pipe);
                if flags.exclusive {
                    self.exclusive_input_files.insert(path, id);
                }
                Ok(id)
            }
        }
    }

    fn open_output_file(&mut self, path: &String, flags: RedirectFlags) -> Result<DestinationId> {
        let path = canonicalize(path)?;
        match self.output_files.get(&path).map(|&id| id) {
            Some(id) => Ok(id),
            None => {
                let pipe = open_output_file(&path, flags, &self.warnings)?;
                let id = self.graph.add_file_destination(pipe);
                self.output_files.insert(path, id);
                Ok(id)
            }
        }
    }

    fn get_mapping(&self, stream_name: &str, i: usize) -> Result<StdioMapping> {
        if i >= self.mappings.len() {
            Err(Error::from(format!(
                "{} index '{}' is out of range",
                stream_name, i
            )))
        } else {
            Ok(self.mappings[i])
        }
    }
}

fn canonicalize(path: &String) -> Result<PathBuf> {
    if !Path::exists(path.as_ref()) {
        fs::File::create(path).map_err(|_| Error::from(format!("Unable to create '{}'", path)))?;
    }
    fs::canonicalize(path).map_err(|_| Error::from(format!("Unable to open '{}'", path)))
}

fn optimize_ostream(graph: &mut Graph, src_id: SourceId, pipe: &mut WritePipe) -> Result<()> {
    let num_edges = match graph.source(src_id) {
        Some(src) => {
            let num_edges = src.edges().len();
            if num_edges > 1 || src.has_reader() {
                return Ok(());
            }
            num_edges
        }
        None => return Ok(()),
    };

    if num_edges == 0 {
        graph.remove_source(src_id);
        *pipe = WritePipe::null()?;
        return Ok(());
    }

    let dst_id = graph.source(src_id).map(|src| src.edges()[0]).unwrap();
    if !graph.destination(dst_id).unwrap().edges().is_empty() {
        return Ok(());
    }

    graph.remove_source(src_id);
    *pipe = graph.remove_destination(dst_id).unwrap();
    Ok(())
}

fn optimize_istream(graph: &mut Graph, dst_id: DestinationId, pipe: &mut ReadPipe) -> Result<()> {
    let num_edges = match graph.destination(dst_id).map(|dst| dst.edges().len()) {
        Some(num_edges) => num_edges,
        None => return Ok(()),
    };

    if num_edges == 0 {
        graph.remove_destination(dst_id);
        *pipe = ReadPipe::null()?;
        return Ok(());
    } else if num_edges > 1 {
        return Ok(());
    }

    let src_id = graph.destination(dst_id).map(|dst| dst.edges()[0]).unwrap();
    if graph
        .source(src_id)
        .map(|src| src.edges().len() != 1 || src.has_reader())
        .unwrap()
    {
        return Ok(());
    }

    graph.remove_destination(dst_id);
    *pipe = graph.remove_source(src_id).unwrap();
    Ok(())
}
