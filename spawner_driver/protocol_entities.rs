use spawner::dataflow::{DestinationId, SourceId};
use spawner::{Error, ProgramMessage, Result, StdioMapping};

use std::char;
use std::str;
use std::sync::mpsc::Sender;

#[derive(Copy, Clone, PartialEq)]
pub struct AgentIdx(pub usize);

#[derive(Clone)]
pub struct Controller {
    sender: Sender<ProgramMessage>,
    mapping: StdioMapping,
}

#[derive(Clone)]
pub struct Agent {
    idx: AgentIdx,
    sender: Sender<ProgramMessage>,
    mapping: StdioMapping,
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
    pub fn new(sender: Sender<ProgramMessage>, mapping: StdioMapping) -> Self {
        Self { sender, mapping }
    }

    fn send(&self, msg: ProgramMessage) -> &Self {
        let _ = self.sender.send(msg);
        self
    }

    pub fn reset_time(&self) {
        self.send(ProgramMessage::ResetTime);
    }

    pub fn terminate(&self) {
        self.send(ProgramMessage::Terminate);
    }

    pub fn stdout(&self) -> SourceId {
        self.mapping.stdout
    }

    pub fn stdin(&self) -> DestinationId {
        self.mapping.stdin
    }
}

impl Agent {
    pub fn new(idx: AgentIdx, sender: Sender<ProgramMessage>, mapping: StdioMapping) -> Self {
        Self {
            idx,
            sender,
            mapping,
        }
    }

    pub fn idx(&self) -> AgentIdx {
        self.idx
    }

    fn send(&self, msg: ProgramMessage) -> &Self {
        let _ = self.sender.send(msg);
        self
    }

    pub fn terminate(&self) {
        self.send(ProgramMessage::Terminate);
    }

    pub fn stop_time_accounting(&self) {
        self.send(ProgramMessage::StopTimeAccounting);
    }

    pub fn suspend(&self) {
        self.send(ProgramMessage::Suspend)
            .send(ProgramMessage::StopTimeAccounting)
            .send(ProgramMessage::ResetTime);
    }

    pub fn resume(&self) {
        self.send(ProgramMessage::Resume)
            .send(ProgramMessage::ResumeTimeAccounting);
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
