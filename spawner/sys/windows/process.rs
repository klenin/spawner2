use crate::process::{
    Environment, ExitStatus, LimitViolation, ProcessInfo, ResourceLimits, ResourceUsage,
};
use crate::sys::limit_checker::LimitChecker;
use crate::sys::windows::common::{cvt, to_utf16, Handle};
use crate::sys::windows::pipe::{ReadPipe, WritePipe};
use crate::sys::windows::utils::{EnvBlock, RawStdio, StartupInfo, User, UserContext};
use crate::sys::IntoInner;
use crate::{Error, Result};

use winapi::shared::minwindef::{DWORD, TRUE};
use winapi::um::errhandlingapi::SetErrorMode;
use winapi::um::jobapi2::{
    AssignProcessToJobObject, CreateJobObjectW, QueryInformationJobObject, TerminateJobObject,
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
    JobObjectBasicAndIoAccountingInformation, JobObjectExtendedLimitInformation,
    JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    STATUS_ACCESS_VIOLATION, STATUS_ARRAY_BOUNDS_EXCEEDED, STATUS_BREAKPOINT,
    STATUS_CONTROL_C_EXIT, STATUS_DATATYPE_MISALIGNMENT, STATUS_FLOAT_DENORMAL_OPERAND,
    STATUS_FLOAT_INEXACT_RESULT, STATUS_FLOAT_INVALID_OPERATION, STATUS_FLOAT_MULTIPLE_FAULTS,
    STATUS_FLOAT_MULTIPLE_TRAPS, STATUS_FLOAT_OVERFLOW, STATUS_FLOAT_STACK_CHECK,
    STATUS_FLOAT_UNDERFLOW, STATUS_GUARD_PAGE_VIOLATION, STATUS_ILLEGAL_INSTRUCTION,
    STATUS_INTEGER_DIVIDE_BY_ZERO, STATUS_INTEGER_OVERFLOW, STATUS_INVALID_DISPOSITION,
    STATUS_IN_PAGE_ERROR, STATUS_NONCONTINUABLE_EXCEPTION, STATUS_PRIVILEGED_INSTRUCTION,
    STATUS_REG_NAT_CONSUMPTION, STATUS_SINGLE_STEP, STATUS_STACK_OVERFLOW,
};

use std::collections::HashMap;
use std::fmt::{self, Write};
use std::mem;
use std::ptr;
use std::time::{Duration, Instant};
use std::u32;

pub struct ProcessStdio {
    pub stdin: ReadPipe,
    pub stdout: WritePipe,
    pub stderr: WritePipe,
}

pub struct Process {
    handle: Handle,
    main_thread: Handle,
    user: Option<User>,
}

unsafe impl Send for Process {}

pub struct Group {
    handle: Handle,
    limit_checker: LimitChecker,
    creation_time: Instant,
}

impl Process {
    pub fn exit_status(&self) -> Result<Option<ExitStatus>> {
        let mut exit_code: DWORD = 0;
        unsafe {
            cvt(GetExitCodeProcess(self.handle.0, &mut exit_code))?;
        }
        Ok(match exit_code {
            STILL_ACTIVE => None,
            _ => Some(match crash_cause(exit_code) {
                Some(cause) => ExitStatus::Crashed(cause.to_string()),
                None => ExitStatus::Finished(exit_code),
            }),
        })
    }

    pub fn suspend(&self) -> Result<()> {
        let _ctx = UserContext::enter(&self.user);
        unsafe {
            match SuspendThread(self.main_thread.0) {
                u32::MAX => Err(Error::last_os_error()),
                _ => Ok(()),
            }
        }
    }

    pub fn resume(&self) -> Result<()> {
        let _ctx = UserContext::enter(&self.user);
        unsafe {
            match ResumeThread(self.main_thread.0) {
                u32::MAX => Err(Error::last_os_error()),
                _ => Ok(()),
            }
        }
    }

    pub fn terminate(&self) -> Result<()> {
        unsafe {
            cvt(TerminateProcess(self.handle.0, 0))?;
        }
        Ok(())
    }

