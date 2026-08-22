use crate::policy::{FsAccess, FsRule};
use anyhow::{Context, Result};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const CREATE_RULESET_VERSION: u32 = 1;
const RULE_PATH_BENEATH: i32 = 1;

const ACCESS_EXECUTE: u64 = 1 << 0;
const ACCESS_WRITE_FILE: u64 = 1 << 1;
const ACCESS_READ_FILE: u64 = 1 << 2;
const ACCESS_READ_DIR: u64 = 1 << 3;
const ACCESS_REMOVE_DIR: u64 = 1 << 4;
const ACCESS_REMOVE_FILE: u64 = 1 << 5;
const ACCESS_MAKE_CHAR: u64 = 1 << 6;
const ACCESS_MAKE_DIR: u64 = 1 << 7;
const ACCESS_MAKE_REG: u64 = 1 << 8;
const ACCESS_MAKE_SOCK: u64 = 1 << 9;
const ACCESS_MAKE_FIFO: u64 = 1 << 10;
const ACCESS_MAKE_BLOCK: u64 = 1 << 11;
const ACCESS_MAKE_SYM: u64 = 1 << 12;

const READ_ACCESS: u64 = ACCESS_READ_FILE | ACCESS_READ_DIR;
const WRITE_ACCESS: u64 = ACCESS_WRITE_FILE
    | ACCESS_REMOVE_DIR
    | ACCESS_REMOVE_FILE
    | ACCESS_MAKE_CHAR
    | ACCESS_MAKE_DIR
    | ACCESS_MAKE_REG
    | ACCESS_MAKE_SOCK
    | ACCESS_MAKE_FIFO
    | ACCESS_MAKE_BLOCK
    | ACCESS_MAKE_SYM;
const HANDLED_ACCESS: u64 = ACCESS_EXECUTE | READ_ACCESS | WRITE_ACCESS;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

pub fn enforce(rules: &[FsRule]) -> Result<()> {
    if abi_version().is_none() {
        return Err(std::io::Error::last_os_error())
            .context("Landlock ABI is unavailable; refusing policy-mode execution");
    }

    let attr = RulesetAttr {
        handled_access_fs: HANDLED_ACCESS,
    };
    let ruleset_fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attr,
            std::mem::size_of::<RulesetAttr>(),
            0u32,
        )
    } as i32;
    if ruleset_fd < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to create Landlock ruleset");
    }

    let result = add_baseline_rules(ruleset_fd)
        .and_then(|()| {
            for rule in rules {
                let access = match rule.access {
                    FsAccess::Read => READ_ACCESS,
                    FsAccess::ReadExecute => READ_ACCESS | ACCESS_EXECUTE,
                    FsAccess::ReadWrite => READ_ACCESS | WRITE_ACCESS,
                    FsAccess::ReadWriteExecute => READ_ACCESS | WRITE_ACCESS | ACCESS_EXECUTE,
                };
                add_path_rule(ruleset_fd, &rule.path, access, true)?;
            }
            Ok(())
        })
        .and_then(|()| {
            let status = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
            if status != 0 {
                return Err(std::io::Error::last_os_error()).context("failed to set no_new_privs");
            }
            let status =
                unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset_fd, 0u32) };
            if status != 0 {
                return Err(std::io::Error::last_os_error())
                    .context("failed to enforce Landlock ruleset");
            }
            Ok(())
        });
    unsafe {
        libc::close(ruleset_fd);
    }
    result
}

pub fn abi_version() -> Option<u32> {
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    };
    (abi >= 1).then_some(abi as u32)
}

fn add_baseline_rules(ruleset_fd: i32) -> Result<()> {
    for path in ["/bin", "/sbin", "/usr", "/lib", "/lib64"] {
        add_path_rule(
            ruleset_fd,
            Path::new(path),
            READ_ACCESS | ACCESS_EXECUTE,
            false,
        )?;
    }
    for path in ["/dev", "/tmp"] {
        add_path_rule(
            ruleset_fd,
            Path::new(path),
            READ_ACCESS | WRITE_ACCESS,
            false,
        )?;
    }
    for path in ["/proc", "/sys"] {
        add_path_rule(ruleset_fd, Path::new(path), READ_ACCESS, false)?;
    }
    Ok(())
}

fn add_path_rule(ruleset_fd: i32, path: &Path, allowed_access: u64, required: bool) -> Result<()> {
    if !path.exists() {
        if required {
            anyhow::bail!("Landlock policy path does not exist: {}", path.display());
        }
        return Ok(());
    }
    let path_c = CString::new(path.as_os_str().as_bytes())?;
    let parent_fd = unsafe { libc::open(path_c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if parent_fd < 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to open Landlock path {}", path.display()));
    }
    let attr = PathBeneathAttr {
        allowed_access,
        parent_fd,
    };
    let status = unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset_fd,
            RULE_PATH_BENEATH,
            &attr,
            0u32,
        )
    };
    unsafe {
        libc::close(parent_fd);
    }
    if status != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to add Landlock rule for {}", path.display()));
    }
    Ok(())
}
