use crate::cmd::{Command, Environment, RedirectFlags, RedirectKind, RedirectList};
use crate::misc::mb2b;
use crate::protocol::{
    AgentIdx, AgentStdout, AgentTermination, CommandIdx, Context, ControllerStdin,
    ControllerStdout, ControllerTermination,
};

use spawner::io::{
    IoBuilder, IoStreams, IstreamDst, IstreamId, OstreamId, OstreamSrc, StdioMapping,
};
use spawner::pipe::{self, ReadPipe, WritePipe};
use spawner::process::{GroupRestrictions, ProcessInfo, ResourceLimits};
use spawner::{self, Actions, Error, Result, Router, Spawner, SpawnerResult};

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};

pub struct Warnings(RefCell<HashSet<String>>);

pub struct Driver<'a> {
    cmds: &'a Vec<Command>,
    io_streams: IoStreams,
    ctx: Context,
    roles: Vec<Role>,
    agent_indices: Vec<CommandIdx>,
    mappings: Vec<StdioMapping>,
    controller_stdin: ControllerStdin,
    builders: Vec<spawner::Builder>,
    warnings: Warnings,
}

#[derive(Copy, Clone)]
enum Role {
    Default,
    Agent(AgentIdx),
    Controller,
}

struct DriverIo {
    controller_idx: Option<usize>,
    builder: IoBuilder,
    mappings: Vec<StdioMapping>,
    output_files: HashMap<PathBuf, OstreamId>,
    warnings: Warnings,

    #[allow(dead_code)]
    exclusive_input_files: HashMap<PathBuf, IstreamId>,
}

impl Warnings {
    pub fn new() -> Self {
        Self(RefCell::new(HashSet::new()))
    }

    pub fn emit<T: ToString>(&self, msg: T) {
        self.0.borrow_mut().insert(msg.to_string());
    }

    pub fn to_vec(&self) -> Vec<String> {
        self.0.borrow().iter().cloned().collect()
    }

    pub fn take(&mut self) -> Warnings {
        mem::replace(self, Warnings::new())
    }
}

impl<'a> Driver<'a> {
    pub fn from_cmds(cmds: &'a Vec<Command>) -> Result<Self> {
        let roles = create_roles(cmds);
        let agent_indices = create_agent_indices(&roles);
        let mut driver_io = DriverIo::from_cmds(cmds)?;
        let warnings = driver_io.warnings.take();
        let (io_streams, mappings, controller_stdin) = driver_io.build()?;

        let mut driver = Driver {
            cmds: cmds,
            io_streams: io_streams,
            ctx: Context::new(),
            roles: roles,
            agent_indices: agent_indices,
            mappings: mappings,
            controller_stdin: controller_stdin,
            builders: Vec::new(),
            warnings: warnings,
        };
        driver.check_cmds()?;
        for i in 0..cmds.len() {
            driver.init_builder(CommandIdx(i));
        }
        Ok(driver)
    }

    pub fn run(mut self) -> Result<Vec<SpawnerResult>> {
        let mut spawners = Vec::new();
        for (sp_builder, mapping) in self.builders.into_iter().zip(self.mappings.iter()) {
            match sp_builder.build(&mut self.io_streams, *mapping) {
                Ok(sp) => spawners.push(sp),
                Err(e) => {
                    for sp in spawners.iter() {
                        sp.runner().terminate();
                    }
                    return Err(e);
                }
            }
        }

        // Drop our reference to controller's stdin. Otherwise we'll hang on `router.wait()`.
        drop(self.controller_stdin);

        // Handle remaining i\o streams.
        let router = Router::from_iostreams(&mut self.io_streams);

        self.ctx.init(
            spawners.iter().map(Spawner::runner).collect(),
            self.io_streams.graph().clone(),
            self.mappings,
        );

        let reports = spawners.into_iter().map(Spawner::wait).collect();
        router.wait();
        Ok(reports)
    }

    pub fn warnings(&self) -> &Warnings {
        &self.warnings
    }

    fn check_cmds(&self) -> Result<()> {
        if self.cmds.iter().filter(|cmd| cmd.controller).count() > 1 {
            return Err(Error::from("There can be at most one controller"));
        }

        for cmd in self.cmds.iter() {
            assert!(cmd.argv.len() > 0);
            if cmd.load_ratio != 5.0 {
                self.warnings.emit("'-lr' option has no effect")
            }
            if cmd.debug {
                self.warnings.emit("'--debug' option has no effect");
            }
            if cmd.delegated {
                self.warnings
                    .emit("'-runas', '--delegated' options have no effect");
            }
            if cmd.use_syspath {
                self.warnings
                    .emit("'-c', '--systempath' options have no effect");
            }
            if cmd.shared_memory.is_some() {
                self.warnings.emit("'--shared-memory' option has no effect");
            }
        }
        Ok(())
    }

