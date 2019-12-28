use crate::process::{
    ExitStatus, GroupIo, GroupMemory, GroupNetwork, GroupPidCounters, GroupTimers, OsLimit,
};
use crate::sys::windows::helpers::{
    cvt, to_utf16, Endpoints, EnvBlock, Handle, JobNotifications, PidList, RawStdio, StartupInfo,
    User, UserContext,
};
use crate::sys::windows::pipe::{ReadPipe, WritePipe};
use crate::sys::windows::process_ext::UiRestrictions;
use crate::sys::IntoInner;
use crate::{Error, Result};

use winapi::shared::minwindef::{DWORD, LPVOID, TRUE};
use winapi::um::errhandlingapi::SetErrorMode;
use winapi::um::jobapi2::{
    AssignProcessToJobObject, CreateJobObjectW, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject,
};
use winapi::um::minwinbase::STILL_ACTIVE;
use winapi::um::processthreadsapi::{
    CreateProcessAsUserW, CreateProcessW, GetExitCodeProcess, ResumeThread, SuspendThread,
    TerminateProcess, PROCESS_INFORMATION,
};
use winapi::um::winbase::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT,
    SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
};
use winapi::um::winnt::{
    JobObjectBasicAccountingInformation, JobObjectBasicAndIoAccountingInformation,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, JOBOBJECTINFOCLASS,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_JOB_MEMORY, STATUS_ACCESS_VIOLATION,
    STATUS_ARRAY_BOUNDS_EXCEEDED, STATUS_BREAKPOINT, STATUS_CONTROL_C_EXIT,
    STATUS_DATATYPE_MISALIGNMENT, STATUS_FLOAT_DENORMAL_OPERAND, STATUS_FLOAT_INEXACT_RESULT,
    STATUS_FLOAT_INVALID_OPERATION, STATUS_FLOAT_MULTIPLE_FAULTS, STATUS_FLOAT_MULTIPLE_TRAPS,
    STATUS_FLOAT_OVERFLOW, STATUS_FLOAT_STACK_CHECK, STATUS_FLOAT_UNDERFLOW,
    STATUS_GUARD_PAGE_VIOLATION, STATUS_ILLEGAL_INSTRUCTION, STATUS_INTEGER_DIVIDE_BY_ZERO,
    STATUS_INTEGER_OVERFLOW, STATUS_INVALID_DISPOSITION, STATUS_IN_PAGE_ERROR,
    STATUS_NONCONTINUABLE_EXCEPTION, STATUS_PRIVILEGED_INSTRUCTION, STATUS_REG_NAT_CONSUMPTION,
    STATUS_SINGLE_STEP, STATUS_STACK_OVERFLOW,
};

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::{self, Write};
use std::mem::{size_of_val, zeroed};
use std::ptr;
use std::time::Duration;
use std::u32;

enum Env {
    Clear,
    Inherit,
    User,
}

pub struct Stdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

pub struct ProcessInfo {
    app: String,
    args: Vec<String>,
    working_dir: Option<String>,
    show_window: bool,
    suspended: bool,
    env: Env,
    envs: HashMap<String, String>,
    user_creds: Option<(String, Option<String>)>,
}

pub struct Process {
    handle: Handle,
    main_thread: Handle,
    user: Option<User>,
}

unsafe impl Send for Process {}

pub struct ResourceUsage<'a> {
    group: &'a Group,
    pid_list: RefCell<PidList>,
    endpoints: RefCell<Endpoints>,
}

pub struct Group {
    job: Handle,
    notifications: RefCell<JobNotifications>,
}

