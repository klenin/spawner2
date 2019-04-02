use crate::command::{Command, EnvKind};
use crate::sys::windows::common::{cvt, Handle};
use crate::sys::windows::utils::{
    CreateProcessOptions, EnvBlock, ProcessInformation, Stdio, ThreadIterator, User,
};
use crate::{Error, Result};

use winapi::shared::minwindef::{DWORD, FALSE};
use winapi::um::jobapi2::{
    AssignProcessToJobObject, CreateJobObjectW, QueryInformationJobObject, TerminateJobObject,
};
use winapi::um::minwinbase::STILL_ACTIVE;
use winapi::um::processthreadsapi::{
    GetExitCodeProcess, OpenThread, ResumeThread, SuspendThread, TerminateProcess,
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
    STATUS_REG_NAT_CONSUMPTION, STATUS_SINGLE_STEP, STATUS_STACK_OVERFLOW, THREAD_SUSPEND_RESUME,
};

use std::mem;
use std::ptr;
use std::u32;

pub struct Process {
    handle: Handle,
    id: DWORD,
    job: Handle,
    user: Option<User>,
}

pub enum Status {
    Running,
    Finished(u32),
    Crashed(u32, &'static str),
}

struct UserContext<'a>(&'a Option<User>);

impl Process {
    pub fn spawn_suspended(cmd: &Command, stdio: Stdio) -> Result<Self> {
        let info = create_suspended(cmd, stdio)?;
        drop(Handle(info.base.hThread));

        let job = match assign_process_to_new_job(info.base.hProcess) {
            Ok(x) => x,
            Err(e) => {
                unsafe {
                    TerminateProcess(info.base.hProcess, 0);
                }
                return Err(e);
            }
        };

        Ok(Process {
            handle: Handle(info.base.hProcess),
            id: info.base.dwProcessId,
            job: job,
            user: info.user,
        })
    }

    pub fn status(&self) -> Result<Status> {
        let mut exit_code: DWORD = 0;
        unsafe {
            cvt(GetExitCodeProcess(self.handle.0, &mut exit_code))?;
        }
        Ok(match exit_code {
            STILL_ACTIVE => Status::Running,
            _ => {
                if let Some(cause) = crash_cause(exit_code) {
                    Status::Crashed(exit_code, cause)
                } else {
                    Status::Finished(exit_code)
                }
            }
        })
    }

    pub fn suspend(&self) -> Result<()> {
        let _ctx = UserContext::enter(&self.user);
        for id in ThreadIterator::new(self.id) {
            unsafe {
                let handle = Handle(cvt(OpenThread(THREAD_SUSPEND_RESUME, FALSE, id))?);
                if SuspendThread(handle.0) == u32::MAX {
                    return Err(Error::last_os_error());
                }
            }
        }
        Ok(())
    }

    pub fn resume(&self) -> Result<()> {
        let _ctx = UserContext::enter(&self.user);
        for id in ThreadIterator::new(self.id) {
            unsafe {
                let handle = Handle(cvt(OpenThread(THREAD_SUSPEND_RESUME, FALSE, id))?);
                if ResumeThread(handle.0) == u32::MAX {
                    return Err(Error::last_os_error());
                }
            }
        }
        Ok(())
    }

    pub fn basic_and_io_info(&self) -> Result<JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION> {
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

    pub fn ext_limit_info(&self) -> Result<JOBOBJECT_EXTENDED_LIMIT_INFORMATION> {
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

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            TerminateJobObject(self.job.0, 0);
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

fn create_suspended(cmd: &Command, stdio: Stdio) -> Result<ProcessInformation> {
    let mut opts = CreateProcessOptions::new(
        std::iter::once(cmd.app.as_str()).chain(cmd.args.iter().map(|a| a.as_str())),
        stdio,
    );
    opts.show_window(cmd.show_window)
        .create_suspended(true)
        .hide_errors(true);
    if let Some(ref dir) = cmd.working_directory {
        opts.working_directory(dir);
    }

    let user = match cmd.username {
        Some(ref name) => Some(User::create(name, cmd.password.as_ref())?),
        None => None,
    };

    match cmd.env_kind {
        EnvKind::Inherit => {
            opts.envs(std::env::vars());
        }
        EnvKind::Clear => {
            opts.env_clear();
        }
        EnvKind::UserDefault => {
            let block = EnvBlock::create(&user)?;
            for var in block.iter() {
                if let Some(idx) = var.find('=') {
                    opts.env(var[0..idx].to_string(), var[idx + 1..].to_string());
                }
            }
        }
    }

    opts.envs(
        cmd.env_vars
            .iter()
            .map(|v| (v.name.as_str(), v.val.as_str())),
    );

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