    fn init_builder(&mut self, cmd_idx: CommandIdx) {
        let cmd = &self.cmds[cmd_idx.0];
        let role = self.roles[cmd_idx.0];
        let mut info = create_base_process_info(cmd, role);
        let mut restrictions = GroupRestrictions::with_limits(ResourceLimits {
            wall_clock_time: cmd.wall_clock_time_limit,
            total_idle_time: cmd.idle_time_limit,
            total_user_time: cmd.time_limit,
            peak_memory_used: cmd.memory_limit.map(mb2b),
            total_bytes_written: cmd.write_limit.map(mb2b),
            total_processes_created: cmd.process_count,
            active_processes: cmd.active_process_count,
            active_network_connections: cmd.active_connection_count,
        });
        self.init_extensions(cmd, &mut info, &mut restrictions);

        let mut builder = spawner::Builder::new(info);
        builder
            .group_restrictions(restrictions)
            .monitor_interval(cmd.monitor_interval)
            .actions(match role {
                Role::Default => Actions::new(),
                Role::Agent(agent_idx) => Actions::new()
                    .on_terminate(AgentTermination::new(
                        agent_idx,
                        self.controller_stdin.clone(),
                    ))
                    .on_stdout_read(AgentStdout::new(self.ctx.clone(), agent_idx, cmd_idx)),
                Role::Controller => Actions::new()
                    .on_terminate(ControllerTermination::new(
                        self.ctx.clone(),
                        self.agent_indices.clone(),
                    ))
                    .on_stdout_read(ControllerStdout::new(
                        self.ctx.clone(),
                        cmd_idx,
                        self.agent_indices.clone(),
                    )),
            });
        self.builders.push(builder);
    }

    #[cfg(windows)]
    fn init_extensions(
        &self,
        cmd: &Command,
        info: &mut ProcessInfo,
        restrictions: &mut GroupRestrictions,
    ) {
        use spawner::windows::process::{GroupRestrictionsExt, ProcessInfoExt, UiRestrictions};
        if cmd.show_window {
            info.show_window(true);
        }
        if cmd.env == Environment::UserDefault {
            info.env_user();
        }
        if cmd.secure {
            restrictions.ui_restrictions(
                UiRestrictions::new()
                    .limit_desktop()
                    .limit_display_settings()
                    .limit_exit_windows()
                    .limit_global_atoms()
                    .limit_handles()
                    .limit_read_clipboard()
                    .limit_write_clipboard()
                    .limit_system_parameters(),
            );
        }
    }

