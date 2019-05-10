use crate::process::{
    Environment, ExitStatus, LimitViolation, ProcessInfo, ResourceLimits, ResourceUsage,
};
use crate::sys::limit_checker::LimitChecker;
use crate::sys::unix::pipe::{PipeFd, ReadPipe, WritePipe};
use crate::sys::IntoInner;
use crate::{Error, Result};

use nix::libc::{getpwnam, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{
    chdir, close, dup2, execve, fork, getpid, getuid, setgroups, setresgid, setresuid, ForkResult,
    Gid, Pid, Uid,
};

use cgroups_fs::{Cgroup, CgroupName};

use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};

use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

pub struct ProcessStdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

pub struct Process {
    pid: Pid,

    exit_status: Option<ExitStatus>,
}

pub struct Group {
    memory: Cgroup,
    cpuacct: Cgroup,
    pids: Cgroup,
    freezer: Cgroup,
    limit_checker: LimitChecker,
    creation_time: Instant,

    // Since we have information only about active tasks we need to memorize amount
    // of dead tasks and amount of bytes written by them.
    active_tasks: HashMap<Pid, u64>,
    dead_tasks_wchar: u64,
    num_dead_tasks: usize,
}

struct RawStdio {
    stdin: PipeFd,
    stdout: PipeFd,
    stderr: PipeFd,
}

struct User {
    uid: Uid,
    gid: Gid,
}

impl Process {
    pub fn exit_status(&mut self) -> Result<Option<ExitStatus>> {
        if self.exit_status.is_none() {
            self.exit_status = match waitpid(self.pid, Some(WaitPidFlag::WNOHANG))? {
                WaitStatus::Exited(pid, code) => {
                    assert_eq!(pid, self.pid);
                    Some(ExitStatus::Finished(code as u32))
                }
                WaitStatus::Signaled(pid, signal, _) => {
                    assert_eq!(pid, self.pid);
                    Some(ExitStatus::Crashed(format!(
                        "Process terminated by the '{}' signal",
                        signal
                    )))
                }
                _ => None,
            };
        }

        Ok(self.exit_status.clone())
    }

    pub fn suspend(&self) -> Result<()> {
        kill(self.pid, Signal::SIGSTOP).map_err(Error::from)
    }

    pub fn resume(&self) -> Result<()> {
        kill(self.pid, Signal::SIGCONT).map_err(Error::from)
    }

    pub fn terminate(&self) -> Result<()> {
        kill(self.pid, Signal::SIGKILL).map_err(Error::from)
    }

    fn suspended<T, U>(info: T, stdio: U) -> Result<Self>
    where
        T: AsRef<ProcessInfo>,
        U: Into<ProcessStdio>,
    {
        let info = info.as_ref();
        let stdio = stdio.into();
        let usr = info.username.as_ref().map(User::new).transpose()?;

        if let ForkResult::Parent { child, .. } = fork()? {
            // Wait for initialization.
            waitpid(child, Some(WaitPidFlag::WSTOPPED))?;
            return Ok(Process {
                pid: child,
                exit_status: None,
            });
        }

        let mut env: HashMap<String, String> = match info.env {
            Environment::Clear => HashMap::new(),
            Environment::Inherit | Environment::UserDefault => std::env::vars().collect(),
        };
        env.extend(info.env_vars.iter().cloned());

        let exit_code = if let Err(_) = init_child_process(
            info.app.clone(),
            info.args.iter().map(String::clone),
            env.iter().map(|(k, v)| format!("{}={}", k, v)),
            RawStdio {
                stdin: stdio.stdin.into_inner(),
                stdout: stdio.stdout.into_inner(),
                stderr: stdio.stderr.into_inner(),
            },
            info.working_directory.as_ref(),
            usr,
        ) {
            // todo: Send error to parent process.
            111
        } else {
            0
        };
        process::exit(exit_code);
    }
}

