use crate::process::{Group, ProcessInfo};
use crate::sys::{AsInnerMut, IntoInner};
use crate::Result;

use winapi::shared::minwindef::DWORD;
use winapi::um::winnt::{
    JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
};

/// https://docs.microsoft.com/en-us/windows/desktop/api/winnt/ns-winnt-_jobobject_basic_ui_restrictions
pub struct UiRestrictions(DWORD);

pub trait GroupExt {
    fn set_ui_restrictions<T>(&mut self, r: T) -> Result<()>
    where
        T: Into<UiRestrictions>;
}

pub trait ProcessInfoExt {
    fn show_window(&mut self, show: bool) -> &mut Self;
    fn env_user(&mut self) -> &mut Self;
}

impl UiRestrictions {
    pub fn new() -> Self {
        Self(0)
    }

    pub fn limit_desktop(mut self) -> Self {
        self.0 |= JOB_OBJECT_UILIMIT_DESKTOP;
        self
    }

    pub fn limit_display_settings(mut self) -> Self {
        self.0 |= JOB_OBJECT_UILIMIT_DISPLAYSETTINGS;
        self
    }

    pub fn limit_exit_windows(mut self) -> Self {
        self.0 |= JOB_OBJECT_UILIMIT_EXITWINDOWS;
        self
    }

    pub fn limit_global_atoms(mut self) -> Self {
        self.0 |= JOB_OBJECT_UILIMIT_GLOBALATOMS;
        self
    }

    pub fn limit_handles(mut self) -> Self {
        self.0 |= JOB_OBJECT_UILIMIT_HANDLES;
        self
    }

    pub fn limit_read_clipboard(mut self) -> Self {
        self.0 |= JOB_OBJECT_UILIMIT_READCLIPBOARD;
        self
    }

    pub fn limit_write_clipboard(mut self) -> Self {
        self.0 |= JOB_OBJECT_UILIMIT_WRITECLIPBOARD;
        self
    }

    pub fn limit_system_parameters(mut self) -> Self {
        self.0 |= JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS;
        self
    }
}

impl IntoInner<DWORD> for UiRestrictions {
    fn into_inner(self) -> DWORD {
        self.0
    }
}

impl ProcessInfoExt for ProcessInfo {
    fn show_window(&mut self, show: bool) -> &mut Self {
        self.as_inner_mut().show_window(show);
        self
    }

    fn env_user(&mut self) -> &mut Self {
        self.as_inner_mut().env_user();
        self
    }
}

impl GroupExt for Group {
    fn set_ui_restrictions<T>(&mut self, r: T) -> Result<()>
    where
        T: Into<UiRestrictions>,
    {
        self.as_inner_mut().set_ui_restrictions(r)
    }
}
