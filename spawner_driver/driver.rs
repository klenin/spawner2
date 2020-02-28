use crate::cmd::{Command, Environment, RedirectFlags, RedirectKind, RedirectList};
use crate::misc::mb2b;
use crate::protocol_entities::{Agent, AgentIdx, Controller};
use crate::protocol_handlers::{AgentStdout, ControllerStdout};
use crate::report::Report;
use crate::sys::{
    init_os_specific_process_extensions, open_input_file, open_output_file, ConsoleReader,
};

use spawner::dataflow::{DestinationId, Graph, SourceId};
use spawner::pipe::{self, WritePipe};
use spawner::process::{Group, ProcessInfo};
use spawner::{
    Error, IdleTimeLimit, Program, ProgramMessage, ResourceLimits, Result, Session, StdioMapping,
};

use spawner_opts::CmdLineOptions;

use json::JsonValue;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

pub struct Warnings(RefCell<HashSet<String>>);

pub struct Driver {
    sess: Session,
    cmds: Vec<Command>,
    warnings: Warnings,
    stdio: DriverStdio,
}

/// All redirects to *std are redirected here.
/// We don't want to redirect directly to STDIN\STDOUT handles since it may result in undefined behaviour.
struct DriverStdio {
    stdin_w: Option<WritePipe>,
}

struct StdioLinker<'w, 's, 'm> {
    mappings: &'m [StdioMapping],
    sess: &'s mut Session,
    stdin: Option<(WritePipe, SourceId)>,
    warnings: &'w Warnings,
    output_files: HashMap<PathBuf, DestinationId>,
    exclusive_input_files: HashMap<PathBuf, SourceId>,
}

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
            writeln!(f, "warning: {}", w)?;
        }
        Ok(())
    }
}