impl Group {
    pub fn new() -> Result<Self> {
        Ok(Self {
            memory: create_cgroup("memory/sp")?,
            cpuacct: create_cgroup("cpuacct/sp")?,
            pids: create_cgroup("pids/sp")?,
            freezer: create_cgroup("freezer/sp")?,
            limit_checker: LimitChecker::new(),
            creation_time: Instant::now(),
            active_tasks: HashMap::new(),
            dead_tasks_wchar: 0,
            num_dead_tasks: 0,
        })
    }

    pub fn set_limits<T: Into<ResourceLimits>>(&mut self, limits: T) -> Result<()> {
        self.limit_checker.set_limits(limits.into());
        Ok(())
    }

    pub fn spawn<T, U>(&mut self, info: T, stdio: U) -> Result<Process>
    where
        T: AsRef<ProcessInfo>,
        U: Into<ProcessStdio>,
    {
        let info = info.as_ref();
        let ps = Process::suspended(info, stdio)?;
        self.memory
            .add_task(ps.pid)
            .and(self.cpuacct.add_task(ps.pid))
            .and(self.pids.add_task(ps.pid))
            .and(self.freezer.add_task(ps.pid))
            .map_err(Error::from)
            .and_then(|_| if info.suspended { Ok(()) } else { ps.resume() })
            .map_err(|e| {
                let _ = ps.terminate();
                e
            })
            .map(|_| ps)
    }

    pub fn resource_usage(&mut self) -> Result<ResourceUsage> {
        let total_user_time = self.cpuacct.get_value::<u64>("cpuacct.usage_user")?;
        let total_sys_time = self.cpuacct.get_value::<u64>("cpuacct.usage_sys")?;

        let max_mem_usage = self.memory.get_value::<u64>("memory.max_usage_in_bytes")?;
        let max_kmem_usage = self
            .memory
            .get_value::<u64>("memory.kmem.max_usage_in_bytes")?;

        self.update_active_tasks()?;

        Ok(ResourceUsage {
            wall_clock_time: self.creation_time.elapsed(),
            total_user_time: Duration::from_nanos(total_user_time),
            total_kernel_time: Duration::from_nanos(total_sys_time),
            peak_memory_used: max_mem_usage + max_kmem_usage,
            total_bytes_written: self.active_tasks.values().cloned().sum::<u64>()
                + self.dead_tasks_wchar,
            total_processes_created: self.num_dead_tasks + self.active_tasks.len(),
            active_processes: self.active_tasks.len(),
        })
    }

    pub fn check_limits(&mut self) -> Result<Option<LimitViolation>> {
        self.resource_usage()
            .map(|usage| self.limit_checker.check(usage))
    }

    pub fn reset_time_usage(&mut self) -> Result<()> {
        let zero = self.resource_usage()?;
        self.limit_checker
            .reset_timers(zero.wall_clock_time, zero.total_user_time);
        Ok(())
    }

    pub fn terminate(&self) -> Result<()> {
        self.freezer.set_raw_value("freezer.state", "FROZEN")?;
        while self.freezer.get_raw_value("freezer.state")? == "FREEZING" {
            thread::sleep(Duration::from_millis(1));
        }
        self.freezer.send_signal_to_all_tasks(Signal::SIGKILL)?;
        self.freezer.set_raw_value("freezer.state", "THAWED")?;
        Ok(())
    }

    fn update_active_tasks(&mut self) -> Result<()> {
        let new_active_tasks = self.collect_active_tasks()?;
        let old_active_tasks = &mut self.active_tasks;
        let mut dead_tasks = Vec::new();

        for (pid, bytes_written) in old_active_tasks.iter_mut() {
            match new_active_tasks.get(pid) {
                None => dead_tasks.push(pid.clone()),
                Some(new_bytes_written) => {
                    if let Some(bytes) = new_bytes_written {
                        *bytes_written = *bytes;
                    }
                }
            }
        }

        for (pid, bytes_written) in new_active_tasks.into_iter() {
            if old_active_tasks.get(&pid).is_none() {
                old_active_tasks.insert(pid, bytes_written.unwrap_or(0));
            }
        }

        self.num_dead_tasks += dead_tasks.len();
        for pid in dead_tasks {
            self.dead_tasks_wchar += old_active_tasks.remove(&pid).unwrap();
        }

        Ok(())
    }

