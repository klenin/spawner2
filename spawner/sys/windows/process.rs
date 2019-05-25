use crate::process::{ExitStatus, LimitViolation, ResourceLimits, ResourceUsage};
use crate::sys::limit_checker::LimitChecker;
use crate::sys::windows::helpers::{
    cvt, to_utf16, Endpoints, EnvBlock, Handle, PidList, RawStdio, StartupInfo, User, UserContext,
};
use crate::sys::windows::pipe::{ReadPipe, WritePipe};
use crate::sys::windows::process_ext::UiRestrictions;
use crate::sys::IntoInner;
use crate::{Error, Result};

use winapi::shared::minwindef::{DWORD, TRUE};
use winapi::um::errhandlingapi::SetErrorMode;
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use winapi::um::ioapiset::{CreateIoCompletionPort, GetQueuedCompletionStatus};
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
    JobObjectAssociateCompletionPortInformation, JobObjectBasicAndIoAccountingInformation,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation,
    JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_JOB_MEMORY,
    JOB_OBJECT_MSG_ACTIVE_PROCESS_LIMIT, JOB_OBJECT_MSG_JOB_MEMORY_LIMIT, STATUS_ACCESS_VIOLATION,
    STATUS_ARRAY_BOUNDS_EXCEEDED, STATUS_BREAKPOINT, STATUS_CONTROL_C_EXIT,
    STATUS_DATATYPE_MISALIGNMENT, STATUS_FLOAT_DENORMAL_OPERAND, STATUS_FLOAT_INEXACT_RESULT,
    STATUS_FLOAT_INVALID_OPERATION, STATUS_FLOAT_MULTIPLE_FAULTS, STATUS_FLOAT_MULTIPLE_TRAPS,
    STATUS_FLOAT_OVERFLOW, STATUS_FLOAT_STACK_CHECK, STATUS_FLOAT_UNDERFLOW,
    STATUS_GUARD_PAGE_VIOLATION, STATUS_ILLEGAL_INSTRUCTION, STATUS_INTEGER_DIVIDE_BY_ZERO,
    STATUS_INTEGER_OVERFLOW, STATUS_INVALID_DISPOSITION, STATUS_IN_PAGE_ERROR,
    STATUS_NONCONTINUABLE_EXCEPTION, STATUS_PRIVILEGED_INSTRUCTION, STATUS_REG_NAT_CONSUMPTION,
    STATUS_SINGLE_STEP, STATUS_STACK_OVERFLOW,
};

use std::collections::HashMap;
use std::fmt::{self, Write};
use std::mem;
use std::ptr;
use std::time::{Duration, Instant};
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

pub struct GroupRestrictions {
    limits: ResourceLimits,
    ui_restrictions: Option<UiRestrictions>,
}

pub struct Group {
    job: Handle,
    completion_port: Handle,
    limit_checker: LimitChecker,
    creation_time: Instant,
    pid_list: PidList,
    endpoints: Endpoints,
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

