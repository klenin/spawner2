use crate::process::{ExitStatus, LimitViolation, ResourceLimits, ResourceUsage};
use crate::sys::limit_checker::LimitChecker;
use crate::sys::unix::missing_decls::{sock_fprog, SECCOMP_MODE_FILTER};
use crate::sys::unix::pipe::{PipeFd, ReadPipe, WritePipe};
use crate::sys::unix::process_ext::SyscallFilter;
use crate::sys::{AsInnerMut, IntoInner};
use crate::{Error, Result};

use nix::errno::errno;
use nix::libc::{
    c_ushort, getpwnam, prctl, PR_SET_NO_NEW_PRIVS, PR_SET_SECCOMP, STDERR_FILENO, STDIN_FILENO,
    STDOUT_FILENO,
};
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{
    chdir, close, dup2, execvpe, fork, getpid, getuid, setgroups, setresgid, setresuid, ForkResult,
    Gid, Pid, Uid,
};

use cgroups_fs::{Cgroup, CgroupName};

use procfs::{self, FDTarget};

use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};

use std::collections::HashMap;
use std::ffi::CString;
use std::iter;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

pub struct Stdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

enum Env {
    Clear,
    Inherit,
}

pub struct ProcessInfo {
    app: String,
    args: Vec<String>,
    working_dir: Option<String>,
    suspended: bool,
    env: Env,
    envs: HashMap<String, String>,
    username: Option<String>,
    filter: Option<SyscallFilter>,
}

pub struct Process {
    pid: Pid,
    exit_status: Option<ExitStatus>,
}

pub struct GroupRestrictions(ResourceLimits);

pub struct Group {
    memory: Cgroup,
    cpuacct: Cgroup,
    pids: Cgroup,
    freezer: Cgroup,
    limit_checker: LimitChecker,
    creation_time: Instant,
    active_tasks: ActiveTasks,
    // Since we have information only about active tasks we need to memorize amount
    // of dead tasks and amount of bytes written by them.
    dead_tasks_info: DeadTasksInfo,
}

struct DeadTasksInfo {
    num_dead_tasks: usize,
    total_bytes_written: u64,
}

struct ActiveTasks {
    wchar_by_pid: HashMap<Pid, u64>,
    pid_by_inode: HashMap<u32, Pid>,
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

impl ProcessInfo {
    pub fn new<T: AsRef<str>>(app: T) -> Self {
        Self {
            app: app.as_ref().to_string(),
            args: Vec::new(),
            working_dir: None,
            suspended: false,
            env: Env::Inherit,
            envs: HashMap::new(),
            username: None,
            filter: None,
        }
    }

    pub fn args<T, U>(&mut self, args: T) -> &mut Self
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>,
    {
        self.args
            .extend(args.into_iter().map(|s| s.as_ref().to_string()));
        self
    }

    pub fn envs<I, K, V>(&mut self, envs: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.envs.extend(
            envs.into_iter()
                .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string())),
        );
        self
    }

    pub fn working_dir<T: AsRef<str>>(&mut self, dir: T) -> &mut Self {
        self.working_dir = Some(dir.as_ref().to_string());
        self
    }

    pub fn suspended(&mut self, v: bool) -> &mut Self {
        self.suspended = v;
        self
    }

    pub fn env_clear(&mut self) -> &mut Self {
        self.env = Env::Clear;
        self
    }

    pub fn env_inherit(&mut self) -> &mut Self {
        self.env = Env::Inherit;
        self
    }

    pub fn user<T, U>(&mut self, username: T, _password: Option<U>) -> &mut Self
    where
        T: AsRef<str>,
        U: AsRef<str>,
    {
        self.username = Some(username.as_ref().to_string());
        self
    }

    pub fn syscall_filter(&mut self, filter: SyscallFilter) -> &mut Self {
        self.filter = Some(filter);
        self
    }
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

    fn suspended(info: &mut ProcessInfo, stdio: RawStdio) -> Result<Self> {
        let usr = info.username.as_ref().map(User::new).transpose()?;

        if let ForkResult::Parent { child, .. } = fork()? {
            // Wait for initialization.
            waitpid(child, Some(WaitPidFlag::WSTOPPED))?;
            return Ok(Process {
                pid: child,
                exit_status: None,
            });
        }

        if let Err(_) = init_child_process(info, stdio, usr) {
            // todo: Send error to parent process.
        }
        process::exit(errno());
    }
}

