use crate::sys::windows::common::{cvt, to_utf16, Handle};
use crate::{Error, Result};

use winapi::shared::basetsd::{DWORD_PTR, SIZE_T};
use winapi::shared::minwindef::{DWORD, FALSE, TRUE, WORD};
use winapi::um::errhandlingapi::SetErrorMode;
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use winapi::um::processthreadsapi::{
    CreateProcessAsUserW, CreateProcessW, DeleteProcThreadAttributeList,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_LIST,
};
use winapi::um::tlhelp32::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use winapi::um::userenv::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use winapi::um::winbase::{
    LogonUserW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT,
    LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT, SEM_FAILCRITICALERRORS,
    SEM_NOGPFAULTERRORBOX, STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};
use winapi::um::winnt::{HANDLE, PVOID};
use winapi::um::winuser::{SW_HIDE, SW_SHOW};

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::collections::HashMap;
use std::fmt::{self, Write};
use std::mem;
use std::ptr;

pub struct Stdio {
    pub stdin: Handle,
    pub stdout: Handle,
    pub stderr: Handle,
}

pub struct User {
    pub handle: Handle,
}

pub struct CreateProcessOptions {
    cmd: Vec<u16>,
    stdio: Stdio,
    show_window: bool,
    create_suspended: bool,
    hide_errors: bool,
    working_directory: Option<Vec<u16>>,
    user: Option<User>,
    env: HashMap<String, String>,
}

pub struct ProcessInformation {
    pub base: PROCESS_INFORMATION,
    pub user: Option<User>,
}

pub struct EnvBlock {
    block: *mut u16,
    len: usize,
}

pub struct ThreadIterator {
    process_id: DWORD,
    end_reached: bool,
    snapshot: Option<ThreadSnapshot>,
}

struct ThreadSnapshot {
    handle: Handle,
    entry: THREADENTRY32,
}

struct StartupInfo {
    base: STARTUPINFOEXW,
    _att_list: AttList,
}

struct AttList {
    ptr: *mut PROC_THREAD_ATTRIBUTE_LIST,
    len: usize,
}

impl User {
    pub fn logon<T, U>(user: T, password: Option<U>) -> Result<Self>
    where
        T: AsRef<str>,
        U: AsRef<str>,
    {
        let mut token = INVALID_HANDLE_VALUE;
        let pwd = match password {
            Some(p) => to_utf16(p.as_ref()),
            None => to_utf16(""),
        };

        unsafe {
            cvt(LogonUserW(
                /*lpUsername=*/ to_utf16(user.as_ref()).as_ptr(),
                /*lpDomain=*/ to_utf16(".").as_ptr(),
                /*lpPassword=*/ pwd.as_ptr(),
                /*dwLogonType=*/ LOGON32_LOGON_INTERACTIVE,
                /*dwLogonProvider=*/ LOGON32_PROVIDER_DEFAULT,
                /*phToken=*/ &mut token,
            ))?;
        }

        Ok(Self {
            handle: Handle(token),
        })
    }
}

impl CreateProcessOptions {
    pub fn new<T, U>(argv: T, stdio: Stdio) -> Self
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>,
    {
        Self {
            cmd: argv_to_cmd(argv),
            stdio: stdio,
            show_window: false,
            create_suspended: false,
            hide_errors: false,
            working_directory: None,
            user: None,
            env: HashMap::new(),
        }
    }

    pub fn hide_errors(&mut self, hide: bool) -> &mut Self {
        self.hide_errors = hide;
        self
    }

    pub fn show_window(&mut self, show: bool) -> &mut Self {
        self.show_window = show;
        self
    }

    pub fn create_suspended(&mut self, suspended: bool) -> &mut Self {
        self.create_suspended = suspended;
        self
    }

    pub fn user(&mut self, u: User) -> &mut Self {
        self.user = Some(u);
        self
    }

    pub fn env_clear(&mut self) -> &mut Self {
        self.env.clear();
        self
    }

