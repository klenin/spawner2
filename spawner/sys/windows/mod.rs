#[allow(clippy::cast_ptr_alignment)]
mod helpers;

mod missing_decls {
    use winapi::shared::basetsd::DWORD_PTR;
    pub const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: DWORD_PTR = 131_074;
}

pub mod error;
pub mod pipe;
pub mod pipe_ext;
pub mod process;
pub mod process_ext;
