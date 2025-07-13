use crate::LegacyCli;
use anyhow::{Context, Result};
use nix::unistd::execvp;
use std::ffi::{CStr, CString};

pub fn exec_command(command: &str, args: &[String], cli: &LegacyCli) -> Result<()> {
    println!("Executing: {} {:?}", command, args);

    // Switch to non-root user if --user flag is specified
    if cli.user {
        let (username, uid, gid) = crate::container::user::get_default_user();
        crate::container::user::switch_user(username, uid, gid)?;
    }

    // For interactive bash, change to /home/user directory
    if command == "/bin/bash" && args.len() == 1 && args[0] == "-i" {
        std::env::set_current_dir("/home/user")
            .context("Failed to change to /home/user directory")?;
    }

    let command_c = CString::new(command).context("Invalid command")?;
    let mut args_c: Vec<CString> = vec![command_c.clone()];

    for arg in args {
        args_c.push(CString::new(arg.as_bytes()).context("Invalid argument")?);
    }

    let args_c_ref: Vec<&CStr> = args_c.iter().map(|c| c.as_c_str()).collect();

    execvp(&command_c, &args_c_ref).with_context(|| format!("Failed to execute: {}", command))?;

    Ok(())
}
