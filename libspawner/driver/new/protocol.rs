use crate::{Error, Result};
use command::OnTerminate;
use pipe::WritePipe;
use runner::Runner;
use session::StdioMapping;
use std::char;
use std::collections::HashMap;
use std::io::{self, Write};
use std::str;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use stdio::{IstreamController, IstreamListeners, OstreamIdx};

#[derive(Copy, Clone, PartialEq)]
pub struct CommandIdx(pub usize);
#[derive(Copy, Clone, PartialEq)]
pub struct AgentIdx(pub usize);

struct ContextInner {
    runners: Vec<Runner>,
    mappings: Vec<StdioMapping>,
}

#[derive(Clone)]
pub struct Context {
    inner: Arc<Mutex<Option<ContextInner>>>,
}

#[derive(Clone)]
pub struct ControllerStdin {
    stdin: Arc<Mutex<WritePipe>>,
}

pub struct ControllerStdout {
    ctx: Context,
    controller_idx: CommandIdx,
    agent_indices: Vec<CommandIdx>,
    ostream_to_agent: HashMap<OstreamIdx, CommandIdx>,
    buf: MessageBuf,
}

pub struct AgentStdout {
    ctx: Context,
    cmd_idx: CommandIdx,
    msg_prefix: String,
    buf: MessageBuf,
}

pub struct AgentTermination {
    idx: AgentIdx,
    stdin: ControllerStdin,
}

struct MessageBuf {
    buf: Vec<u8>,
    max_size: usize,
}

enum MessageKind<'a> {
    Message(&'a [u8]),
    Terminate,
    Resume,
}

struct Message<'a> {
    agent_idx: Option<AgentIdx>,
    kind: MessageKind<'a>,
}

impl Context {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    pub fn init(&mut self, runners: Vec<Runner>, mappings: Vec<StdioMapping>) {
        *self.inner.lock().unwrap() = Some(ContextInner {
            runners: runners,
            mappings: mappings,
        });
    }

    fn wait_for_init(&self) -> Result<()> {
        for _ in 0..1000 {
            if self.inner.lock().unwrap().is_some() {
                return Ok(());
            }
            thread::sleep(Duration::from_micros(100));
        }
        Err(Error::from("Context haven't been initialized for too long"))
    }

    fn runner(&self, idx: CommandIdx) -> Runner {
        self.inner.lock().unwrap().as_ref().unwrap().runners[idx.0].clone()
    }
}

impl ControllerStdin {
    pub fn new(stdin: WritePipe) -> Self {
        Self {
            stdin: Arc::new(Mutex::new(stdin)),
        }
    }
}

impl Write for ControllerStdin {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.stdin.lock().unwrap().write(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stdin.lock().unwrap().flush()
    }
}

impl ControllerStdout {
    pub fn new(ctx: Context, controller_idx: CommandIdx, agent_indices: Vec<CommandIdx>) -> Self {
        Self {
            ctx: ctx,
            controller_idx: controller_idx,
            agent_indices: agent_indices,
            ostream_to_agent: HashMap::new(),
            buf: MessageBuf::new(),
        }
    }

    fn init(&mut self) -> Result<()> {
        if self.ostream_to_agent.is_empty() {
            self.ctx.wait_for_init()?;
            let mtx_guard = self.ctx.inner.lock().unwrap();
            let mappings = &mtx_guard.as_ref().unwrap().mappings;
            self.ostream_to_agent = self
                .agent_indices
                .iter()
                .map(|&i| (mappings[i.0].stdin, i))
                .collect();
        }
        Ok(())
    }

    fn handle_msg(&mut self, listeners: &mut IstreamListeners) -> Result<()> {
        self.init()?;
        self.ctx.runner(self.controller_idx).reset_timers();

        let msg = self.buf.as_msg()?;
        if let Some(agent_idx) = msg.agent_idx {
            if agent_idx.0 >= self.agent_indices.len() {
                return Err(Error::from(format!(
                    "Agent index '{}' is out of range",
                    agent_idx.0 + 1,
                )));
            }

            let agent = self.ctx.runner(self.agent_indices[agent_idx.0]);
            match msg.kind {
                MessageKind::Terminate => agent.terminate(),
                MessageKind::Resume => agent.resume(),
                _ => {}
            }
        }

        for i in 0..listeners.len() {
            let mut listener = listeners.at(i);
            let agent_idx = self
                .ostream_to_agent
                .get(&listener.ostream_idx())
                .map(|i| *i);
            if agent_idx.is_none() {
                listener.write(self.buf.as_slice());
            } else if let MessageKind::Message(data) = msg.kind {
                if agent_idx == msg.agent_idx.map(|i| self.agent_indices[i.0]) {
                    listener.write(data);
                }
            }
        }
        Ok(())
    }
}

