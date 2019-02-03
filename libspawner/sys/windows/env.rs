use crate::Result;
use command::{EnvKind, EnvVar};
use std::env;
use std::mem;
use std::ptr;
use sys::windows::common::ok_nonzero;
use sys::windows::common::to_utf16;
use winapi::shared::minwindef::FALSE;
use winapi::um::userenv::{CreateEnvironmentBlock, DestroyEnvironmentBlock};

pub fn create(kind: EnvKind, vars: &Vec<EnvVar>) -> Result<Vec<u16>> {
    // https://docs.microsoft.com/en-us/windows/desktop/api/processthreadsapi/nf-processthreadsapi-createprocessa
    //
    // An environment block consists of a null-terminated block of null-terminated strings.
    // Each string is in the following form:
    //     name=value\0
    //
    // A Unicode environment block is terminated by four zero bytes: two for the last string,
    // two more to terminate the block.
    let mut env = match kind {
        EnvKind::Clear => Vec::new(),
        EnvKind::Inherit => current_env(),
        EnvKind::UserDefault => user_default_env()?,
    };
    for var in vars {
        env.extend(to_utf16(format!("{}={}", var.name, var.val)));
    }
    if env.len() == 0 {
        env.push(0);
    }
    env.push(0);
    Ok(env)
}

fn current_env() -> Vec<u16> {
    env::vars()
        .map(|(name, val)| to_utf16(format!("{}={}", name, val)))
        .flatten()
        .collect()
}

fn user_default_env() -> Result<Vec<u16>> {
    let mut result: Vec<u16> = Vec::new();

    unsafe {
        let mut env_block: *mut u16 = ptr::null_mut();
        ok_nonzero(CreateEnvironmentBlock(
            mem::transmute(&mut env_block),
            ptr::null_mut(), // todo: user
            FALSE,
        ))?;

        let mut it = env_block;
        loop {
            result.push(*it);
            if *it == 0 && *it.offset(1) == 0 {
                break;
            }
            it = it.offset(1);
        }
        DestroyEnvironmentBlock(mem::transmute(env_block));
    }

    Ok(result)
}