    #[cfg(unix)]
    fn init_extensions(
        &self,
        cmd: &Command,
        info: &mut ProcessInfo,
        _restrictions: &mut GroupRestrictions,
    ) {
        use spawner::unix::process::{ProcessInfoExt, SyscallFilterBuilder};
        if cmd.show_window {
            self.warnings.emit("'-sw' option works on windows only");
        }
        if cmd.env == Environment::UserDefault {
            self.warnings.emit("'-env=user-default' option works on windows only, '-env=inherit' will be used instead");
            info.env_inherit();
        }

        // Syscall numbers to allow execve.

        #[cfg(target_arch = "x86")]
        let syscall_table = [
            173, // rt_sigreturn
            252, // exit_group
            1,   // exit
            3,   // read
            4,   // write
            175, // rt_sigprocmask
            174, // rt_sigaction
            162, // nanosleep
            45,  // brk
            11,  // execve
            6,   // close
            5,   // open
            33,  // access
            108, // fstat
            90,  // mmap
            91,  // munmap
            125, // mprotect
        ];

        #[cfg(target_arch = "x86_64")]
        let syscall_table = [
            15,  // rt_sigreturn
            231, // exit_group
            60,  // exit
            0,   // read
            1,   // write
            14,  // rt_sigprocmask
            13,  // rt_sigaction
            35,  // nanosleep
            12,  // brk
            59,  // execve
            3,   // close
            2,   // open
            21,  // access
            5,   // fstat
            9,   // mmap
            158, // arch_prctl
            11,  // munmap
            10,  // mprotect
        ];

        if cmd.secure {
            let mut builder = SyscallFilterBuilder::block_all();
            for syscall in syscall_table.iter() {
                builder.allow(*syscall);
            }
            info.syscall_filter(builder.build());
        }
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

impl DriverIo {
    fn from_cmds(cmds: &Vec<Command>) -> Result<Self> {
        let mut builder = IoBuilder::new();
        let mappings = cmds
            .iter()
            .map(|_| builder.add_stdio())
            .collect::<Result<_>>()?;
        let mut driver_io = Self {
            builder: builder,
            mappings: mappings,
            controller_idx: cmds.iter().position(|cmd| cmd.controller),
            output_files: HashMap::new(),
            exclusive_input_files: HashMap::new(),
            warnings: Warnings::new(),
        };
        for (idx, cmd) in cmds.iter().enumerate() {
            let mapping = driver_io.mappings[idx];
            driver_io.redirect_ostream(mapping.stdin, &cmd.stdin_redirect)?;
            driver_io.redirect_istream(mapping.stdout, &cmd.stdout_redirect)?;
            driver_io.redirect_istream(mapping.stderr, &cmd.stderr_redirect)?;
        }
        Ok(driver_io)
    }

    fn build(mut self) -> Result<(IoStreams, Vec<StdioMapping>, ControllerStdin)> {
        let (r, w) = pipe::create()?;
        if let Some(idx) = self.controller_idx {
            let stdin = self.mappings[idx].stdin;
            self.builder.add_ostream_src(stdin, OstreamSrc::Pipe(r))?;
        }
        Ok((self.builder.build(), self.mappings, ControllerStdin::new(w)))
    }

    fn redirect_istream(&mut self, istream: IstreamId, redirect_list: &RedirectList) -> Result<()> {
        for redirect in redirect_list.items.iter() {
            let id = match &redirect.kind {
                RedirectKind::File(path) => self.open_output_file(path, redirect.flags)?,
                RedirectKind::Stdin(i) => {
                    self.check_stdio_idx("Stdin", *i)?;
                    self.mappings[*i].stdin
                }
                RedirectKind::Stderr(i) => {
                    self.check_stdio_idx("Stderr", *i)?;
                    self.mappings[*i].stdin
                }
                _ => continue,
            };
            self.builder
                .add_istream_dst(istream, IstreamDst::Ostream(id))?;
        }
        Ok(())
    }

    fn redirect_ostream(&mut self, ostream: OstreamId, redirect_list: &RedirectList) -> Result<()> {
        for redirect in redirect_list.items.iter() {
            let id = match &redirect.kind {
                RedirectKind::File(path) => self.open_input_file(path, redirect.flags)?,
                RedirectKind::Stdout(i) => {
                    self.check_stdio_idx("Stdout", *i)?;
                    self.mappings[*i].stdout
                }
                _ => continue,
            };
            self.builder
                .add_ostream_src(ostream, OstreamSrc::Istream(id))?;
        }
        Ok(())
    }

    #[cfg(windows)]
    fn open_input_file(&mut self, path: &String, flags: RedirectFlags) -> Result<IstreamId> {
        use spawner::windows::pipe::ReadPipeExt;

        let path = canonicalize(path)?;
        if flags.exclusive {
            match self.exclusive_input_files.get(&path).map(|&id| id) {
                Some(id) => Ok(id),
                None => {
                    let id = self.builder.add_file_istream(ReadPipe::lock(&path)?)?;
                    self.exclusive_input_files.insert(path, id);
                    Ok(id)
                }
            }
        } else {
            self.builder.add_file_istream(ReadPipe::open(path)?)
        }
    }

    #[cfg(unix)]
    fn open_input_file(&mut self, path: &String, flags: RedirectFlags) -> Result<IstreamId> {
        if flags.exclusive {
            self.warnings
                .emit("Exclusive redirect works on windows only");
        }
        let path = canonicalize(path)?;
        self.builder.add_file_istream(ReadPipe::open(path)?)
    }

    #[cfg(windows)]
    fn open_output_file(&mut self, path: &String, flags: RedirectFlags) -> Result<OstreamId> {
        use spawner::windows::pipe::WritePipeExt;

        let path = canonicalize(path)?;
        match self.output_files.get(&path).map(|&id| id) {
            Some(id) => Ok(id),
            None => {
                let pipe = if flags.exclusive {
                    WritePipe::lock(&path)?
                } else {
                    WritePipe::open(&path)?
                };
                let id = self.builder.add_file_ostream(pipe)?;
                self.output_files.insert(path, id);
                Ok(id)
            }
        }
    }

    #[cfg(unix)]
    fn open_output_file(&mut self, path: &String, flags: RedirectFlags) -> Result<OstreamId> {
        if flags.exclusive {
            self.warnings
                .emit("Exclusive redirect works on windows only");
        }
        let path = canonicalize(path)?;
        match self.output_files.get(&path).map(|&id| id) {
            Some(id) => Ok(id),
            None => {
                let id = self.builder.add_file_ostream(WritePipe::open(&path)?)?;
                self.output_files.insert(path, id);
                Ok(id)
            }
        }
    }

    fn check_stdio_idx(&self, stream: &str, idx: usize) -> Result<()> {
        if idx >= self.mappings.len() {
            Err(Error::from(format!(
                "{} index '{}' is out of range",
                stream, idx
            )))
        } else {
            Ok(())
        }
    }
}

fn canonicalize(path: &String) -> Result<PathBuf> {
    if !Path::exists(path.as_ref()) {
        fs::File::create(path).map_err(|_| Error::from(format!("Unable to create '{}'", path)))?;
    }
    fs::canonicalize(path).map_err(|_| Error::from(format!("Unable to open '{}'", path)))
}

fn create_base_process_info(cmd: &Command, role: Role) -> ProcessInfo {
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

fn create_agent_indices(roles: &Vec<Role>) -> Vec<CommandIdx> {
    roles
        .iter()
        .enumerate()
        .filter_map(|(idx, role)| match role {
            Role::Agent(_) => Some(CommandIdx(idx)),
            _ => None,
        })
        .collect()
}
