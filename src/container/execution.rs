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

    // For interactive bash, set up custom prompt and environment AFTER user switch
    let setup_bash_env = command == "/bin/bash" && (args.is_empty() || (args.len() == 1 && args[0] == "-i"));
    
    if setup_bash_env {
        std::env::set_current_dir("/home/user")
            .context("Failed to change to /home/user directory")?;
        
        // Set up custom prompt and environment variables
        // SAFETY: We are setting environment variables in a controlled container environment
        // before exec, which is safe in this context
        unsafe {
            let ps1 = "\\[\\033[1;34m\\][container]\\[\\033[0m\\] \\[\\033[1;32m\\]\\w\\[\\033[0m\\] $ ";
            std::env::set_var("PS1", ps1);
            
            // Re-set environment variables after user switch (switch_user may have overridden them)
            std::env::set_var("HOME", "/home/user");
            
            // Set up welcome message via PROMPT_COMMAND
            std::env::set_var(
                "PROMPT_COMMAND",
                r#"if [ -z "$CONTAINER_WELCOMED" ]; then
    echo "Welcome to Kakuri container bash"
    echo ""
    alias ll='ls -la'
    alias la='ls -A'
    alias l='ls -CF'
    export CONTAINER_WELCOMED=1
fi"#,
            );
        }
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