impl Driver {
    pub fn from_argv<T, U>(argv: T) -> Result<Self>
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>,
    {
        let warnings = Warnings::new();
        let cmds = parse_argv(argv)?;
        check_cmds(&cmds, &warnings)?;

        let mut sess = Session::new();
        let mut senders = Vec::new();
        let roles = create_roles(&cmds);
        let mappings = cmds
            .iter()
            .zip(roles.iter())
            .map(|(cmd, role)| {
                let channel = channel();
                senders.push(channel.0.clone());
                create_program(cmd, channel.1, *role, &warnings).and_then(|p| sess.add_program(p))
            })
            .collect::<Result<Vec<_>>>()?;

        let stdio = StdioLinker::new(&mut sess, &mappings, &warnings).link(&cmds)?;

        if let Some(controller) = cmds.iter().position(|cmd| cmd.controller) {
            // Initialize protocol entities.
            let controller = Controller::new(senders[controller].clone(), mappings[controller]);
            let agents = roles
                .iter()
                .zip(mappings.iter())
                .zip(senders.iter())
                .filter_map(|((role, mapping), sender)| match role {
                    Role::Agent(idx) => Some(Agent::new(*idx, sender.clone(), *mapping)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            check_protocol_entities(&controller, &agents, sess.graph(), &warnings);

            for entity in roles {
                init_entity_handler(entity, sess.graph_mut(), &controller, &agents);
            }
            for agent in &agents {
                agent.stop_time_accounting();
            }
        }

        Ok(Self {
            sess,
            cmds,
            warnings,
            stdio,
        })
    }

    pub fn run(self) -> Result<Vec<Report>> {
        eprint!("{}", self.warnings);

        let cmds = self.cmds;
        let run = self.sess.run()?;

        if let Some(stdin) = self.stdio.stdin_w {
            ConsoleReader::spawn(stdin).join(&run);
        }

        let reports = run
            .wait()
            .into_iter()
            .zip(cmds.iter())
            .map(|(r, c)| Report::new(c, r))
            .collect::<Vec<_>>();
        if reports.is_empty() {
            Command::print_help();
        } else {
            print_reports(&cmds, &reports)?;
        }
        Ok(reports)
    }
}

impl<'w, 's, 'm> StdioLinker<'w, 's, 'm> {
    fn new(sess: &'s mut Session, mappings: &'m [StdioMapping], warnings: &'w Warnings) -> Self {
        Self {
            sess,
            mappings,
            stdin: None,
            warnings,
            output_files: HashMap::new(),
            exclusive_input_files: HashMap::new(),
        }
    }

    fn link(mut self, cmds: &[Command]) -> Result<DriverStdio> {
        for (idx, cmd) in cmds.iter().enumerate() {
            let mapping = self.mappings[idx];
            self.redirect_destination(mapping.stdin, &cmd.stdin_redirect)?;
            self.redirect_source(mapping.stdout, &cmd.stdout_redirect)?;
            self.redirect_source(mapping.stderr, &cmd.stderr_redirect)?;
        }
        Ok(DriverStdio {
            stdin_w: self.stdin.map(|s| s.0),
        })
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
                RedirectKind::Std => {
                    if self.stdin.is_none() {
                        let (r, w) = pipe::create()?;
                        self.stdin = Some((w, self.sess.graph_mut().add_source(r)));
                    }
                    self.stdin.as_ref().unwrap().1
                }
                _ => continue,
            };
            self.sess.graph_mut().connect(src, dst);
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
            self.sess.graph_mut().connect(src, dst);
        }
        Ok(())
    }

    fn open_input_file(&mut self, path: &str, flags: RedirectFlags) -> Result<SourceId> {
        let path = canonicalize(path)?;
        match self.exclusive_input_files.get(&path).copied() {
            Some(id) => Ok(id),
            None => {
                let pipe = open_input_file(&path, flags, &self.warnings)?;
                let id = self.sess.graph_mut().add_source(pipe);
                if flags.exclusive {
                    self.exclusive_input_files.insert(path, id);
                    // Avoid inlining to keep pipe open as long as possible.
                    self.sess.disable_source_optimization(id);
                }
                Ok(id)
            }
        }
    }

    fn open_output_file(&mut self, path: &str, flags: RedirectFlags) -> Result<DestinationId> {
        let path = canonicalize(path)?;
        match self.output_files.get(&path).copied() {
            Some(id) => Ok(id),
            None => {
                let pipe = open_output_file(&path, flags, &self.warnings)?;
                let id = self.sess.graph_mut().add_file_destination(pipe);
                self.output_files.insert(path, id);
                if flags.exclusive {
                    // Avoid inlining to keep pipe open as long as possible.
                    self.sess.disable_destination_optimization(id);
                }
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

fn canonicalize(path: &str) -> Result<PathBuf> {
    if !Path::exists(path.as_ref()) {
        fs::File::create(path).map_err(|_| Error::from(format!("Unable to create '{}'", path)))?;
    }
    fs::canonicalize(path).map_err(|_| Error::from(format!("Unable to open '{}'", path)))
}

impl Role {
    fn is_agent(&self) -> bool {
        match self {
            Role::Agent(_) => true,
            _ => false,
        }
    }
}

fn parse_argv<T, U>(argv: T) -> Result<Vec<Command>>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    let argv: Vec<String> = argv.into_iter().map(|x| x.as_ref().to_string()).collect();
    let mut default_cmd = Command::from_env()?;
    let mut pos = 0;
    let mut cmds: Vec<Command> = Vec::new();

    while pos < argv.len() {
        let mut cmd = default_cmd.clone();
        pos += cmd.parse_argv(&argv[pos..]).map_err(Error::from)?;

        let mut sep_pos = argv.len();
        if let Some(sep) = &cmd.separator {
            let full_sep = format!("--{}", sep);
            if let Some(i) = argv[pos..].iter().position(|x| x == &full_sep) {
                sep_pos = pos + i;
            }
        }
        cmd.argv.extend_from_slice(&argv[pos..sep_pos]);
        pos = sep_pos + 1;

        if cmd.argv.is_empty() {
            default_cmd = cmd;
        } else {
            default_cmd.separator = cmd.separator.clone();
            cmds.push(cmd);
        }
    }

    Ok(cmds)
}

fn print_reports(cmds: &[Command], reports: &[Report]) -> std::io::Result<()> {
    let mut output_files: HashMap<&String, Vec<&Report>> = HashMap::new();
    for (i, cmd) in cmds.iter().enumerate() {
        if !cmd.hide_report && reports.len() == 1 {
            println!("{}", reports[i]);
        }
        if let Some(filename) = &cmd.output_file {
            output_files
                .entry(filename)
                .or_insert_with(Vec::new)
                .push(&reports[i]);
        }
    }

    for (filename, file_reports) in output_files.into_iter() {
        let _ = fs::remove_file(filename);
        let mut file = fs::File::create(filename)?;

        if file_reports.len() == 1 && !file_reports[0].kind.is_json() {
            write!(&mut file, "{}", file_reports[0])?;
        } else if file_reports.iter().all(|r| r.kind.is_json()) {
            let json_reports =
                JsonValue::Array(file_reports.into_iter().map(Report::to_json).collect());
            json_reports.write_pretty(&mut file, 4)?;
        }
    }

    Ok(())
}

fn check_cmds(cmds: &[Command], warnings: &Warnings) -> Result<()> {
    if cmds.iter().filter(|cmd| cmd.controller).count() > 1 {
        return Err(Error::from("There can be at most one controller"));
    }
    for cmd in cmds.iter() {
        assert!(!cmd.argv.is_empty());
        if cmd.delegated {
            warnings.emit("'-runas', '--delegated' options have no effect");
        }
        if cmd.shared_memory.is_some() {
            warnings.emit("'--shared-memory' option has no effect");
        }
    }
    Ok(())
}

fn check_protocol_entities(
    controller: &Controller,
    agents: &[Agent],
    graph: &Graph,
    warnings: &Warnings,
) {
    for agent in agents {
        if !graph.has_connection(controller.stdout(), agent.stdin()) {
            warnings.emit(format!(
                "Controller is not connected to agent#{} via stdout",
                agent.idx().0 + 1
            ))
        }
        if !graph.has_connection(agent.stdout(), controller.stdin()) {
            warnings.emit(format!(
                "Agent#{} is not connected to controller via stdout",
                agent.idx().0 + 1
            ))
        }
    }
}

fn init_entity_handler(entity: Role, graph: &mut Graph, controller: &Controller, agents: &[Agent]) {
    match entity {
        Role::Agent(idx) => {
            let agent = &agents[idx.0];
            if !graph.has_connection(agent.stdout(), controller.stdin()) {
                // Do not send any messages to controller if there's no connection.
                return;
            }
            graph
                .source_mut(agent.stdout())
                .unwrap()
                .set_reader(AgentStdout::new(agent.clone()));
        }
        Role::Controller => {
            graph
                .source_mut(controller.stdout())
                .unwrap()
                .set_reader(ControllerStdout::new(controller.clone(), agents.to_vec()));
        }
        _ => {}
    }
}

fn create_program(
    cmd: &Command,
    receiver: Receiver<ProgramMessage>,
    role: Role,
    warnings: &Warnings,
) -> Result<Program> {
    let mut info = create_process_info(cmd, role);
    let mut group = Group::new()?;
    init_os_specific_process_extensions(cmd, &mut info, &mut group, warnings).map(|_| {
        Program::new_with(info, |p| {
            p.group(group)
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
                .msg_receiver(receiver);
        })
    })
}

fn create_process_info(cmd: &Command, role: Role) -> ProcessInfo {
    let mut info = ProcessInfo::new(&cmd.argv[0]);
    info.args(cmd.argv[1..].iter())
        .suspended(role.is_agent())
        .search_in_path(cmd.use_syspath)
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

fn create_roles(cmds: &[Command]) -> Vec<Role> {
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