    fn suspended<T, U>(info: T, stdio: U) -> Result<Self>
    where
        T: AsRef<ProcessInfo>,
        U: Into<ProcessStdio>,
    {
        let stdio = stdio.into();
        let info = info.as_ref();

        let mut user = info
            .username
            .as_ref()
            .map(|uname| User::create(uname, info.password.as_ref()))
            .transpose()?;

        let mut env = match info.env {
            Environment::Inherit => std::env::vars().collect(),
            Environment::Clear => HashMap::new(),
            Environment::UserDefault => {
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
        env.extend(info.env_vars.iter().cloned());

        create_suspended_process(
            std::iter::once(&info.app).chain(info.args.iter()),
            env,
            RawStdio {
                stdin: stdio.stdin.into_inner(),
                stdout: stdio.stdout.into_inner(),
                stderr: stdio.stderr.into_inner(),
            },
            info.working_directory.as_ref(),
            user.as_mut(),
            info.show_window,
        )
        .map(|info| Self {
            handle: Handle(info.hProcess),
            main_thread: Handle(info.hThread),
            user: user,
        })
    }
}

impl Group {
    pub fn new() -> Result<Self> {
        let handle = Handle(unsafe { cvt(CreateJobObjectW(ptr::null_mut(), ptr::null()))? });
        Ok(Self {
            handle: handle,
            limit_checker: LimitChecker::new(),
            creation_time: Instant::now(),
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
        cvt(unsafe { AssignProcessToJobObject(self.handle.0, ps.handle.0) })
            .and_then(|_| if info.suspended { Ok(()) } else { ps.resume() })
            .map_err(|e| {
                let _ = ps.terminate();
                e
            })
            .map(|_| ps)
    }

    pub fn resource_usage(&self) -> Result<ResourceUsage> {
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
        cvt(unsafe { TerminateJobObject(self.handle.0, 0) })?;
        Ok(())
    }

    fn basic_and_io_info(&self) -> Result<JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION> {
        unsafe {
            let mut basic_and_io_info: JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION =
                mem::zeroed();

            cvt(QueryInformationJobObject(
                /*hJob=*/ self.handle.0,
                /*JobObjectInfoClass=*/ JobObjectBasicAndIoAccountingInformation,
                /*lpJobObjectInfo=*/ mem::transmute(&mut basic_and_io_info),
                /*cbJobObjectInfoLength=*/ mem::size_of_val(&basic_and_io_info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))
            .map(|_| basic_and_io_info)
        }
    }

    fn ext_limit_info(&self) -> Result<JOBOBJECT_EXTENDED_LIMIT_INFORMATION> {
        unsafe {
            let mut ext_info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
            cvt(QueryInformationJobObject(
                /*hJob=*/ self.handle.0,
                /*JobObjectInfoClass=*/ JobObjectExtendedLimitInformation,
                /*lpJobObjectInfo=*/ mem::transmute(&mut ext_info),
                /*cbJobObjectInfoLength=*/ mem::size_of_val(&ext_info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))
            .map(|_| ext_info)
        }
    }
}

fn create_suspended_process<S, T, U>(
    argv: T,
    env: HashMap<String, String>,
    stdio: RawStdio,
    working_dir: Option<U>,
    user: Option<&mut User>,
    show_window: bool,
) -> Result<PROCESS_INFORMATION>
where
    S: AsRef<str>,
    T: IntoIterator<Item = S>,
    U: AsRef<str>,
{
    let mut cmd = argv_to_cmd(argv);
    let mut env = create_env(env);
    let creation_flags =
        CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED;
    let working_dir = working_dir.map_or(ptr::null(), |dir| to_utf16(dir.as_ref()).as_ptr());
    let user_token = user.as_ref().map(|u| u.token.0);

    let mut inherited_handles = [stdio.stdin.0, stdio.stdout.0, stdio.stderr.0];
    let mut startup_info = StartupInfo::create(
        &stdio,
        &mut inherited_handles,
        user.map(|u| &mut u.desktop_name),
        show_window,
    )?;

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
                /*lpStartupInfo=*/ mem::transmute(&mut startup_info.base),
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
                /*lpStartupInfo=*/ mem::transmute(&mut startup_info.base),
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
