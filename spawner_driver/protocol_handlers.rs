use crate::protocol_entities::{Agent, AgentIdx, Controller, Message, MessageKind};

use spawner::dataflow::{Connection, DestinationId, OnRead};
use spawner::{Error, OnTerminate, Result};

use std::collections::HashMap;
use std::io::Write;

pub struct ControllerStdout {
    controller: Controller,
    agents: Vec<Agent>,
    agent_by_stdin_id: HashMap<DestinationId, AgentIdx>,
    buf: MessageBuf,
}

pub struct AgentStdout {
    agent: Agent,
    msg_prefix: String,
    buf: MessageBuf,
}

pub struct AgentTermination {
    msg: String,
    controller: Controller,
}

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
            controller: controller,
            agents: agents,
            agent_by_stdin_id: agent_by_stdin_id,
            buf: MessageBuf::new(),
        }
    }

    fn handle_msg(&self, connections: &mut [Connection]) -> Result<()> {
        self.controller.reset_time();

        let msg = self.buf.as_msg()?;
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
        for connection in connections {
            let agent_idx = self
                .agent_by_stdin_id
                .get(&connection.destination_id())
                .map(|&i| i);

            match (agent_idx, msg.kind()) {
                (Some(_), MessageKind::Data(data)) => {
                    if agent_idx == msg.agent_idx() {
                        connection.send(data);
                    }
                }
                (Some(_), _) => {}
                (None, _) => {
                    connection.send(self.buf.as_slice());
                }
            }
        }
    }
}

impl OnRead for ControllerStdout {
    fn on_read(&mut self, data: &[u8], connections: &mut [Connection]) -> Result<()> {
        let mut next_msg = self.buf.write(data)?;
        while self.buf.is_msg_ready() {
            self.handle_msg(connections)?;
            self.buf.clear();
            next_msg = self.buf.write(next_msg)?;
        }
        Ok(())
    }
}

impl AgentStdout {
    pub fn new(agent: Agent) -> Self {
        let mut buf = MessageBuf::new();
        let msg_prefix = format!("{}#", agent.idx().0 + 1);
        buf.write(msg_prefix.as_bytes()).unwrap();
        Self {
            agent: agent,
            msg_prefix: msg_prefix,
            buf: buf,
        }
    }
}

impl OnRead for AgentStdout {
    fn on_read(&mut self, data: &[u8], connections: &mut [Connection]) -> Result<()> {
        let mut next_msg = self.buf.write(data)?;
        while self.buf.is_msg_ready() {
            self.agent.suspend();

            for connection in connections.iter_mut() {
                connection.send(self.buf.as_slice());
            }

            self.buf.clear();
            self.buf.write(self.msg_prefix.as_bytes()).unwrap();
            next_msg = self.buf.write(next_msg)?;
        }
        Ok(())
    }
}

impl AgentTermination {
    pub fn new(agent: &Agent, controller: Controller) -> Self {
        Self {
            msg: format!("{}T#\n", agent.idx().0 + 1),
            controller: controller,
        }
    }
}

impl OnTerminate for AgentTermination {
    fn on_terminate(&mut self) {
        let _ = self.controller.stdin().write_all(self.msg.as_bytes());
    }
}

impl ControllerTermination {
    pub fn new(agents: Vec<Agent>) -> Self {
        Self { agents: agents }
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