impl ProcessInfo {
    pub fn new<T: AsRef<str>>(app: T) -> Self {
        Self {
            app: app.as_ref().to_string(),
            args: Vec::new(),
            working_dir: None,
            show_window: true,
            suspended: false,
            env: Env::Inherit,
            envs: HashMap::new(),
            user_creds: None,
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

    pub fn env_user(&mut self) -> &mut Self {
        self.env = Env::User;
        self
    }

    pub fn user<T, U>(&mut self, username: T, password: Option<U>) -> &mut Self
    where
        T: AsRef<str>,
        U: AsRef<str>,
    {
        self.user_creds = Some((
            username.as_ref().to_string(),
            password.map(|p| p.as_ref().to_string()),
        ));
        self
    }

    pub fn show_window(&mut self, show: bool) -> &mut Self {
        self.show_window = show;
        self
    }
}

impl AsRef<ProcessInfo> for ProcessInfo {
    fn as_ref(&self) -> &ProcessInfo {
        self
    }
}

impl Process {
    pub fn exit_status(&self) -> Result<Option<ExitStatus>> {
        let mut exit_code: DWORD = 0;
        unsafe {
            cvt(GetExitCodeProcess(self.handle.raw(), &mut exit_code))?;
        }
        // In this example agent can be suspended during process shutdown. This can give
        // us an exit code but it does not indicate process termination. To make sure that
        // process is properly terminated there is an extra `terminate` call.
        //
        // spawner_driver::run(&[
        //     "--separator=@",
        //     "-d=1",
        //     "--@",
        //     "--controller",
        //     "app.exe",
        //     "1W#\n",
        //     "wake_controller",
        //     "--@",
        //     "--in=*0.stdout",
        //     "--out=*0.stdin",
        //     "app.exe",
        //     "me",
        //     "ssa",
        //     "ge",
        //     "\n",
        // ]);
        Ok(match exit_code {
            STILL_ACTIVE => None,
            _ => {
                let _ = self.terminate();
                Some(match crash_cause(exit_code) {
                    Some(cause) => ExitStatus::Crashed(cause.to_string()),
                    None => ExitStatus::Finished(exit_code),
                })
            }
        })
    }

    pub fn suspend(&self) -> Result<()> {
        let _ctx = UserContext::enter(&self.user);
        unsafe {
            match SuspendThread(self.main_thread.raw()) {
                u32::MAX => Err(Error::last_os_error()),
                _ => Ok(()),
            }
        }
    }

    pub fn resume(&self) -> Result<()> {
        let _ctx = UserContext::enter(&self.user);
        unsafe {
            match ResumeThread(self.main_thread.raw()) {
                u32::MAX => Err(Error::last_os_error()),
                _ => Ok(()),
            }
        }
    }

    pub fn terminate(&self) -> Result<()> {
        unsafe {
            cvt(TerminateProcess(self.handle.raw(), 0))?;
        }
        Ok(())
    }

    pub fn spawn(info: &mut ProcessInfo, stdio: Stdio) -> Result<Self> {
        let ps = Self::suspended(info, stdio)?;
        if !info.suspended {
            ps.resume()?;
        }
        Ok(ps)
    }

    pub fn spawn_in_group(info: &mut ProcessInfo, stdio: Stdio, group: &mut Group) -> Result<Self> {
        let ps = Self::suspended(info, stdio)?;
        group.add(&ps)?;
        if !info.suspended {
            ps.resume()?;
        }
        Ok(ps)
    }

    fn suspended(info: &mut ProcessInfo, stdio: Stdio) -> Result<Self> {
        let mut user = info
            .user_creds
            .as_ref()
            .map(|(name, password)| User::create(name, password.as_ref()))
            .transpose()?;

        let mut env = match info.env {
            Env::Clear => HashMap::new(),
            Env::Inherit => std::env::vars().collect(),
            Env::User => EnvBlock::create(&user)?
                .iter()
                .map(|var| {
                    let idx = var.find('=').unwrap();
                    (var[0..idx].to_string(), var[idx + 1..].to_string())
                })
                .collect(),
        };
        env.extend(info.envs.iter().map(|(k, v)| (k.clone(), v.clone())));

        create_suspended_process(
            std::iter::once(&info.app).chain(info.args.iter()),
            env,
            RawStdio {
                stdin: stdio.stdin.into_inner(),
                stdout: stdio.stdout.into_inner(),
                stderr: stdio.stderr.into_inner(),
            },
            info.working_dir.as_ref(),
            user.as_mut(),
            info.show_window,
        )
        .map(|info| Self {
            handle: Handle::new(info.hProcess),
            main_thread: Handle::new(info.hThread),
            user: user,
        })
    }
}

macro_rules! count_endpoints {
    ($pids:expr, $endpoints:expr) => {
        $endpoints
            .iter()
            .filter(|row| {
                $pids
                    .iter()
                    .find(|&&pid| pid as DWORD == row.dwOwningPid)
                    .is_some()
            })
            .count()
    };
}

impl<'a> ResourceUsage<'a> {
    pub fn new(group: &'a Group) -> Self {
        Self {
            group: group,
            pid_list: RefCell::new(PidList::new()),
            endpoints: RefCell::new(Endpoints::new()),
        }
    }

    pub fn update(&mut self) -> Result<()> {
        Ok(())
    }

    pub fn timers(&self) -> Result<Option<GroupTimers>> {
        self.group.basic_info().map(|info| {
            // Total user time in 100-nanosecond ticks.
            let total_user_time = unsafe { *info.TotalUserTime.QuadPart() } as u64;
            // Total kernel time in 100-nanosecond ticks.
            let total_kernel_time = unsafe { *info.TotalKernelTime.QuadPart() } as u64;

            Some(GroupTimers {
                total_user_time: Duration::from_nanos(total_user_time * 100),
                total_kernel_time: Duration::from_nanos(total_kernel_time * 100),
            })
        })
    }

    pub fn memory(&self) -> Result<Option<GroupMemory>> {
        self.group.ext_limit_info().map(|info| {
            Some(GroupMemory {
                max_usage: info.PeakJobMemoryUsed as u64,
            })
        })
    }

    pub fn io(&self) -> Result<Option<GroupIo>> {
        self.group.basic_and_io_info().map(|info| {
            Some(GroupIo {
                total_bytes_written: info.IoInfo.WriteTransferCount,
            })
        })
    }

    pub fn pid_counters(&self) -> Result<Option<GroupPidCounters>> {
        self.group.basic_and_io_info().map(|info| {
            Some(GroupPidCounters {
                total_processes: info.BasicInfo.TotalProcesses as usize,
                active_processes: info.BasicInfo.ActiveProcesses as usize,
            })
        })
    }

    pub fn network(&self) -> Result<Option<GroupNetwork>> {
        let mut pid_list = self.pid_list.borrow_mut();
        let pids = pid_list.update(&self.group.job)?;
        let mut endpoints = self.endpoints.borrow_mut();

        Ok(Some(GroupNetwork {
            active_connections: count_endpoints!(pids, endpoints.load_tcpv4()?)
                + count_endpoints!(pids, endpoints.load_tcpv6()?)
                + count_endpoints!(pids, endpoints.load_udpv4()?)
                + count_endpoints!(pids, endpoints.load_udpv6()?),
        }))
    }
}

impl Group {
    pub fn new() -> Result<Self> {
        unsafe { cvt(CreateJobObjectW(ptr::null_mut(), ptr::null())) }
            .map(Handle::new)
            .map_err(Error::from)
            .and_then(|job| {
                JobNotifications::new(&job).map(|notifications| Self {
                    job: job,
                    notifications: RefCell::new(notifications),
                })
            })
    }

    pub fn set_ui_restrictions<T>(&mut self, restrictions: T) -> Result<()>
    where
        T: Into<UiRestrictions>,
    {
        let mut ui_restrictions = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: restrictions.into().into_inner(),
        };
        unsafe {
            cvt(SetInformationJobObject(
                /*hJob=*/ self.job.raw(),
                /*JobObjectInformationClass=*/ JobObjectBasicUIRestrictions,
                /*lpJobObjectInformation=*/ &mut ui_restrictions as *mut _ as LPVOID,
                /*cbJobObjectInformationLength=*/
                size_of_val(&ui_restrictions) as DWORD,
            ))?;
        }
        Ok(())
    }

    pub fn add(&self, ps: &Process) -> Result<()> {
        unsafe { cvt(AssignProcessToJobObject(self.job.raw(), ps.handle.raw()))? };
        Ok(())
    }

    pub fn set_os_limit(&mut self, limit: OsLimit, value: u64) -> Result<bool> {
        let mut ext_limit_info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };

        match limit {
            OsLimit::Memory => {
                ext_limit_info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
                ext_limit_info.JobMemoryLimit = value as usize;
            }
            OsLimit::ActiveProcess => {
                ext_limit_info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
                ext_limit_info.BasicLimitInformation.ActiveProcessLimit = value as DWORD;
            }
        }

        unsafe {
            cvt(SetInformationJobObject(
                /*hJob=*/ self.job.raw(),
                /*JobObjectInformationClass=*/ JobObjectExtendedLimitInformation,
                /*lpJobObjectInformation=*/ &mut ext_limit_info as *mut _ as LPVOID,
                /*cbJobObjectInformationLength=*/ size_of_val(&ext_limit_info) as DWORD,
            ))?;
        }

        Ok(true)
    }

