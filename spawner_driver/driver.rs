use crate::cmd::{Command, Environment};
use crate::io::{IoStreams, StdioMapping};
use crate::misc::mb2b;
use crate::protocol_entities::{Agent, AgentIdx, Controller};
use crate::protocol_handlers::{
    AgentStdout, AgentTermination, ControllerStdout, ControllerTermination,
};
use crate::sys::init_os_specific_process_extensions;

use spawner::dataflow::Graph;
use spawner::pipe;
use spawner::process::{Group, ProcessInfo};
use spawner::{
    Error, IdleTimeLimit, MessageChannel, Report, ResourceLimits, Result, SpawnedProgram, Spawner,
};

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;
use std::sync::mpsc::channel;

pub struct Warnings(RefCell<HashSet<String>>);

pub struct Driver {
    graph: Graph,
    programs: Vec<SpawnedProgram>,
    warnings: Warnings,
    mappings: Vec<StdioMapping>,
}

#[derive(Debug)]
pub struct Errors {
    pub errors: Vec<Error>,
}

pub type DriverResult = std::result::Result<Report, Errors>;

#[derive(Copy, Clone)]
enum Role {
    Default,
    Agent(AgentIdx),
    Controller,
}

impl Warnings {
    pub fn new() -> Self {
        Self(RefCell::new(HashSet::new()))
    }

    pub fn emit<T: ToString>(&self, msg: T) {
        self.0.borrow_mut().insert(msg.to_string());
    }
}

impl fmt::Display for Warnings {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for w in self.0.borrow().iter() {
            write!(f, "warning: {}", w)?;
        }
        Ok(())
    }
}

impl Driver {
    pub fn new(cmds: &Vec<Command>) -> Result<Self> {
        let mut streams = IoStreams::new(cmds)?;
        check_cmds(cmds, &streams.warnings)?;

        let roles = create_roles(cmds);
        let mut senders = Vec::new();
        let mut programs = cmds
            .iter()
            .zip(roles.iter())
            .map(|(cmd, role)| {
                let channel = channel();
                senders.push(channel.0.clone());
                create_program(cmd, channel, *role, &streams.warnings)
            })
            .collect::<Result<Vec<_>>>()?;

        if let Some(controller) = cmds.iter().position(|cmd| cmd.controller) {
            let (r, w) = pipe::create()?;
            let id = streams.graph.add_source(r);
            let controller_mapping = streams.mappings[controller];
            streams.graph.connect(id, controller_mapping.stdin);

            let controller = Controller::new(senders[controller].clone(), w, controller_mapping);
            let agents = roles
                .iter()
                .zip(streams.mappings.iter())
                .zip(senders.iter())
                .filter_map(|((role, mapping), sender)| match role {
                    Role::Agent(idx) => Some(Agent::new(*idx, sender.clone(), *mapping)),
                    _ => None,
                })
                .collect();
            for (program, role) in programs.iter_mut().zip(roles.iter()) {
                init_handlers(&mut streams, program, *role, &controller, &agents);
            }
        }

        streams.optimize()?;

        for (program, stdio) in programs.iter_mut().zip(streams.stdio_list.into_iter()) {
            program.stdio(stdio);
        }

        Ok(Driver {
            graph: streams.graph,
            programs: programs,
            warnings: streams.warnings,
            mappings: streams.mappings,
        })
    }

    pub fn run(self) -> Vec<DriverResult> {
        let mut errors = Vec::new();
        let mut reports = Vec::new();

        let transmitter = self.graph.transmit_data();
        let spawner = Spawner::spawn(self.programs);
        let transmitter_result = transmitter.wait();

        for report in spawner.wait() {
            let mut program_errors = Vec::new();
            match report {
                Ok(report) => reports.push(report),
                Err(e) => program_errors.push(e),
            }
            errors.push(program_errors);
        }

        if let Err(mut transmitter_errors) = transmitter_result {
            for (mapping, program_errors) in self.mappings.into_iter().zip(errors.iter_mut()) {
                for src in &[mapping.stdout, mapping.stderr] {
                    if let Some(err) = transmitter_errors.errors.remove(src) {
                        program_errors.push(err);
                    }
                }
            }
        }

        if errors.iter().all(|e| e.is_empty()) {
            reports.into_iter().map(Ok).collect()
        } else {
            errors
                .into_iter()
                .map(|errs| Err(Errors { errors: errs }))
                .collect()
        }
    }

