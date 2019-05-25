use crate::cmd::{Command, Environment, RedirectKind, RedirectList};
use crate::misc::mb2b;
use crate::protocol::{
    AgentIdx, AgentStdout, AgentTermination, CommandIdx, Context, ControllerStdin,
    ControllerStdout, ControllerTermination,
};

use spawner::iograph::{IstreamDst, IstreamId, OstreamId, OstreamSrc};
use spawner::pipe::{self, ReadPipe};
use spawner::process::{GroupRestrictions, ProcessInfo, ResourceLimits};
use spawner::task::{Controllers, Spawner, StdioMapping, Tasks};
use spawner::{Error, Result};

use std::cell::RefCell;
use std::collections::HashSet;

pub struct Driver<'a> {
    tasks: Tasks,
    ctx: Context,
    cmds: &'a Vec<Command>,
    controller_stdin_r: ReadPipe,
    controller_stdin_w: ControllerStdin,
    warnings: RefCell<HashSet<String>>,
}

enum Role {
    Default,
    Agent(AgentIdx),
    Controller,
}

impl<'a> Driver<'a> {
    pub fn from_cmds(cmds: &'a Vec<Command>) -> Result<Self> {
        let (r, w) = pipe::create()?;
        let mut driver = Driver {
            tasks: Tasks::new(),
            cmds: cmds,
            ctx: Context::new(),
            controller_stdin_r: r,
            controller_stdin_w: ControllerStdin::new(w),
            warnings: RefCell::new(HashSet::new()),
        };
        driver.check_cmds()?;
        driver.add_tasks()?;
        driver.setup_stdio()?;
        Ok(driver)
    }

    pub fn spawn(mut self) -> Result<Spawner> {
        if let Some(controller_idx) = self.cmds.iter().position(|cmd| cmd.controller) {
            let stdin = self.tasks.stdio_mapping(controller_idx).stdin;
            self.tasks
                .io()
                .add_ostream_src(stdin, self.controller_stdin_r)?;
        }

        let stdio_mappings = self.tasks.stdio_mappings().collect();
        let spawner = Spawner::spawn(self.tasks)?;
        self.ctx.init(
            spawner.runners().collect(),
            spawner.io_graph().clone(),
            stdio_mappings,
        );
        Ok(spawner)
    }

    pub fn warnings(&self) -> Vec<String> {
        self.warnings.borrow().iter().cloned().collect()
    }

    fn emit_warning<T: ToString>(&self, msg: T) {
        self.warnings.borrow_mut().insert(msg.to_string());
    }

    fn check_cmds(&self) -> Result<()> {
        if self.cmds.iter().filter(|cmd| cmd.controller).count() > 1 {
            return Err(Error::from("There can be at most one controller"));
        }

        for cmd in self.cmds.iter() {
            assert!(cmd.argv.len() > 0);
            if cmd.load_ratio != 5.0 {
                self.emit_warning("'-lr' option has no effect")
            }
            if cmd.debug {
                self.emit_warning("'--debug' option has no effect");
            }
            if cmd.delegated {
                self.emit_warning("'-runas', '--delegated' options have no effect");
            }
            if cmd.use_syspath {
                self.emit_warning("'-c', '--systempath' options have no effect");
            }
            if cmd.shared_memory.is_some() {
                self.emit_warning("'--shared-memory' option has no effect");
            }
        }
        Ok(())
    }

    fn add_tasks(&mut self) -> Result<()> {
        let roles = create_roles(&self.cmds);
        let agent_indices = create_agent_indices(&roles);
        for (idx, role) in roles.into_iter().enumerate() {
            self.add_task(CommandIdx(idx), role, &agent_indices)?;
        }
        Ok(())
    }

    fn add_task(
        &mut self,
        cmd_idx: CommandIdx,
        role: Role,
        agent_indices: &Vec<CommandIdx>,
    ) -> Result<()> {
        let cmd = &self.cmds[cmd_idx.0];
        let ctls = match role {
            Role::Default => Controllers::new(),
            Role::Agent(agent_idx) => Controllers::new()
                .on_terminate(AgentTermination::new(
                    agent_idx,
                    self.controller_stdin_w.clone(),
                ))
                .stdout_controller(AgentStdout::new(self.ctx.clone(), agent_idx, cmd_idx)),
            Role::Controller => Controllers::new()
                .on_terminate(ControllerTermination::new(
                    self.ctx.clone(),
                    agent_indices.clone(),
                ))
                .stdout_controller(ControllerStdout::new(
                    self.ctx.clone(),
                    cmd_idx,
                    agent_indices.clone(),
                )),
        };
        let mut info = create_base_process_info(cmd, role);
        let mut restrictions = GroupRestrictions::new(ResourceLimits {
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

        self.tasks
            .add(info, restrictions, cmd.monitor_interval, ctls)
            .map(|_| ())
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
            self.emit_warning("'-sw' option works on windows only");
        }
        if cmd.env == Environment::UserDefault {
            self.emit_warning("'-env=user-default' option works on windows only, '-env=inherit' will be used instead");
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

    fn stdio_mapping(&self, i: usize) -> StdioMapping {
        self.tasks.stdio_mapping(i)
    }

    fn setup_stdio(&mut self) -> Result<()> {
        for (idx, cmd) in self.cmds.iter().enumerate() {
            let mapping = self.stdio_mapping(idx);
            self.redirect_ostream(mapping.stdin, &cmd.stdin_redirect)?;
            self.redirect_istream(mapping.stdout, &cmd.stdout_redirect)?;
            self.redirect_istream(mapping.stderr, &cmd.stderr_redirect)?;
        }
        Ok(())
    }

    fn redirect_istream(&mut self, istream: IstreamId, redirect_list: &RedirectList) -> Result<()> {
        for redirect in redirect_list.items.iter() {
            let dst = match &redirect.kind {
                RedirectKind::File(f) => {
                    Some(IstreamDst::File(f.into(), redirect.flags.file_lock()))
                }
                RedirectKind::Stdin(i) => {
                    self.check_stdio_idx("Stdin", *i)?;
                    Some(self.stdio_mapping(*i).stdin.into())
                }
                RedirectKind::Stderr(i) => {
                    self.check_stdio_idx("Stderr", *i)?;
                    Some(self.stdio_mapping(*i).stdin.into())
                }
                _ => None,
            };
            if let Some(dst) = dst {
                self.tasks.io().add_istream_dst(istream, dst)?;
            }
        }
        Ok(())
    }

    fn redirect_ostream(&mut self, ostream: OstreamId, redirect_list: &RedirectList) -> Result<()> {
        for redirect in redirect_list.items.iter() {
            let src = match &redirect.kind {
                RedirectKind::File(f) => {
                    Some(OstreamSrc::File(f.into(), redirect.flags.file_lock()))
                }
                RedirectKind::Stdout(i) => {
                    self.check_stdio_idx("Stdout", *i)?;
                    Some(self.stdio_mapping(*i).stdout.into())
                }
                _ => None,
            };
            if let Some(src) = src {
                self.tasks.io().add_ostream_src(ostream, src)?;
            }
        }
        Ok(())
    }

    fn check_stdio_idx(&self, stream: &str, idx: usize) -> Result<()> {
        if idx >= self.cmds.len() {
            Err(Error::from(format!(
                "{} index '{}' is out of range",
                stream, idx
            )))
        } else {
            Ok(())
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
