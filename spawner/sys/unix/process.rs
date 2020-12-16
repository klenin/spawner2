use crate::process::{
    ExitStatus, GroupIo, GroupMemory, GroupNetwork, GroupPidCounters, GroupTimers, OsLimit,
};
use crate::sys::unix::missing_decls::{sock_fprog, SECCOMP_MODE_FILTER};
use crate::sys::unix::pipe::{PipeFd, ReadPipe, WritePipe};
use crate::sys::unix::process_ext::SyscallFilter;
use crate::sys::unix::shared_mem::SharedMem;
use crate::sys::{AsInnerMut, IntoInner};
use crate::{Error, Result};

use nix::errno::Errno;
use nix::libc::{
    c_ushort, getpwnam, prctl, PR_SET_NO_NEW_PRIVS, PR_SET_SECCOMP, STDERR_FILENO, STDIN_FILENO,
    STDOUT_FILENO,
};
use nix::sched::{sched_setaffinity, CpuSet};
use nix::sys::signal::{kill, raise, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{
    chdir, close, dup2, execve, execvpe, fork, setgroups, setresgid, setresuid, ForkResult, Gid,
    Pid, Uid,
};

use cgroups_fs::{Cgroup, CgroupName};

use procfs::process::FDTarget;

use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::iter;
use std::mem;
use std::os::unix::io::RawFd;
use std::process;
use std::thread;
use std::time::Duration;

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
    search_in_path: bool,
    env: Env,
    envs: HashMap<String, String>,
    username: Option<String>,
    filter: Option<SyscallFilter>,
    cpuset: Option<CpuSet>,
}

#[derive(Copy, Clone)]
enum InitError {
    Group(Option<nix::Error>),
    Other(nix::Error),
    Impersonate(nix::Error),
    Seccomp(nix::Error),
    CloseFd,
}

type InitResult = std::result::Result<(), InitError>;

enum ProcessStatus {
    Alive(SharedMem<InitResult>),
    Exited(ExitStatus),
}

pub struct Process {
    pid: Pid,
    status: ProcessStatus,
}

pub struct ResourceUsage<'a> {
    group: &'a Group,
    active_tasks: ActiveTasks,
    // Since we have information only about active tasks we need to memorize amount
    // of dead tasks and amount of bytes written by them.
    dead_tasks_info: DeadTasksInfo,
}

pub struct Group {
    memory: Cgroup,
    cpuacct: Cgroup,
    pids: Cgroup,
    freezer: Cgroup,
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
            search_in_path: true,
            env: Env::Inherit,
            envs: HashMap::new(),
            username: None,
            filter: None,
            cpuset: None,
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

    pub fn search_in_path(&mut self, v: bool) -> &mut Self {
        self.search_in_path = v;
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

    pub fn cpuset(&mut self, cpuset: CpuSet) -> &mut Self {
        self.cpuset = Some(cpuset);
        self
    }
}

impl Process {
    pub fn exit_status(&mut self) -> Result<Option<ExitStatus>> {
        if let ProcessStatus::Exited(ref status) = self.status {
            return Ok(Some(status.clone()));
        }

        let exit_status = match waitpid(self.pid, Some(WaitPidFlag::WNOHANG))? {
            WaitStatus::Exited(pid, code) => {
                assert_eq!(pid, self.pid);
                ExitStatus::Finished(code as u32)
            }
            WaitStatus::Signaled(pid, signal, _) => {
                assert_eq!(pid, self.pid);
                ExitStatus::Crashed(format!("Process terminated by the '{}' signal", signal))
            }
            _ => return Ok(None),
        };

        // Process has exited. Check initialization result.
        let init_error =
            match mem::replace(&mut self.status, ProcessStatus::Exited(exit_status.clone())) {
                ProcessStatus::Alive(r) => match *r.lock().unwrap() {
                    Ok(_) => return Ok(Some(exit_status)),
                    Err(e) => e,
                },
                _ => return Ok(Some(exit_status)),
            };

        match init_error {
            InitError::Other(e) => Err(Error::from(e)),
            InitError::Impersonate(e) => {
                Err(Error::from(format!("Failed to impersonate user: {}", e)))
            }
            InitError::Seccomp(e) => {
                Err(Error::from(format!("Failed to initialize seccomp: {}", e)))
            }
            InitError::Group(e) => match e {
                Some(e) => Err(Error::from(format!(
                    "Failed to add process to cgroup: {}",
                    e
                ))),
                None => Err(Error::from("Failed to add process to cgroup")),
            },
            InitError::CloseFd => Err(Error::from("Failed to close file descriptors")),
        }
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

    pub fn spawn(info: &mut ProcessInfo, stdio: Stdio) -> Result<Self> {
        create_process(info, stdio, None).map(|(pid, init_result)| Self {
            pid,
            status: ProcessStatus::Alive(init_result),
        })
    }

    pub fn spawn_in_group(info: &mut ProcessInfo, stdio: Stdio, group: &mut Group) -> Result<Self> {
        create_process(info, stdio, Some(group)).map(|(pid, init_result)| Self {
            pid,
            status: ProcessStatus::Alive(init_result),
        })
    }
}

impl<'a> ResourceUsage<'a> {
    pub fn new(group: &'a Group) -> Self {
        Self {
            group,
            active_tasks: ActiveTasks::new(),
            dead_tasks_info: DeadTasksInfo::new(),
        }
    }