impl AsMut<ProcessInfo> for ProcessInfo {
    fn as_mut(&mut self) -> &mut ProcessInfo {
        self
    }
}

impl GroupRestrictions {
    pub fn new<T: Into<ResourceLimits>>(limits: T) -> Self {
        Self(limits.into())
    }
}

impl Group {
    pub fn new<T>(restrictions: T) -> Result<Self>
    where
        T: Into<GroupRestrictions>,
    {
        let limits = restrictions.into().0;
        let memory = create_cgroup("memory/sp")?;
        let pids = create_cgroup("pids/sp")?;

        if let Some(mem_limit) = limits.peak_memory_used {
            memory.set_value("memory.limit_in_bytes", mem_limit)?;
        }
        if let Some(proc_count) = limits.active_processes {
            pids.set_value("pids.max", proc_count)?;
        }

        Ok(Self {
            memory: memory,
            cpuacct: create_cgroup("cpuacct/sp")?,
            pids: pids,
            freezer: create_cgroup("freezer/sp")?,
            limit_checker: LimitChecker::new(limits),
            creation_time: Instant::now(),
            active_tasks: ActiveTasks::new(),
            dead_tasks_info: DeadTasksInfo::new(),
        })
    }

    pub fn spawn<T, U>(&mut self, mut info: T, stdio: U) -> Result<Process>
    where
        T: AsMut<ProcessInfo>,
        U: Into<Stdio>,
    {
        let info = info.as_mut();
        let stdio = stdio.into();
        let ps = Process::suspended(
            info,
            RawStdio {
                stdin: stdio.stdin.into_inner(),
                stdout: stdio.stdout.into_inner(),
                stderr: stdio.stderr.into_inner(),
            },
        )?;
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

        let dead_tasks_info = self.active_tasks.update(self.freezer.get_tasks()?);
        self.dead_tasks_info.num_dead_tasks += dead_tasks_info.num_dead_tasks;
        self.dead_tasks_info.total_bytes_written += dead_tasks_info.total_bytes_written;

        Ok(ResourceUsage {
            wall_clock_time: self.creation_time.elapsed(),
            total_user_time: Duration::from_nanos(total_user_time),
            total_kernel_time: Duration::from_nanos(total_sys_time),
            peak_memory_used: max_mem_usage + max_kmem_usage,
            total_bytes_written: self.active_tasks.total_bytes_written()
                + self.dead_tasks_info.total_bytes_written,
            total_processes_created: self.dead_tasks_info.num_dead_tasks
                + self.active_tasks.count(),
            active_processes: self.active_tasks.count(),
            active_network_connections: self
                .active_tasks
                .count_network_connections()
                .map_err(|e| Error::from(e.to_string()))?,
        })
    }

    pub fn check_limits(&mut self) -> Result<Option<LimitViolation>> {
        if self.memory.get_value::<usize>("memory.failcnt")? > 0 {
            return Ok(Some(LimitViolation::MemoryLimitExceeded));
        }
        if self.pids.get_raw_value("pids.events")? != "max 0\n" {
            return Ok(Some(LimitViolation::ActiveProcessLimitExceeded));
        }

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
}

impl Drop for Group {
    fn drop(&mut self) {
        self.freezer.remove().ok();
        self.memory.remove().ok();
        self.cpuacct.remove().ok();
        self.pids.remove().ok();
    }
}

impl DeadTasksInfo {
    fn new() -> Self {
        Self {
            num_dead_tasks: 0,
            total_bytes_written: 0,
        }
    }
}

impl ActiveTasks {
    fn new() -> Self {
        Self {
            wchar_by_pid: HashMap::new(),
            pid_by_inode: HashMap::new(),
        }
    }

    fn count(&self) -> usize {
        self.wchar_by_pid.len()
    }

    fn total_bytes_written(&self) -> u64 {
        self.wchar_by_pid.values().sum()
    }

