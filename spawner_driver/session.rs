use crate::misc::mb2b;
use crate::opts::{Options, PipeKind, StdioRedirectKind, StdioRedirectList};
use crate::protocol::{
    AgentIdx, AgentStdout, AgentTermination, CommandIdx, Context, ControllerStdin,
    ControllerStdout, ControllerTermination,
};

use spawner::command::{Command, CommandController, Limits};
use spawner::iograph::{IstreamId, OstreamId};
use spawner::pipe::{self, ReadPipe};
use spawner::session::{IstreamDst, OstreamSrc, Session, SessionBuilder, StdioMapping};
use spawner::{Error, Result};

use std::cell::RefCell;

pub struct SessionBuilderEx<'a> {
    base: RefCell<SessionBuilder>,
    ctx: Context,
    cmds: &'a Vec<Options>,
    mappings: Vec<StdioMapping>,
    controller_stdin_r: ReadPipe,
    controller_stdin_w: ControllerStdin,
}

enum Role {
    Default,
    Agent(AgentIdx),
    Controller,
}

impl<'a> SessionBuilderEx<'a> {
    pub fn from_cmds(cmds: &'a Vec<Options>) -> Result<Self> {
        let (r, w) = pipe::create()?;
        let mut builder = SessionBuilderEx {
            base: RefCell::new(SessionBuilder::new()),
            cmds: cmds,
            mappings: Vec::new(),
            ctx: Context::new(),
            controller_stdin_r: r,
            controller_stdin_w: ControllerStdin::new(w),
        };

        builder.setup_stdio()?;
        Ok(builder)
    }

    pub fn spawn(mut self) -> Result<Session> {
        if let Some(controller_idx) = self.cmds.iter().position(|cmd| cmd.controller) {
            let stdin = self.mappings[controller_idx].stdin;
            self.base
                .borrow_mut()
                .add_ostream_src(stdin, OstreamSrc::pipe(self.controller_stdin_r))?;
        }

        let sess = self.base.into_inner().spawn()?;
        self.ctx.init(
            sess.controllers()
                .map(|c| c.runner_controller().clone())
                .collect(),
            sess.io_graph().clone(),
            self.mappings,
        );
        Ok(sess)
    }

    fn setup_stdio(&mut self) -> Result<()> {
        let roles = create_roles(self.cmds);
        let ctls = create_cmd_controllers(&roles, &self.ctx, &self.controller_stdin_w);

        for ((cmd, ctl), role) in self
            .cmds
            .iter()
            .zip(ctls.into_iter())
            .zip(roles.into_iter())
        {
            let mapping = self.base.borrow_mut().add_task(
                Command {
                    app: cmd.argv[0].clone(),
                    args: cmd.argv.iter().skip(1).map(|s| s.to_string()).collect(),
                    env_vars: cmd.env_vars.clone(),
                    env_kind: cmd.env,
                    monitor_interval: cmd.monitor_interval,
                    show_window: cmd.show_window,
                    create_suspended: role.is_agent(),
                    limits: Limits {
                        max_wall_clock_time: cmd.wall_clock_time_limit,
                        max_idle_time: cmd.idle_time_limit,
                        max_user_time: cmd.time_limit,
                        max_memory_usage: cmd.memory_limit.map(|v| mb2b(v)),
                        max_output_size: cmd.write_limit.map(|v| mb2b(v)),
                        max_processes: cmd.process_count,
                    },
                    working_directory: cmd.working_directory.clone(),
                    username: cmd.username.clone(),
                    password: cmd.password.clone(),
                },
                ctl,
            )?;
            self.mappings.push(mapping);
        }

        for (cmd, mapping) in self.cmds.iter().zip(self.mappings.iter()) {
            self.redirect_ostream(mapping.stdin, &cmd.stdin_redirect)?;
            self.redirect_istream(mapping.stdout, &cmd.stdout_redirect)?;
            self.redirect_istream(mapping.stderr, &cmd.stderr_redirect)?;
        }

        Ok(())
    }

    fn redirect_istream(
        &self,
        istream: IstreamId,
        redirect_list: &StdioRedirectList,
    ) -> Result<()> {
        for redirect in redirect_list.items.iter() {
            let dst = match &redirect.kind {
                StdioRedirectKind::File(f) => {
                    Some(IstreamDst::file(f, redirect.flags.share_mode()))
                }
                StdioRedirectKind::Pipe(pipe_kind) => match pipe_kind {
                    PipeKind::Stdin(i) => {
                        self.check_stdio_idx("Stdin", *i)?;
                        Some(IstreamDst::ostream(self.mappings[*i].stdin))
                    }
                    PipeKind::Stderr(i) => {
                        self.check_stdio_idx("Stderr", *i)?;
                        Some(IstreamDst::ostream(self.mappings[*i].stdin))
                    }
                    _ => None,
                },
            };
            if let Some(dst) = dst {
                self.base.borrow_mut().add_istream_dst(istream, dst)?;
            }
        }
        Ok(())
    }

    fn redirect_ostream(
        &self,
        ostream: OstreamId,
        redirect_list: &StdioRedirectList,
    ) -> Result<()> {
        for redirect in redirect_list.items.iter() {
            let src = match &redirect.kind {
                StdioRedirectKind::File(f) => {
                    Some(OstreamSrc::file(f, redirect.flags.share_mode()))
                }
                StdioRedirectKind::Pipe(pipe_kind) => match pipe_kind {
                    PipeKind::Stdout(i) => {
                        self.check_stdio_idx("Stdout", *i)?;
                        Some(OstreamSrc::istream(self.mappings[*i].stdout))
                    }
                    _ => None,
                },
            };
            if let Some(src) = src {
                self.base.borrow_mut().add_ostream_src(ostream, src)?;
            }
        }
        Ok(())
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

impl Role {
    fn is_agent(&self) -> bool {
        match self {
            Role::Agent(_) => true,
            _ => false,
        }
    }
}

fn create_cmd_controllers(
    roles: &Vec<Role>,
    ctx: &Context,
    controller_stdin: &ControllerStdin,
) -> Vec<CommandController> {
    roles
        .iter()
        .enumerate()
        .map(|(idx, role)| match role {
            Role::Default => CommandController {
                on_terminate: None,
                stdout_controller: None,
            },
            Role::Agent(agent_idx) => CommandController {
                on_terminate: Some(Box::new(AgentTermination::new(
                    *agent_idx,
                    controller_stdin.clone(),
                ))),
                stdout_controller: Some(Box::new(AgentStdout::new(
                    ctx.clone(),
                    *agent_idx,
                    CommandIdx(idx),
                ))),
            },
            Role::Controller => {
                let agent_indices = create_agent_indices(&roles);
                CommandController {
                    on_terminate: Some(Box::new(ControllerTermination::new(
                        ctx.clone(),
                        agent_indices.clone(),
                    ))),
                    stdout_controller: Some(Box::new(ControllerStdout::new(
                        ctx.clone(),
                        CommandIdx(idx),
                        agent_indices,
                    ))),
                }
            }
        })
        .collect()
}

fn create_roles(cmds: &[Options]) -> Vec<Role> {
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