    pub fn update(&mut self) -> Result<()> {
        let dead_tasks_info = self.active_tasks.update(&self.group.freezer)?;
        self.dead_tasks_info.num_dead_tasks += dead_tasks_info.num_dead_tasks;
        self.dead_tasks_info.total_bytes_written += dead_tasks_info.total_bytes_written;
        Ok(())
    }

    pub fn memory(&self) -> Result<Option<GroupMemory>> {
        let mem = &self.group.memory;
        Ok(Some(GroupMemory {
            max_usage: mem.get_value::<u64>("memory.max_usage_in_bytes")?
                + mem.get_value::<u64>("memory.kmem.max_usage_in_bytes")?,
        }))
    }

    pub fn io(&self) -> Result<Option<GroupIo>> {
        Ok(Some(GroupIo {
            total_bytes_written: self.active_tasks.total_bytes_written()
                + self.dead_tasks_info.total_bytes_written,
        }))
    }

    pub fn pid_counters(&self) -> Result<Option<GroupPidCounters>> {
        let active_processes = self.active_tasks.count();
        Ok(Some(GroupPidCounters {
            active_processes,
            total_processes: self.dead_tasks_info.num_dead_tasks + active_processes,
        }))
    }

    pub fn network(&self) -> Result<Option<GroupNetwork>> {
        Ok(Some(GroupNetwork {
            active_connections: self
                .active_tasks
                .count_network_connections()
                .map_err(|e| Error::from(e.to_string()))?,
        }))
    }

    pub fn timers(&self) -> Result<Option<GroupTimers>> {
        let cpuacct = &self.group.cpuacct;
        Ok(Some(GroupTimers {
            total_user_time: Duration::from_nanos(cpuacct.get_value::<u64>("cpuacct.usage_user")?),
            total_kernel_time: Duration::from_nanos(cpuacct.get_value::<u64>("cpuacct.usage_sys")?),
        }))
    }
}

impl Group {
    pub fn new() -> Result<Self> {
        Ok(Self {
            memory: create_cgroup("memory/sp")?,
            cpuacct: create_cgroup("cpuacct/sp")?,
            pids: create_cgroup("pids/sp")?,
            freezer: create_cgroup("freezer/sp")?,
        })
    }

    fn add_pid(&mut self, pid: Pid) -> std::io::Result<()> {
        self.memory
            .add_task(pid)
            .and(self.cpuacct.add_task(pid))
            .and(self.pids.add_task(pid))
            .and(self.freezer.add_task(pid))
    }

    pub fn add(&mut self, ps: &Process) -> Result<()> {
        self.add_pid(ps.pid).map_err(Error::from)
    }

