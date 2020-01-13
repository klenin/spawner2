use crate::io::StdioMapping;

use spawner::dataflow::{DestinationId, SourceId};
use spawner::RunnerMessage;
use spawner::{Error, Result};

use std::char;
use std::str;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

#[derive(Copy, Clone, PartialEq)]
pub struct AgentIdx(pub usize);

#[derive(Clone)]
pub struct Controller {
    sender: Sender<RunnerMessage>,
    mapping: StdioMapping,
}

#[derive(Clone)]
pub struct Agent {
    idx: AgentIdx,
    sender: Sender<RunnerMessage>,
    mapping: StdioMapping,
    is_terminated: Arc<AtomicBool>,
}

pub enum MessageKind<'a> {
    Data(&'a [u8]),
    Terminate,
    Resume,
}

pub struct Message<'a> {
    agent_idx: Option<AgentIdx>,
    kind: MessageKind<'a>,
    raw: &'a [u8],
}

impl Controller {
    pub fn new(sender: Sender<RunnerMessage>, mapping: StdioMapping) -> Self {
        Self { sender, mapping }
    }

    pub fn reset_time(&self) {
        let _ = self.sender.send(RunnerMessage::ResetTime);
    }

    pub fn stdout(&self) -> SourceId {
        self.mapping.stdout
    }

    pub fn stdin(&self) -> DestinationId {
        self.mapping.stdin
    }
}

impl Agent {
    pub fn new(idx: AgentIdx, sender: Sender<RunnerMessage>, mapping: StdioMapping) -> Self {
        Self {
            idx,
            sender,
            mapping,
            is_terminated: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn idx(&self) -> AgentIdx {
        self.idx
    }

    fn send(&self, msg: RunnerMessage) -> &Self {
        let _ = self.sender.send(msg);
        self
    }

    pub fn terminate(&self) {
        self.send(RunnerMessage::Terminate);
    }

    pub fn stop_time_accounting(&self) {
        self.send(RunnerMessage::StopTimeAccounting);
    }

    pub fn suspend(&self) {
        self.send(RunnerMessage::Suspend)
            .send(RunnerMessage::StopTimeAccounting)
            .send(RunnerMessage::ResetTime);
    }

    pub fn resume(&self) {
        self.send(RunnerMessage::Resume)
            .send(RunnerMessage::ResumeTimeAccounting);
    }

    pub fn stdio_mapping(&self) -> StdioMapping {
        self.mapping
    }

    pub fn stdout(&self) -> SourceId {
        self.mapping.stdout
    }

    pub fn stdin(&self) -> DestinationId {
        self.mapping.stdin
    }

    pub fn is_terminated(&self) -> bool {
        self.is_terminated.load(Ordering::SeqCst)
    }

    pub fn set_terminated(&self) {
        self.is_terminated.store(true, Ordering::SeqCst)
    }
}

impl<'a> Message<'a> {
    fn parse_header(header: &'a [u8], msg: &'a [u8]) -> Result<(usize, MessageKind<'a>)> {
        if header.is_empty() {
            return Err(Error::from("Missing header in controller message"));
        }

        let header_str = str::from_utf8(header)
            .map_err(|_| Error::from("Invalid header in controller message"))?;

        let mut num_digits = 0;
        for c in header_str.chars() {
            if char::is_digit(c, 10) {
                num_digits += 1;
            } else {
                break;
            }
        }

        let agent_idx = usize::from_str_radix(&header_str[..num_digits], 10).map_err(|_| {
            Error::from(format!(
                "Unable to parse agent index '{}'",
                &header_str[..num_digits]
            ))
        })?;

        match &header_str[num_digits..] {
            "" => Ok((agent_idx, MessageKind::Data(msg))),
            "W" => Ok((agent_idx, MessageKind::Resume)),
            "S" => Ok((agent_idx, MessageKind::Terminate)),
            _ => Err(Error::from(format!(
                "Invalid controller command '{}' in '{}'",
                &header_str[num_digits..],
                header_str
            ))),
        }
    }

    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.is_empty() {
            return Err(Error::from("Empty controller message"));
        }
        if !data.ends_with(&[b'\n']) {
            return Err(Error::from("Controller message must end with '\n'"));
        }

        let (header, msg) = match data.iter().position(|&x| x == b'#') {
            Some(hash_pos) => (&data[..hash_pos], &data[hash_pos + 1..]),
            None => return Err(Error::from("Missing '#' in controller message")),
        };

        Message::parse_header(header, msg).map(|(agent_idx, kind)| Self {
            agent_idx: match agent_idx {
                0 => None,
                x => Some(AgentIdx(x - 1)),
            },
            kind,
            raw: data,
        })
    }

    pub fn kind(&self) -> &MessageKind {
        &self.kind
    }

    pub fn agent_idx(&self) -> Option<AgentIdx> {
        self.agent_idx
    }

    pub fn as_raw(&self) -> &[u8] {
        self.raw
    }
}