    pub fn env<K, V>(&mut self, k: K, v: V) -> &mut Self
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.env
            .insert(k.as_ref().to_string(), v.as_ref().to_string());
        self
    }

    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for (k, v) in vars.into_iter() {
            self.env(k, v);
        }
        self
    }

    pub fn working_directory<S: AsRef<str>>(&mut self, dir: S) -> &mut Self {
        self.working_directory = Some(to_utf16(dir.as_ref()));
        self
    }

    pub fn create(mut self) -> Result<ProcessInformation> {
        let cmd = self.cmd.as_mut_ptr();
        let creation_flags = self.creation_flags();
        let mut env = self.create_env();
        let working_dir = self
            .working_directory
            .map_or(ptr::null(), |dir| dir.as_ptr());

        let mut inherited_handles = [self.stdio.stdin.0, self.stdio.stdout.0, self.stdio.stderr.0];
        let mut startup_info =
            StartupInfo::create(&self.stdio, &mut inherited_handles, self.show_window)?;

        let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };

        unsafe {
            if self.hide_errors {
                SetErrorMode(SEM_NOGPFAULTERRORBOX | SEM_FAILCRITICALERRORS);
            }

            let result = if let Some(ref user) = self.user {
                CreateProcessAsUserW(
                    /*hToken=*/ user.handle.0,
                    /*lpApplicationName=*/ ptr::null(),
                    /*lpCommandLine=*/ cmd,
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
                    /*lpCommandLine=*/ cmd,
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

        Ok(ProcessInformation {
            base: process_info,
            user: self.user,
        })
    }

    fn creation_flags(&self) -> DWORD {
        let mut flags = CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT;
        if self.create_suspended {
            flags |= CREATE_SUSPENDED;
        }
        flags
    }

    fn create_env(&self) -> Vec<u16> {
        // https://docs.microsoft.com/en-us/windows/desktop/api/processthreadsapi/nf-processthreadsapi-createprocessa
        //
        // An environment block consists of a null-terminated block of null-terminated strings.
        // Each string is in the following form:
        //     name=value\0
        //
        // A Unicode environment block is terminated by four zero bytes: two for the last string,
        // two more to terminate the block.
        let mut result: Vec<u16> = self
            .env
            .iter()
            .map(|(name, val)| to_utf16(format!("{}={}", name, val)))
            .flatten()
            .chain(std::iter::once(0))
            .collect();
        if result.len() == 1 {
            result.push(0);
        }
        result
    }
}

impl EnvBlock {
    pub fn create(user: &Option<User>) -> Result<Self> {
        let mut block: *mut u16 = ptr::null_mut();
        let mut len = 0;
        unsafe {
            cvt(CreateEnvironmentBlock(
                mem::transmute(&mut block),
                match user {
                    Some(u) => u.handle.0,
                    None => ptr::null_mut(),
                },
                FALSE,
            ))?;

            while *block.offset(len) != 0 && *block.offset(len + 1) != 0 {
                len += 1;
            }
        }

        Ok(Self {
            block: block,
            len: len as usize,
        })
    }

    pub fn as_slice(&self) -> &[u16] {
        unsafe { std::slice::from_raw_parts(self.block, self.len) }
    }

    pub fn iter<'a>(&'a self) -> impl Iterator<Item = String> + 'a {
        self.as_slice()
            .split(|c| *c == 0)
            .map(String::from_utf16_lossy)
    }
}

impl Drop for EnvBlock {
    fn drop(&mut self) {
        unsafe {
            DestroyEnvironmentBlock(mem::transmute(self.block));
        }
    }
}

impl ThreadIterator {
    pub fn new(process_id: DWORD) -> Self {
        Self {
            process_id: process_id,
            end_reached: false,
            snapshot: None,
        }
    }
}

impl Iterator for ThreadIterator {
    type Item = DWORD;