    pub fn set_os_limit(&mut self, limit: OsLimit, value: u64) -> Result<bool> {
        match limit {
            OsLimit::Memory => {
                self.memory.set_value("memory.limit_in_bytes", value)?;
            }
            OsLimit::ActiveProcess => {
                self.pids.set_value("pids.max", value)?;
            }
        }
        Ok(true)
    }

    pub fn is_os_limit_hit(&self, limit: OsLimit) -> Result<bool> {
        match limit {
            OsLimit::Memory => Ok(self.memory.get_value::<usize>("memory.failcnt")? > 0),
            OsLimit::ActiveProcess => Ok(self.pids.get_raw_value("pids.events")? != "max 0\n"),
        }
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
        let tcp_inodes = procfs::net::tcp()?
            .into_iter()
            .chain(procfs::net::tcp6()?)
            .map(|tcp_entry| tcp_entry.inode);

        let udp_inodes = procfs::net::udp()?
            .into_iter()
            .chain(procfs::net::udp6()?)
            .map(|udp_entry| udp_entry.inode);

        Ok(tcp_inodes
            .chain(udp_inodes)
            .filter(|inode| self.pid_by_inode.get(inode).is_some())
            .count())
    }

    fn update(&mut self, freezer: &Cgroup) -> Result<DeadTasksInfo> {
        self.pid_by_inode.clear();
        let new_wchar_by_pid = freezer
            .get_tasks()?
            .into_iter()
            .filter_map(|pid| procfs::process::Process::new(pid.as_raw()).ok())
            .map(|ps| {
                let pid = Pid::from_raw(ps.pid());

                if let Ok(fds) = ps.fd() {
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
                    *wchar = std::cmp::max(*wchar, new_wchar.unwrap_or(0));
                    None
                }
                None => Some(*pid),
            })
            .collect::<Vec<Pid>>();

        for (pid, wchar) in new_wchar_by_pid.iter() {
            if old_wchar_by_pid.get(pid).is_none() {
                old_wchar_by_pid.insert(*pid, wchar.unwrap_or(0));
            }
        }

        Ok(DeadTasksInfo {
            num_dead_tasks: dead_tasks.len(),
            total_bytes_written: dead_tasks
                .into_iter()
                .map(|pid| old_wchar_by_pid.remove(&pid).unwrap())
                .sum(),
        })
    }
}

impl User {
    fn new(login: &str) -> Result<Self> {
        // todo: Check password?
        let pwd = unsafe { getpwnam(to_cstr(login)?.as_ptr()) };
        if pwd.is_null() {
            Err(Error::from(format!("Incorrect username '{}'", login)))
        } else {
            Ok(Self {
                uid: Uid::from_raw(unsafe { (*pwd).pw_uid }),
                gid: Gid::from_raw(unsafe { (*pwd).pw_gid }),
            })
        }
    }