impl IstreamController for ControllerStdout {
    fn handle_data(&mut self, data: &[u8], listeners: &mut IstreamListeners) -> Result<()> {
        let mut next_msg = self.buf.write(data)?;
        while self.buf.is_msg_ready() {
            self.handle_msg(listeners)?;
            self.buf.clear();
            next_msg = self.buf.write(next_msg)?;
        }
        Ok(())
    }
}

impl AgentStdout {
    pub fn new(ctx: Context, agent_idx: AgentIdx, cmd_idx: CommandIdx) -> Self {
        let mut buf = MessageBuf::new();
        let msg_prefix = format!("{}#", agent_idx.0 + 1);
        buf.write(msg_prefix.as_bytes()).unwrap();
        Self {
            ctx: ctx,
            cmd_idx: cmd_idx,
            msg_prefix: msg_prefix,
            buf: buf,
        }
    }

    fn agent(&mut self) -> Result<Runner> {
        self.ctx
            .wait_for_init()
            .map(|_| self.ctx.runner(self.cmd_idx))
    }
}

impl IstreamController for AgentStdout {
    fn handle_data(&mut self, data: &[u8], listeners: &mut IstreamListeners) -> Result<()> {
        let mut next_msg = self.buf.write(data)?;
        while self.buf.is_msg_ready() {
            let agent = self.agent()?;
            agent.suspend();
            agent.reset_timers();

            for i in 0..listeners.len() {
                listeners.at(i).write(self.buf.as_slice());
            }

            self.buf.clear();
            self.buf.write(self.msg_prefix.as_bytes()).unwrap();
            next_msg = self.buf.write(next_msg)?;
        }
        Ok(())
    }
}

impl AgentTermination {
    pub fn new(agent_idx: AgentIdx, stdin: ControllerStdin) -> Self {
        Self {
            idx: agent_idx,
            stdin: stdin,
        }
    }
}

impl OnTerminate for AgentTermination {
    fn on_terminate(&mut self) {
        let _ = self
            .stdin
            .write_all(format!("{}T#\n", self.idx.0 + 1).as_bytes());
    }
}

impl MessageBuf {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            max_size: 65536, // Default buffer size in c++ spawner.
        }
    }

    fn write<'a>(&mut self, data: &'a [u8]) -> Result<&'a [u8]> {
        let data_len = match data.iter().position(|&b| b == b'\n') {
            Some(pos) => pos + 1,
            None => data.len(),
        };

        if data_len > (self.max_size - self.buf.len()) {
            Err(Error::from("Controller message is too long"))
        } else {
            self.buf.extend(&data[..data_len]);
            Ok(&data[data_len..])
        }
    }

    fn clear(&mut self) {
        self.buf.clear();
    }

    fn is_msg_ready(&self) -> bool {
        self.buf.ends_with(&[b'\n'])
    }

    fn as_slice(&self) -> &[u8] {
        self.buf.as_slice()
    }

    fn as_msg(&self) -> Result<Message> {
        Message::parse(self.as_slice())
    }
}

impl<'a> Message<'a> {
    fn parse_header(header: &'a [u8], msg: &'a [u8]) -> Result<(usize, MessageKind<'a>)> {
        if header.len() == 0 {
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
            "" => Ok((agent_idx, MessageKind::Message(msg))),
            "W" => Ok((agent_idx, MessageKind::Resume)),
            "S" => Ok((agent_idx, MessageKind::Terminate)),
            _ => Err(Error::from(format!(
                "Invalid controller command '{}' in '{}'",
                &header_str[num_digits + 1..],
                header_str
            ))),
        }
    }

    fn parse(data: &'a [u8]) -> Result<Self> {
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

        let (agent_idx, kind) = Message::parse_header(header, msg)?;
        Ok(Self {
            agent_idx: match agent_idx {
                0 => None,
                x => Some(AgentIdx(x - 1)),
            },
            kind: kind,
        })
    }
}