    fn collect_active_tasks(&self) -> Result<HashMap<Pid, Option<u64>>> {
        self.freezer.get_tasks().map_err(Error::from).map(|tasks| {
            tasks
                .into_iter()
                .map(|pid| (pid, task_wchar(pid)))
                .collect()
        })
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.freezer.remove().ok();
        self.memory.remove().ok();
        self.cpuacct.remove().ok();
        self.pids.remove().ok();
    }
}

impl User {
    fn new(login: &String) -> Result<Self> {
        // todo: Check password?
        let pwd = unsafe { getpwnam(to_cstr(login.as_str())?.as_ptr()) };
        if pwd.is_null() {
            Err(Error::from(format!("Incorrect username '{}'", login)))
        } else {
            Ok(Self {
                uid: Uid::from_raw(unsafe { (*pwd).pw_uid }),
                gid: Gid::from_raw(unsafe { (*pwd).pw_gid }),
            })
        }
    }

    fn impersonate(&self) -> Result<()> {
        // Drop groups if we have root priveleges. Otherwise setgroups call will fail
        // with `Operation not permitted` error.
        if getuid().is_root() {
            setgroups(&[self.gid])?;
        }
        setresgid(self.gid, self.gid, self.gid)?;
        setresuid(self.uid, self.uid, self.uid)?;
        Ok(())
    }
}

fn create_cgroup(subsystem: &'static str) -> Result<Cgroup> {
    let mut rng = thread_rng();
    let name = format!(
        "task_{}",
        (0..7).map(|_| rng.sample(Alphanumeric)).collect::<String>()
    );
    let cgroup = Cgroup::new(&CgroupName::new(&name), subsystem);
    cgroup.create().map_err(|e| {
        Error::from(format!(
            "Cannot create cgroup /{}/{}: {}",
            subsystem, name, e
        ))
    })?;
    Ok(cgroup)
}

fn to_cstr<S: Into<Vec<u8>>>(s: S) -> Result<CString> {
    CString::new(s).map_err(|e| Error::from(e.to_string()))
}

fn close_stdio() -> Result<()> {
    close(STDIN_FILENO)?;
    close(STDOUT_FILENO)?;
    close(STDERR_FILENO)?;
    Ok(())
}

fn init_child_process<S, P, A, E>(
    path: S,
    args: A,
    env: E,
    stdio: RawStdio,
    working_dir: Option<P>,
    usr: Option<User>,
) -> Result<()>
where
    S: Into<Vec<u8>>,
    P: AsRef<Path>,
    A: IntoIterator<Item = S>,
    E: IntoIterator<Item = S>,
{
    if let Some(usr) = usr {
        usr.impersonate()?;
    }

    close_stdio()?;
    dup2(stdio.stdin.0, STDIN_FILENO)?;
    dup2(stdio.stdout.0, STDOUT_FILENO)?;
    dup2(stdio.stderr.0, STDERR_FILENO)?;

    if let Some(dir) = working_dir {
        chdir(dir.as_ref())?
    }

    let c_path = to_cstr(path)?;
    let mut c_args = vec![c_path.clone()];
    let mut c_env = Vec::new();

    for arg in args.into_iter() {
        c_args.push(to_cstr(arg)?);
    }

    for var in env.into_iter() {
        c_env.push(to_cstr(var)?);
    }

    kill(getpid(), Signal::SIGSTOP)?;
    execve(&c_path, &c_args, &c_env)?;
    Ok(())
}

fn task_wchar(pid: Pid) -> Option<u64> {
    let mut io = String::new();

    if fs::File::open(format!("/proc/{}/io", pid))
        .and_then(|mut f| f.read_to_string(&mut io))
        .is_err()
    {
        // If the process is in zombie state we'll get permission denied error.
        return None;
    }

    io.split_whitespace()
        .skip_while(|&s| s != "wchar:")
        .skip(1)
        .next()
        .map(|num| num.parse::<u64>().unwrap_or(0))
}
