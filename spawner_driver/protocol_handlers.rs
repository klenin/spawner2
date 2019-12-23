use crate::protocol_entities::{Agent, AgentIdx, Controller, Message, MessageKind};

use spawner::dataflow::{Connection, DestinationId, SourceReader};
use spawner::pipe::ReadPipe;
use spawner::{Error, OnTerminate, Result};

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::thread;
use std::time::Duration;

pub struct ControllerStdout {
    controller: Controller,
    agents: Vec<Agent>,
    agent_by_stdin_id: HashMap<DestinationId, AgentIdx>,
}

pub struct AgentStdout(Agent);

pub struct AgentTermination(Agent);

pub struct ControllerTermination {
    agents: Vec<Agent>,
}

struct MessageBuf {
    buf: Vec<u8>,
    max_size: usize,
}

impl ControllerStdout {
    pub fn new(controller: Controller, agents: Vec<Agent>) -> Self {
        let agent_by_stdin_id = agents
            .iter()
            .enumerate()
            .map(|(idx, agent)| (agent.stdio_mapping().stdin, AgentIdx(idx)))
            .collect();
        Self {
            controller,
            agents,
            agent_by_stdin_id,
        }
    }

    fn handle_msg(&self, msg: Message, connections: &mut [Connection]) -> Result<()> {
        self.controller.reset_time();

        if let Some(agent_idx) = msg.agent_idx() {
            if agent_idx.0 >= self.agents.len() {
                return Err(Error::from(format!(
                    "Agent index '{}' is out of range",
                    agent_idx.0 + 1,
                )));
            }

            let agent = &self.agents[agent_idx.0];
            match msg.kind() {
                MessageKind::Terminate => agent.terminate(),
                MessageKind::Resume => agent.resume(),
                _ => {}
            }
        }

        self.transmit_msg(msg, connections);
        Ok(())
    }

    fn transmit_msg(&self, msg: Message, connections: &mut [Connection]) {
        for c in connections {
            let agent_idx = self.agent_by_stdin_id.get(&c.destination_id()).copied();

            match (agent_idx, msg.kind()) {
                (Some(_), MessageKind::Data(data)) => {
                    if agent_idx == msg.agent_idx() {
                        c.send(data);
                    }
                }
                (Some(_), _) => {}
                (None, _) => {
                    c.send(msg.as_raw());
                }
            }
        }
    }
}

impl SourceReader for ControllerStdout {
    fn read(&mut self, stdout: &mut ReadPipe, connections: &mut [Connection]) -> Result<()> {
        let mut stdout_reader = BufReader::new(stdout);
        let mut msg_buf = MessageBuf::new();
        let mut data_len = 0;
        loop {
            stdout_reader.consume(data_len);
            let data = stdout_reader.fill_buf().unwrap_or(&[]);
            data_len = data.len();
            if data_len == 0 {
                return Ok(());
            }

            let mut next_msg_data = msg_buf.write(data)?;
            while msg_buf.is_msg_ready() {
                self.handle_msg(msg_buf.as_msg()?, connections)?;
                msg_buf.clear();
                next_msg_data = msg_buf.write(next_msg_data)?;
            }
        }
    }
}

impl AgentStdout {
    pub fn new(agent: Agent) -> Self {
        Self(agent)
    }
}

impl SourceReader for AgentStdout {
    fn read(&mut self, stdout: &mut ReadPipe, connections: &mut [Connection]) -> Result<()> {
        let mut stdout_reader = BufReader::new(stdout);
        let mut msg_buf = MessageBuf::new();
        let msg_prefix = format!("{}#", self.0.idx().0 + 1);
        msg_buf.write(msg_prefix.as_bytes()).unwrap();
        let mut data_len = 0;

        loop {
            stdout_reader.consume(data_len);
            let data = stdout_reader.fill_buf().unwrap_or(&[]);
            data_len = data.len();
            if data_len == 0 {
                break;
            }

            let mut next_msg_data = msg_buf.write(data)?;
            while msg_buf.is_msg_ready() {
                self.0.suspend();

                for c in connections.iter_mut() {
                    c.send(msg_buf.as_slice());
                }

                msg_buf.clear();
                msg_buf.write(msg_prefix.as_bytes()).unwrap();
                next_msg_data = msg_buf.write(next_msg_data)?;
            }
        }

        while !self.0.is_terminated() {
            thread::sleep(Duration::from_millis(1));
        }

        let term_message = format!("{}T#\n", self.0.idx().0 + 1);
        for c in connections.iter_mut() {
            c.send(term_message.as_bytes());
        }

        Ok(())
    }
}

impl AgentTermination {
    pub fn new(agent: Agent) -> Self {
        Self(agent)
    }
}

impl OnTerminate for AgentTermination {
    fn on_terminate(&mut self) {
        self.0.set_terminated();
    }
}

impl Drop for AgentTermination {
    fn drop(&mut self) {
        self.0.set_terminated();
    }
}

impl ControllerTermination {
    pub fn new(agents: Vec<Agent>) -> Self {
        Self { agents }
    }
}

impl OnTerminate for ControllerTermination {
    fn on_terminate(&mut self) {
        for agent in &self.agents {
            agent.resume();
        }
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
            Err(Error::from("Protocol message is too long"))
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
