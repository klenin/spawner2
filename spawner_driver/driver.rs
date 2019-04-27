use crate::cmd::{Command, RedirectKind, RedirectList};
use crate::misc::mb2b;
use crate::protocol::{
    AgentIdx, AgentStdout, AgentTermination, CommandIdx, Context, ControllerStdin,
    ControllerStdout, ControllerTermination,
};

use spawner::iograph::{IstreamDst, IstreamId, OstreamId, OstreamSrc};
use spawner::pipe::{self, ReadPipe};
use spawner::process::{ProcessInfo, ResourceLimits};
use spawner::task::{Spawner, StdioMapping, Task, Tasks};
use spawner::{Error, Result};

pub struct Driver<'a> {
    tasks: Tasks,
    ctx: Context,
    cmds: &'a Vec<Command>,
    controller_stdin_r: ReadPipe,
    controller_stdin_w: ControllerStdin,
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
        };

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
            spawner
                .controllers()
                .map(|ctl| ctl.runner().clone())
                .collect(),
            spawner.io_graph().clone(),
            stdio_mappings,
        );
        Ok(spawner)
    }

    fn stdio_mapping(&self, i: usize) -> StdioMapping {
        self.tasks.stdio_mapping(i)
    }

    fn setup_stdio(&mut self) -> Result<()> {
        let tasks = create_tasks(&self.ctx, &self.cmds, &self.controller_stdin_w);
        self.tasks.extend(tasks.into_iter())?;

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

fn create_tasks(
    ctx: &Context,
    cmds: &Vec<Command>,
    controller_stdin: &ControllerStdin,
) -> Vec<Task> {
    let roles = create_roles(cmds);
    let agent_indices = create_agent_indices(&roles);

    cmds.iter()
        .zip(roles.iter())
        .enumerate()
        .map(|(idx, (cmd, role))| Task {
            process_info: ProcessInfo {
                app: cmd.argv[0].clone(),
                args: cmd.argv[1..].iter().cloned().collect(),
                env_vars: cmd.env_vars.clone(),
                env: cmd.env,
                show_window: cmd.show_window,
                working_directory: cmd.working_directory.clone(),
                username: cmd.username.clone(),
                password: cmd.password.clone(),
                resource_limits: ResourceLimits {
                    max_wall_clock_time: cmd.wall_clock_time_limit,
                    max_idle_time: cmd.idle_time_limit,
                    max_user_time: cmd.time_limit,
                    max_memory_usage: cmd.memory_limit.map(|v| mb2b(v)),
                    max_output_size: cmd.write_limit.map(|v| mb2b(v)),
                    max_processes: cmd.process_count,
                },
            },

            monitor_interval: cmd.monitor_interval,
            resume_process: !role.is_agent(),
            on_terminate: match role {
                Role::Default => None,
                Role::Agent(agent_idx) => {
                    AgentTermination::new(*agent_idx, controller_stdin.clone()).into()
                }
                Role::Controller => {
                    ControllerTermination::new(ctx.clone(), agent_indices.clone()).into()
                }
            },
            stdout_controller: match role {
                Role::Default => None,
                Role::Agent(agent_idx) => {
                    AgentStdout::new(ctx.clone(), *agent_idx, CommandIdx(idx)).into()
                }
                Role::Controller => {
                    ControllerStdout::new(ctx.clone(), CommandIdx(idx), agent_indices.clone())
                        .into()
                }
            },
        })
        .collect()
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
