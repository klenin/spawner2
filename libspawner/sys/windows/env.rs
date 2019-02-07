use crate::Result;
use command::{EnvKind, EnvVar};
use std::collections::HashMap;
use std::env;
use std::mem;
use std::ptr;
use std::slice;
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
        EnvKind::Clear => HashMap::new(),
        EnvKind::Inherit => env::vars().collect(),
        EnvKind::UserDefault => user_default_env()?,
    };
    for var in vars {
        env.insert(var.name.clone(), var.val.clone());
    }

    let mut result: Vec<u16> = env
        .iter()
        .map(|(name, val)| to_utf16(format!("{}={}", name, val)))
        .flatten()
        .chain(std::iter::once(0))
        .collect();
    if result.len() == 1 {
        result.push(0);
    }
    Ok(result)
}

fn create_env_block<'a>() -> Result<&'a mut [u16]> {
    unsafe {
        let mut env_block: *mut u16 = ptr::null_mut();
        ok_nonzero(CreateEnvironmentBlock(
            mem::transmute(&mut env_block),
            ptr::null_mut(), // todo: user
            FALSE,
        ))?;

        let mut i = 0;
        while *env_block.offset(i) != 0 && *env_block.offset(i + 1) != 0 {
            i += 1;
        }

        Ok(slice::from_raw_parts_mut(env_block, i as usize))
    }
}

fn destroy_env_block(block: &mut [u16]) {
    unsafe {
        DestroyEnvironmentBlock(mem::transmute(block.as_mut_ptr()));
    }
}

fn user_default_env() -> Result<HashMap<String, String>> {
    let env_block = create_env_block()?;
    let mut result: HashMap<String, String> = HashMap::new();
    for var in env_block.split(|c| *c == 0) {
        let nameval = String::from_utf16_lossy(var);
        if let Some(idx) = nameval.find('=') {
            result.insert(nameval[0..idx].to_string(), nameval[idx + 1..].to_string());
        }
    }
    destroy_env_block(env_block);
    Ok(result)
}