    fn impersonate(&self) -> nix::Result<()> {
        setgroups(&[self.gid])?;
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

fn create_env(info: &ProcessInfo) -> Result<Vec<CString>> {
    let mut env = match info.env {
        Env::Clear => HashMap::new(),
        Env::Inherit => std::env::vars().collect(),
    };
    env.extend(info.envs.iter().map(|(k, v)| (k.clone(), v.clone())));

    env.into_iter()
        .map(|(k, v)| to_cstr(format!("{}={}", k, v)))
        .collect()
}

fn create_args(info: &ProcessInfo) -> Result<Vec<CString>> {
    iter::once(info.app.as_str())
        .chain(info.args.iter().map(|s| s.as_str()))
        .map(to_cstr)
        .collect()
}

fn close_all_fds(ignore: &[RawFd]) -> InitResult {
    procfs::process::Process::myself()
        .and_then(|ps| ps.fd())
        .map_err(|_| InitError::CloseFd)?
        .into_iter()
        .map(|fd_info| fd_info.fd as RawFd)
        .filter(|&fd| !ignore.iter().any(|&x| x == fd))
        .for_each(|fd| {
            let _ = close(fd);
        });
    Ok(())
}

fn init_stdio(stdio: RawStdio) -> nix::Result<()> {
    dup2(stdio.stdin.raw(), STDIN_FILENO)?;
    dup2(stdio.stdout.raw(), STDOUT_FILENO)?;
    dup2(stdio.stderr.raw(), STDERR_FILENO)?;
    Ok(())
}

fn init_seccomp(filter: &mut SyscallFilter) -> nix::Result<()> {
    if unsafe { prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } == -1 {
        return Err(nix::Error::last());
    }
    let inner = filter.as_inner_mut();
    let mut prog = sock_fprog {
        len: inner.len() as c_ushort,
        filter: inner.as_mut_ptr(),
    };
    if unsafe { prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &mut prog) } == -1 {
        return Err(nix::Error::last());
    }
    Ok(())
}

fn init_child_process(
    stdio: RawStdio,
    working_dir: Option<&str>,
    filter: Option<&mut SyscallFilter>,
    group: Option<&mut Group>,
    usr: Option<&User>,
    cpuset: Option<&CpuSet>,
) -> InitResult {
    group
        .map(|g| g.add_pid(Pid::this()))
        .transpose()
        .map_err(|e| {
            InitError::Group(
                e.raw_os_error()
                    .map(|v| nix::Error::from_errno(Errno::from_i32(v))),
            )
        })?;

    // Even though we set FD_CLOEXEC flag on all pipes, some child processes
    // still inherit pipes of their siblings.
    // Close all open file descriptors to fix this.
    close_all_fds(&[stdio.stdin.raw(), stdio.stdout.raw(), stdio.stderr.raw()])?;

    init_stdio(stdio)
        .and_then(|_| working_dir.map(chdir).transpose())
        .and_then(|_| {
            cpuset
                .map(|x| sched_setaffinity(Pid::this(), x))
                .transpose()
        })
        .map_err(InitError::Other)?;

    usr.map(User::impersonate)
        .transpose()
        .map_err(InitError::Impersonate)?;

    filter
        .map(init_seccomp)
        .transpose()
        .map_err(InitError::Seccomp)?;

    Ok(())
}

fn exec_app(app: &CStr, args: &[&CStr], env: &[&CStr], search_in_path: bool) -> nix::Result<()> {
    raise(Signal::SIGSTOP)?;
    if search_in_path {
        execvpe(app, args, env)?;
    } else {
        execve(app, args, env)?;
    }
    Ok(())
}

fn create_process(
    info: &mut ProcessInfo,
    stdio: Stdio,
    group: Option<&mut Group>,
) -> Result<(Pid, SharedMem<InitResult>)> {
    let usr = info
        .username
        .as_ref()
        .map(|s| User::new(s.as_str()))
        .transpose()?;
    let init_result = SharedMem::alloc(Ok(()))?;
    let app = to_cstr(info.app.as_str())?;
    let args = create_args(info)?;
    let args_ref = (0..args.len())
        .map(|i| args[i].as_c_str())
        .collect::<Vec<_>>();
    let env = create_env(info)?;
    let env_ref = (0..env.len())
        .map(|i| env[i].as_c_str())
        .collect::<Vec<_>>();

    if let ForkResult::Parent { child, .. } = fork()? {
        // Wait for initialization to complete.
        waitpid(child, Some(WaitPidFlag::WSTOPPED))?;
        if !info.suspended {
            kill(child, Signal::SIGCONT)?;
        }
        return Ok((child, init_result));
    }

    *init_result.lock().unwrap() = init_child_process(
        RawStdio {
            stdin: stdio.stdin.into_inner(),
            stdout: stdio.stdout.into_inner(),
            stderr: stdio.stderr.into_inner(),
        },
        info.working_dir.as_deref(),
        info.filter.as_mut(),
        group,
        usr.as_ref(),
        info.cpuset.as_ref(),
    )
    .and_then(|_| {
        exec_app(&app, &args_ref, &env_ref, info.search_in_path).map_err(InitError::Other)
    });

    process::exit(0);
}