    fn suspended<T, U>(info: T, stdio: U) -> Result<Self>
    where
        T: AsRef<ProcessInfo>,
        U: Into<Stdio>,
    {
        let stdio = stdio.into();
        let info = info.as_ref();

        let mut user = info
            .user_creds
            .as_ref()
            .map(|(name, password)| User::create(name, password.as_ref()))
            .transpose()?;

        let mut env = match info.env {
            Env::Clear => HashMap::new(),
            Env::Inherit => std::env::vars().collect(),
            Env::User => {
                let block = EnvBlock::create(&user)?;
                let x = block
                    .iter()
                    .map(|var| {
                        let idx = var.find('=').unwrap();
                        (var[0..idx].to_string(), var[idx + 1..].to_string())
                    })
                    .collect();
                x
            }
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

impl GroupRestrictions {
    pub fn new<T: Into<ResourceLimits>>(limits: T) -> Self {
        Self {
            limits: limits.into(),
            ui_restrictions: None,
        }
    }

    pub fn ui_restrictions<T>(&mut self, r: T) -> &mut Self
    where
        T: Into<UiRestrictions>,
    {
        self.ui_restrictions = Some(r.into());
        self
    }
}

impl Group {
    pub fn new<T>(restrictions: T) -> Result<Self>
    where
        T: Into<GroupRestrictions>,
    {
        let restrictions = restrictions.into();
        let limits = restrictions.limits.clone();
        create_job(restrictions).and_then(|job| {
            create_job_completion_port(&job).map(|port| Self {
                job: job,
                completion_port: port,
                limit_checker: LimitChecker::new(limits),
                creation_time: Instant::now(),
                pid_list: PidList::new(),
                endpoints: Endpoints::new(),
            })
        })
    }

    pub fn spawn<T, U>(&mut self, info: T, stdio: U) -> Result<Process>
    where
        T: AsRef<ProcessInfo>,
        U: Into<Stdio>,
    {
        let info = info.as_ref();
        let ps = Process::suspended(info, stdio)?;
        cvt(unsafe { AssignProcessToJobObject(self.job.raw(), ps.handle.raw()) })
            .map_err(Error::from)
            .and_then(|_| if info.suspended { Ok(()) } else { ps.resume() })
            .map_err(|e| {
                let _ = ps.terminate();
                e
            })
            .map(|_| ps)
    }

    pub fn resource_usage(&mut self) -> Result<ResourceUsage> {
        let basic_and_io_info = self.basic_and_io_info()?;
        let ext_limit_info = self.ext_limit_info()?;

        // Total user time in 100-nanosecond ticks.
        let total_user_time =
            unsafe { *basic_and_io_info.BasicInfo.TotalUserTime.QuadPart() } as u64;

        // Total kernel time in 100-nanosecond ticks.
        let total_kernel_time =
            unsafe { *basic_and_io_info.BasicInfo.TotalKernelTime.QuadPart() } as u64;

        Ok(ResourceUsage {
            wall_clock_time: self.creation_time.elapsed(),
            total_user_time: Duration::from_nanos(total_user_time * 100),
            total_kernel_time: Duration::from_nanos(total_kernel_time * 100),
            peak_memory_used: ext_limit_info.PeakJobMemoryUsed as u64,
            total_processes_created: basic_and_io_info.BasicInfo.TotalProcesses as usize,
            active_processes: basic_and_io_info.BasicInfo.ActiveProcesses as usize,
            total_bytes_written: basic_and_io_info.IoInfo.WriteTransferCount,
            active_network_connections: self.active_network_connections()?,
        })
    }

    pub fn check_limits(&mut self) -> Result<Option<LimitViolation>> {
        let mut num_bytes = 0;
        let mut _key = 0;
        let mut _overlapped = ptr::null_mut();
        if unsafe {
            GetQueuedCompletionStatus(
                /*CompletionPort=*/ self.completion_port.raw(),
                /*lpNumberOfBytes=*/ &mut num_bytes,
                /*lpCompletionKey=*/ &mut _key,
                /*lpOverlapped=*/ &mut _overlapped,
                /*dwMilliseconds=*/ 0,
            )
        } == TRUE
        {
            match num_bytes {
                JOB_OBJECT_MSG_JOB_MEMORY_LIMIT => {
                    return Ok(Some(LimitViolation::MemoryLimitExceeded));
                }
                JOB_OBJECT_MSG_ACTIVE_PROCESS_LIMIT => {
                    return Ok(Some(LimitViolation::ActiveProcessLimitExceeded));
                }
                _ => {}
            }
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
        cvt(unsafe { TerminateJobObject(self.job.raw(), 0) })?;
        Ok(())
    }

    fn basic_and_io_info(&self) -> Result<JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION> {
        unsafe {
            let mut basic_and_io_info: JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION =
                mem::zeroed();

            cvt(QueryInformationJobObject(
                /*hJob=*/ self.job.raw(),
                /*JobObjectInfoClass=*/ JobObjectBasicAndIoAccountingInformation,
                /*lpJobObjectInfo=*/ mem::transmute(&mut basic_and_io_info),
                /*cbJobObjectInfoLength=*/ mem::size_of_val(&basic_and_io_info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))
            .map_err(Error::from)
            .map(|_| basic_and_io_info)
        }
    }

    fn ext_limit_info(&self) -> Result<JOBOBJECT_EXTENDED_LIMIT_INFORMATION> {
        unsafe {
            let mut ext_info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
            cvt(QueryInformationJobObject(
                /*hJob=*/ self.job.raw(),
                /*JobObjectInfoClass=*/ JobObjectExtendedLimitInformation,
                /*lpJobObjectInfo=*/ mem::transmute(&mut ext_info),
                /*cbJobObjectInfoLength=*/ mem::size_of_val(&ext_info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))
            .map_err(Error::from)
            .map(|_| ext_info)
        }
    }

    fn active_network_connections(&mut self) -> Result<usize> {
        let pids = self.pid_list.update(&self.job)?;
        Ok(count_endpoints!(pids, self.endpoints.load_tcpv4()?)
            + count_endpoints!(pids, self.endpoints.load_tcpv6()?)
            + count_endpoints!(pids, self.endpoints.load_udpv4()?)
            + count_endpoints!(pids, self.endpoints.load_udpv6()?))
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

    let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };

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
                /*lpEnvironment=*/ mem::transmute(env.as_mut_ptr()),
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
                /*lpEnvironment=*/ mem::transmute(env.as_mut_ptr()),
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

fn create_job(restrictions: GroupRestrictions) -> Result<Handle> {
    let limits = restrictions.limits;
    unsafe {
        let job = Handle::new(cvt(CreateJobObjectW(ptr::null_mut(), ptr::null()))?);

        let mut ext_limit_info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
        if let Some(mem_limit) = limits.peak_memory_used {
            ext_limit_info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
            ext_limit_info.JobMemoryLimit = mem_limit as usize;
        }
        if let Some(proc_count) = limits.active_processes {
            ext_limit_info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            ext_limit_info.BasicLimitInformation.ActiveProcessLimit = proc_count as DWORD;
        }

        cvt(SetInformationJobObject(
            /*hJob=*/ job.raw(),
            /*JobObjectInformationClass=*/ JobObjectExtendedLimitInformation,
            /*lpJobObjectInformation=*/ mem::transmute(&mut ext_limit_info),
            /*cbJobObjectInformationLength=*/ mem::size_of_val(&ext_limit_info) as DWORD,
        ))?;

        if let Some(class) = restrictions.ui_restrictions.map(IntoInner::into_inner) {
            let mut ui_restrictions = JOBOBJECT_BASIC_UI_RESTRICTIONS {
                UIRestrictionsClass: class,
            };
            cvt(SetInformationJobObject(
                /*hJob=*/ job.raw(),
                /*JobObjectInformationClass=*/ JobObjectBasicUIRestrictions,
                /*lpJobObjectInformation=*/ mem::transmute(&mut ui_restrictions),
                /*cbJobObjectInformationLength=*/
                mem::size_of_val(&ui_restrictions) as DWORD,
            ))?;
        }

        Ok(job)
    }
}

fn create_job_completion_port(job: &Handle) -> Result<Handle> {
    unsafe {
        let port = Handle::new(cvt(CreateIoCompletionPort(
            /*FileHandle=*/ INVALID_HANDLE_VALUE,
            /*ExistingCompletionPort=*/ ptr::null_mut(),
            /*CompletionKey=*/ 0,
            /*NumberOfConcurrentThreads=*/ 1,
        ))?);

        let mut port_info = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: ptr::null_mut(),
            CompletionPort: port.raw(),
        };

        cvt(SetInformationJobObject(
            /*hJob=*/ job.raw(),
            /*JobObjectInformationClass=*/ JobObjectAssociateCompletionPortInformation,
            /*lpJobObjectInformation=*/ mem::transmute(&mut port_info),
            /*cbJobObjectInformationLength=*/ mem::size_of_val(&port_info) as DWORD,
        ))?;

        Ok(port)
    }
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
