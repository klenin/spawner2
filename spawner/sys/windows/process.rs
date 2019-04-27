use crate::process::{Environment, ExitStatus, LimitViolation, ProcessInfo, ResourceUsage};
use crate::sys::limit_checker::LimitChecker;
use crate::sys::windows::common::{cvt, Handle};
use crate::sys::windows::pipe::{ReadPipe, WritePipe};
use crate::sys::windows::utils::{CreateProcessOptions, EnvBlock, ProcessInformation, Stdio, User};
use crate::sys::IntoInner;
use crate::{Error, Result};

use winapi::shared::minwindef::DWORD;
use winapi::um::jobapi2::{
    AssignProcessToJobObject, CreateJobObjectW, QueryInformationJobObject, TerminateJobObject,
};
use winapi::um::processthreadsapi::{
    GetExitCodeProcess, ResumeThread, SuspendThread, TerminateProcess,
};
use winapi::um::securitybaseapi::{ImpersonateLoggedOnUser, RevertToSelf};
use winapi::um::winnt::{
    JobObjectBasicAndIoAccountingInformation, JobObjectExtendedLimitInformation, HANDLE,
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
    job: Handle,
    user: Option<User>,
    creation_time: Instant,
    limit_checker: LimitChecker,
}

unsafe impl Send for Process {}

struct UserContext<'a>(&'a Option<User>);

impl Process {
    pub fn suspended<T, U>(info: T, stdio: U) -> Result<Self>
    where
        T: AsRef<ProcessInfo>,
        U: Into<ProcessStdio>,
    {
        let stdio = stdio.into();
        let info = info.as_ref();

        let ps = create_suspended_process(
            info,
            Stdio {
                stdin: stdio.stdin.into_inner(),
                stdout: stdio.stdout.into_inner(),
                stderr: stdio.stderr.into_inner(),
            },
        )?;

        let job = match assign_process_to_new_job(ps.base.hProcess) {
            Ok(x) => x,
            Err(e) => {
                unsafe {
                    TerminateProcess(ps.base.hProcess, 0);
                }
                return Err(e);
            }
        };

        Ok(Process {
            handle: Handle(ps.base.hProcess),
            main_thread: Handle(ps.base.hThread),
            job: job,
            user: ps.user,
            creation_time: Instant::now(),
            limit_checker: LimitChecker::new(info.resource_limits),
        })
    }

    pub fn exit_status(&self) -> Result<Option<ExitStatus>> {
        let basic_and_io_info = self.basic_and_io_info()?;
        if basic_and_io_info.BasicInfo.ActiveProcesses == 0 {
            let mut exit_code: DWORD = 0;
            unsafe {
                cvt(GetExitCodeProcess(self.handle.0, &mut exit_code))?;
            }
            if let Some(cause) = crash_cause(exit_code) {
                Ok(Some(ExitStatus::Crashed(cause.to_string())))
            } else {
                Ok(Some(ExitStatus::Finished(exit_code)))
            }
        } else {
            Ok(None)
        }
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

    pub fn reset_time_usage(&mut self) -> Result<()> {
        let zero = self.resource_usage()?;
        self.limit_checker
            .reset_timers(zero.wall_clock_time, zero.total_user_time);
        Ok(())
    }

    pub fn check_limits(&mut self) -> Result<Option<LimitViolation>> {
        self.resource_usage()
            .map(|usage| self.limit_checker.check(usage))
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
            total_bytes_written: basic_and_io_info.IoInfo.WriteTransferCount,
        })
    }

    pub fn terminate(&self) -> Result<()> {
        cvt(unsafe { TerminateJobObject(self.job.0, 0) })?;
        Ok(())
    }

    fn basic_and_io_info(&self) -> Result<JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION> {
        unsafe {
            let mut basic_and_io_info: JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION =
                mem::zeroed();

            cvt(QueryInformationJobObject(
                /*hJob=*/ self.job.0,
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
                /*hJob=*/ self.job.0,
                /*JobObjectInfoClass=*/ JobObjectExtendedLimitInformation,
                /*lpJobObjectInfo=*/ mem::transmute(&mut ext_info),
                /*cbJobObjectInfoLength=*/ mem::size_of_val(&ext_info) as DWORD,
                /*lpReturnLength=*/ ptr::null_mut(),
            ))
            .map(|_| ext_info)
        }
    }
}

impl<'a> UserContext<'a> {
    fn enter(user: &'a Option<User>) -> Result<Self> {
        if let Some(u) = user {
            unsafe {
                cvt(ImpersonateLoggedOnUser(u.token.0))?;
            }
        }
        Ok(Self(user))
    }
}

impl<'a> Drop for UserContext<'a> {
    fn drop(&mut self) {
        if self.0.is_some() {
            unsafe {
                RevertToSelf();
            }
        }
    }
}

fn create_suspended_process(info: &ProcessInfo, stdio: Stdio) -> Result<ProcessInformation> {
    let mut opts = CreateProcessOptions::new(
        std::iter::once(info.app.as_str()).chain(info.args.iter().map(|a| a.as_str())),
        stdio,
    );
    opts.show_window(info.show_window)
        .create_suspended(true)
        .hide_errors(true);
    if let Some(ref dir) = info.working_directory {
        opts.working_directory(dir);
    }

    let user = info
        .username
        .as_ref()
        .map(|uname| User::create(uname, info.password.as_ref()))
        .transpose()?;

    match info.env {
        Environment::Inherit => {
            opts.envs(std::env::vars());
        }
        Environment::Clear => {
            opts.env_clear();
        }
        Environment::UserDefault => {
            let block = EnvBlock::create(&user)?;
            for var in block.iter() {
                if let Some(idx) = var.find('=') {
                    opts.env(var[0..idx].to_string(), var[idx + 1..].to_string());
                }
            }
        }
    }

    opts.envs(info.env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())));

    if let Some(u) = user {
        opts.user(u);
    }

    opts.create()
}

fn assign_process_to_new_job(process: HANDLE) -> Result<Handle> {
    unsafe {
        let job = Handle(cvt(CreateJobObjectW(ptr::null_mut(), ptr::null()))?);
        cvt(AssignProcessToJobObject(job.0, process)).map(|_| job)
    }
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