    fn count_network_connections(&self) -> procfs::ProcResult<usize> {
        let tcp_inodes = procfs::tcp()?
            .into_iter()
            .chain(procfs::tcp6()?)
            .map(|tcp_entry| tcp_entry.inode);

        let udp_inodes = procfs::udp()?
            .into_iter()
            .chain(procfs::udp6()?)
            .map(|udp_entry| udp_entry.inode);

        Ok(tcp_inodes
            .chain(udp_inodes)
            .filter(|inode| self.pid_by_inode.get(inode).is_some())
            .count())
    }

    fn update<T>(&mut self, pids: T) -> DeadTasksInfo
    where
        T: IntoIterator<Item = Pid>,
    {
        self.pid_by_inode.clear();

        let new_wchar_by_pid = pids
            .into_iter()
            .filter_map(|pid| procfs::Process::new(pid.as_raw()).ok())
            .map(|ps| {
                let pid = Pid::from_raw(ps.pid());

                if let Some(fds) = ps.fd().ok() {
                    self.pid_by_inode
                        .extend(fds.into_iter().filter_map(|fd| match fd.target {
                            FDTarget::Socket(inode) => Some((inode, pid)),
                            _ => None,
                        }));
                }

                (pid, ps.io().ok().map(|io| io.wchar))
            })
            .collect::<HashMap<Pid, Option<u64>>>();

        let old_wchar_by_pid = &mut self.wchar_by_pid;
        let dead_tasks = old_wchar_by_pid
            .iter_mut()
            .filter_map(|(pid, wchar)| match new_wchar_by_pid.get(pid) {
                Some(new_wchar) => {
                    *wchar += new_wchar.unwrap_or(0);
                    None
                }
                None => Some(pid.clone()),
            })
            .collect::<Vec<Pid>>();

        for (pid, wchar) in new_wchar_by_pid.iter() {
            if old_wchar_by_pid.get(pid).is_none() {
                old_wchar_by_pid.insert(pid.clone(), wchar.unwrap_or(0));
            }
        }

        DeadTasksInfo {
            num_dead_tasks: dead_tasks.len(),
            total_bytes_written: dead_tasks
                .into_iter()
                .map(|pid| old_wchar_by_pid.remove(&pid).unwrap())
                .sum(),
        }
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

fn seccomp_err() -> Error {
    Error::from(format!(
        "Failed to initialize seccomp: {}",
        Error::last_os_error()
    ))
}

fn init_seccomp(filter: &mut SyscallFilter) -> Result<()> {
    if unsafe { prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } == -1 {
        return Err(seccomp_err());
    }
    let inner = filter.as_inner_mut();
    let mut prog = sock_fprog {
        len: inner.len() as c_ushort,
        filter: inner.as_mut_ptr(),
    };
    if unsafe { prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &mut prog) } == -1 {
        return Err(seccomp_err());
    }
    Ok(())
}

fn init_child_process(info: &mut ProcessInfo, stdio: RawStdio, usr: Option<User>) -> Result<()> {
    close_stdio()?;
    dup2(stdio.stdin.0, STDIN_FILENO)?;
    dup2(stdio.stdout.0, STDOUT_FILENO)?;
    dup2(stdio.stderr.0, STDERR_FILENO)?;

    info.working_dir
        .as_ref()
        .map(|s| chdir(s.as_str()))
        .transpose()?;
    info.filter.as_mut().map(init_seccomp).transpose()?;
    usr.as_ref().map(User::impersonate).transpose()?;

    let mut env = match info.env {
        Env::Clear => HashMap::new(),
        Env::Inherit => std::env::vars().collect(),
    };
    env.extend(info.envs.iter().map(|(k, v)| (k.clone(), v.clone())));

    let c_cmd = to_cstr(info.app.as_str())?;
    let c_args = iter::once(info.app.as_str())
        .chain(info.args.iter().map(|s| s.as_str()))
        .map(to_cstr)
        .collect::<Result<Vec<_>>>()?;
    let c_env = env
        .into_iter()
        .map(|(k, v)| to_cstr(format!("{}={}", k, v)))
        .collect::<Result<Vec<_>>>()?;

    kill(getpid(), Signal::SIGSTOP)?;
    execvpe(&c_cmd, &c_args, &c_env)?;
    Ok(())
}