    pub fn is_os_limit_hit(&self, limit: OsLimit) -> Result<bool> {
        let mut notifications = self.notifications.borrow_mut();
        match limit {
            OsLimit::Memory => notifications.is_memory_limit_hit(),
            OsLimit::ActiveProcess => notifications.is_active_process_limit_hit(),
        }
    }

    pub fn terminate(&self) -> Result<()> {
        cvt(unsafe { TerminateJobObject(self.job.raw(), 0) })?;
        Ok(())
    }

    fn query_info<T>(&self, class: JOBOBJECTINFOCLASS) -> Result<T> {
        unsafe {
            let mut info = zeroed::<T>();
            cvt(QueryInformationJobObject(
                /*hJob=*/ self.job.raw(),
                /*JobObjectInfoClass=*/ class,
                /*lpJobObjectInfo=*/ &mut info as *mut _ as LPVOID,
                /*cbJobObjectInfoLength=*/ size_of_val(&info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))
            .map_err(Error::from)
            .map(|_| info)
        }
    }

    fn basic_info(&self) -> Result<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION> {
        self.query_info(JobObjectBasicAccountingInformation)
    }

    fn basic_and_io_info(&self) -> Result<JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION> {
        self.query_info(JobObjectBasicAndIoAccountingInformation)
    }

    fn ext_limit_info(&self) -> Result<JOBOBJECT_EXTENDED_LIMIT_INFORMATION> {
        self.query_info(JobObjectExtendedLimitInformation)
    }
}

fn create_suspended_process<K, V, E, S, T, U>(
    argv: T,
    env: E,
    stdio: RawStdio,
    working_dir: Option<U>,
    user: Option<&mut User>,
    show_window: bool,
) -> Result<PROCESS_INFORMATION>
where
    K: AsRef<str>,
    V: AsRef<str>,
    E: IntoIterator<Item = (K, V)>,
    S: AsRef<str>,
    T: IntoIterator<Item = S>,
    U: AsRef<str>,
{
    let mut cmd = argv_to_cmd(argv);
    let mut env = create_env(env);
    let creation_flags =
        CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED;
    let working_dir = working_dir.map_or(ptr::null(), |dir| to_utf16(dir.as_ref()).as_ptr());
    let user_token = user.as_ref().map(|u| u.token().raw());

    let mut inherited_handles = [stdio.stdin.raw(), stdio.stdout.raw(), stdio.stderr.raw()];
    let mut startup_info = StartupInfo::create(&stdio, &mut inherited_handles, user, show_window)?;

    let mut process_info: PROCESS_INFORMATION = unsafe { zeroed() };

    unsafe {
        SetErrorMode(SEM_NOGPFAULTERRORBOX | SEM_FAILCRITICALERRORS);
        let result = if let Some(user_token) = user_token {
            CreateProcessAsUserW(
                /*hToken=*/ user_token,
                /*lpApplicationName=*/ ptr::null(),
                /*lpCommandLine=*/ cmd.as_mut_ptr(),
                /*lpProcessAttributes=*/ ptr::null_mut(),
                /*lpThreadAttributes=*/ ptr::null_mut(),
                /*bInheritHandles=*/ TRUE,
                /*dwCreationFlags=*/ creation_flags,
                /*lpEnvironment=*/ env.as_mut_ptr() as LPVOID,
                /*lpCurrentDirectory=*/ working_dir,
                /*lpStartupInfo=*/ startup_info.as_mut_ptr(),
                /*lpProcessInformation=*/ &mut process_info,
            )
        } else {
            CreateProcessW(
                /*lpApplicationName=*/ ptr::null(),
                /*lpCommandLine=*/ cmd.as_mut_ptr(),
                /*lpProcessAttributes=*/ ptr::null_mut(),
                /*lpThreadAttributes=*/ ptr::null_mut(),
                /*bInheritHandles=*/ TRUE,
                /*dwCreationFlags=*/ creation_flags,
                /*lpEnvironment=*/ env.as_mut_ptr() as LPVOID,
                /*lpCurrentDirectory=*/ working_dir,
                /*lpStartupInfo=*/ startup_info.as_mut_ptr(),
                /*lpProcessInformation=*/ &mut process_info,
            )
        };

        // Restore default error mode.
        SetErrorMode(0);
        cvt(result)?;
    }

    Ok(process_info)
}

fn argv_to_cmd<T, U>(argv: T) -> Vec<u16>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    let mut cmd = String::new();
    for (idx, arg) in argv.into_iter().enumerate() {
        if idx != 0 {
            cmd.write_char(' ').unwrap();
        }
        write_quoted(&mut cmd, arg.as_ref());
    }
    to_utf16(cmd)
}