    fn next(&mut self) -> Option<Self::Item> {
        if self.snapshot.is_none() {
            self.snapshot = ThreadSnapshot::create();
        }

        if self.snapshot.is_none() || self.end_reached {
            return None;
        }

        let snapshot = self.snapshot.as_mut().unwrap();
        let mut result: Option<Self::Item> = None;
        while result.is_none() {
            if snapshot.entry.th32OwnerProcessID == self.process_id {
                result = Some(snapshot.entry.th32ThreadID);
            }
            if unsafe { Thread32Next(snapshot.handle.0, &mut snapshot.entry) } == FALSE {
                self.end_reached = true;
                break;
            }
        }
        result
    }
}

impl ThreadSnapshot {
    fn create() -> Option<Self> {
        unsafe {
            let mut entry: THREADENTRY32 = mem::zeroed();
            entry.dwSize = mem::size_of_val(&entry) as DWORD;
            let handle = match CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) {
                INVALID_HANDLE_VALUE => return None,
                x => Handle(x),
            };
            if Thread32First(handle.0, &mut entry) == FALSE {
                return None;
            }
            Some(ThreadSnapshot {
                entry: entry,
                handle: handle,
            })
        }
    }
}

impl StartupInfo {
    fn create(stdio: &Stdio, inherited_handles: &mut [HANDLE], show_window: bool) -> Result<Self> {
        // Unfortunately, winapi-rs does not define this.
        const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: DWORD_PTR = 131074;

        let mut att_list = AttList::allocate(1)?;
        unsafe {
            att_list.update(
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                mem::transmute(inherited_handles.as_mut_ptr()),
                inherited_handles.len() * mem::size_of::<HANDLE>(),
            )?;
        }

        let mut info: STARTUPINFOEXW = unsafe { mem::zeroed() };
        info.lpAttributeList = att_list.ptr;
        info.StartupInfo.cb = mem::size_of_val(&info) as DWORD;
        info.StartupInfo.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
        info.StartupInfo.lpDesktop = ptr::null_mut();
        info.StartupInfo.wShowWindow = if show_window { SW_SHOW } else { SW_HIDE } as WORD;
        info.StartupInfo.hStdInput = stdio.stdin.0;
        info.StartupInfo.hStdOutput = stdio.stdout.0;
        info.StartupInfo.hStdError = stdio.stderr.0;

        Ok(StartupInfo {
            base: info,
            _att_list: att_list,
        })
    }
}

impl AttList {
    fn allocate(attribs_count: DWORD) -> Result<Self> {
        unsafe {
            let mut len: SIZE_T = 0;
            InitializeProcThreadAttributeList(ptr::null_mut(), attribs_count, 0, &mut len);
            let ptr: *mut PROC_THREAD_ATTRIBUTE_LIST =
                mem::transmute(alloc_zeroed(Layout::from_size_align_unchecked(len, 4)));

            if ptr == ptr::null_mut() {
                return Err(Error::from(
                    "Cannot allocate memory for PROC_THREAD_ATTRIBUTE_LIST",
                ));
            }

            cvt(InitializeProcThreadAttributeList(
                /*lpAttributeList=*/ ptr,
                /*dwAttributeCount=*/ attribs_count,
                /*dwFlags=*/ 0,
                /*lpSize=*/ &mut len,
            ))?;

            Ok(Self { ptr: ptr, len: len })
        }
    }

    fn update(&mut self, attribute: DWORD_PTR, value: PVOID, size: SIZE_T) -> Result<()> {
        unsafe {
            cvt(UpdateProcThreadAttribute(
                /*lpAttributeList=*/ self.ptr,
                /*dwFlags=*/ 0,
                /*Attribute=*/ attribute,
                /*lpValue=*/ value,
                /*cbSize=*/ size,
                /*lpPreviousValue=*/ ptr::null_mut(),
                /*lpReturnSize=*/ ptr::null_mut(),
            ))?;
        }
        Ok(())
    }
}

impl Drop for AttList {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.ptr);
            dealloc(
                mem::transmute(self.ptr),
                Layout::from_size_align_unchecked(self.len, 4),
            );
        }
    }
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