    pub fn warnings(&self) -> &Warnings {
        &self.warnings
    }
}

impl Role {
    fn is_agent(&self) -> bool {
        match self {
            Role::Agent(_) => true,
            _ => false,
        }
    }
}

impl std::error::Error for Errors {}

impl fmt::Display for Errors {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for e in self.errors.iter() {
            writeln!(f, "{}", e)?;
        }
        Ok(())
    }
}

fn check_cmds(cmds: &Vec<Command>, warnings: &Warnings) -> Result<()> {
    if cmds.iter().filter(|cmd| cmd.controller).count() > 1 {
        return Err(Error::from("There can be at most one controller"));
    }
    for cmd in cmds.iter() {
        assert!(cmd.argv.len() > 0);
        if cmd.debug {
            warnings.emit("'--debug' option has no effect");
        }
        if cmd.delegated {
            warnings.emit("'-runas', '--delegated' options have no effect");
        }
        if cmd.use_syspath {
            warnings.emit("'-c', '--systempath' options have no effect");
        }
        if cmd.shared_memory.is_some() {
            warnings.emit("'--shared-memory' option has no effect");
        }
    }
    Ok(())
}

fn init_handlers(
    streams: &mut IoStreams,
    program: &mut SpawnedProgram,
    role: Role,
    controller: &Controller,
    agents: &Vec<Agent>,
) {
    match role {
        Role::Agent(idx) => {
            let agent = agents[idx.0].clone();
            program.on_terminate(AgentTermination::new(&agent, controller.clone()));
            streams
                .graph
                .source_mut(agent.stdio_mapping().stdout)
                .unwrap()
                .set_handler(AgentStdout::new(agent));
        }
        Role::Controller => {
            program.on_terminate(ControllerTermination::new(agents.clone()));
            streams
                .graph
                .source_mut(controller.stdio_mapping().stdout)
                .unwrap()
                .set_handler(ControllerStdout::new(controller.clone(), agents.clone()));
        }
        _ => {}
    }
}

fn create_program(
    cmd: &Command,
    channel: MessageChannel,
    role: Role,
    warnings: &Warnings,
) -> Result<SpawnedProgram> {
    let mut info = create_process_info(cmd, role);
    let mut group = Group::new()?;
    init_os_specific_process_extensions(cmd, &mut info, &mut group, warnings)?;

    let mut program = SpawnedProgram::new(info);
    program
        .group(group)
        .monitor_interval(cmd.monitor_interval)
        .resource_limits(ResourceLimits {
            wall_clock_time: cmd.wall_clock_time_limit,
            idle_time: cmd.idle_time_limit.map(|limit| IdleTimeLimit {
                total_idle_time: limit,
                cpu_load_threshold: cmd.load_ratio / 100.0,
            }),
            total_user_time: cmd.time_limit,
            max_memory_usage: cmd.memory_limit.map(mb2b),
            total_bytes_written: cmd.write_limit.map(mb2b),
            total_processes_created: cmd.process_count,
            active_processes: cmd.active_process_count,
            active_network_connections: cmd.active_connection_count,
        })
        .wait_for_children(cmd.wait_for_children)
        .msg_channel(channel);
    Ok(program)
}

fn create_process_info(cmd: &Command, role: Role) -> ProcessInfo {
    let mut info = ProcessInfo::new(&cmd.argv[0]);
    info.args(cmd.argv[1..].iter())
        .suspended(role.is_agent())
        .envs(cmd.env_vars.iter().cloned());
    if let Some(ref wd) = cmd.working_directory {
        info.working_dir(wd);
    }
    if let Some(ref uname) = cmd.username {
        info.user(uname, cmd.password.as_ref());
    }
    match cmd.env {
        Environment::Clear => {
            info.env_clear();
        }
        Environment::Inherit => {
            info.env_inherit();
        }
        _ => {}
    }
    info
}

fn create_roles(cmds: &Vec<Command>) -> Vec<Role> {
    if let Some(ctl_pos) = cmds.iter().position(|cmd| cmd.controller) {
        let mut agent_idx = 0;
        cmds.iter()
            .enumerate()
            .map(|(idx, _)| {
                if idx == ctl_pos {
                    Role::Controller
                } else {
                    let agent = Role::Agent(AgentIdx(agent_idx));
                    agent_idx += 1;
                    agent
                }
            })
            .collect()
    } else {
        cmds.iter().map(|_| Role::Default).collect()
    }
}