fn write_quoted<W, S>(w: &mut W, s: S)
where
    W: fmt::Write,
    S: AsRef<str>,
{
    let escaped = s.as_ref().replace("\"", "\\\"");
    if escaped.find(' ').is_some() {
        write!(w, "\"{}\"", escaped)
    } else {
        w.write_str(escaped.as_str())
    }
    .unwrap();
}

fn create_env<I, K, V>(vars: I) -> Vec<u16>
where
    K: AsRef<str>,
    V: AsRef<str>,
    I: IntoIterator<Item = (K, V)>,
{
    let mut result = vars
        .into_iter()
        .map(|(k, v)| to_utf16(format!("{}={}", k.as_ref(), v.as_ref())))
        .flatten()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    // Environment block is terminated by 2 zeros.
    if result.len() == 1 {
        result.push(0);
    }
    result
}

fn crash_cause(exit_code: DWORD) -> Option<&'static str> {
    match exit_code {
        STATUS_ACCESS_VIOLATION => Some("AccessViolation"),
        STATUS_ARRAY_BOUNDS_EXCEEDED => Some("ArrayBoundsExceeded"),
        STATUS_BREAKPOINT => Some("Breakpoint"),
        STATUS_CONTROL_C_EXIT => Some("Control_C_Exit"),
        STATUS_DATATYPE_MISALIGNMENT => Some("DatatypeMisalignment"),
        STATUS_FLOAT_DENORMAL_OPERAND => Some("FloatDenormalOperand"),
        STATUS_FLOAT_INEXACT_RESULT => Some("FloatInexactResult"),
        STATUS_FLOAT_INVALID_OPERATION => Some("FloatInvalidOperation"),
        STATUS_FLOAT_MULTIPLE_FAULTS => Some("FloatMultipleFaults"),
        STATUS_FLOAT_MULTIPLE_TRAPS => Some("FloatMultipleTraps"),
        STATUS_FLOAT_OVERFLOW => Some("FloatOverflow"),
        STATUS_FLOAT_STACK_CHECK => Some("FloatStackCheck"),
        STATUS_FLOAT_UNDERFLOW => Some("FloatUnderflow"),
        STATUS_GUARD_PAGE_VIOLATION => Some("GuardPageViolation"),
        STATUS_ILLEGAL_INSTRUCTION => Some("IllegalInstruction"),
        STATUS_IN_PAGE_ERROR => Some("InPageError"),
        STATUS_INVALID_DISPOSITION => Some("InvalidDisposition"),
        STATUS_INTEGER_DIVIDE_BY_ZERO => Some("IntegerDivideByZero"),
        STATUS_INTEGER_OVERFLOW => Some("IntegerOverflow"),
        STATUS_NONCONTINUABLE_EXCEPTION => Some("NoncontinuableException"),
        STATUS_PRIVILEGED_INSTRUCTION => Some("PrivilegedInstruction"),
        STATUS_REG_NAT_CONSUMPTION => Some("RegNatConsumption"),
        STATUS_SINGLE_STEP => Some("SingleStep"),
        STATUS_STACK_OVERFLOW => Some("StackOverflow"),
        _ => None,
    }
}
