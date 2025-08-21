mod execution;
mod filesystem;
mod namespaces;
pub mod user;

use crate::{LegacyCli, registry::ContainerConfig};
use anyhow::{Context, Result};
use std::process::Command;

pub fn run_container(command: &str, args: &[String], cli: &LegacyCli) -> Result<()> {
    println!("Creating unprivileged container...");

    // Set up cleanup for temporary containers on exit
    let temp_container_path = format!("/tmp/container_{}", std::process::id());
    let cleanup_path = temp_container_path.clone();
    std::panic::set_hook(Box::new(move |_| {
        let _ = std::fs::remove_dir_all(&cleanup_path);
    }));

    // Get current executable path before unshare (since /proc/self/exe won't be available after)
    let current_exe = std::env::current_exe()
        .context("Failed to get current executable path")?
        .to_str()
        .context("Invalid executable path")?
        .to_string();


    // Use unshare command to set up user namespace with mapping
    let mut unshare_cmd = Command::new("unshare");

    if cli.user {
        // For --user flag: Map a range that includes both UID 0 and UID 1000
        let host_uid = unsafe { nix::libc::getuid() };
        let host_gid = unsafe { nix::libc::getgid() };

        unshare_cmd.args(&[
            "--user",
            "--map-users",
            &format!("0:{}:1", host_uid),
            "--map-users",
            &format!("1000:100000:1"),
            "--map-groups",
            &format!("0:{}:1", host_gid),
            "--map-groups",
            &format!("1000:100000:1"),
            "--pid",
            "--fork",
            "--",
            &current_exe,
            "--internal-container-init",
            command,
        ]);
    } else {
        // Normal case: Map current user as root for full capabilities
        unshare_cmd.args(&[
            "--user",
            "--map-root-user",
            "--pid",
            "--fork",
            "--",
            &current_exe,
            "--internal-container-init",
            command,
        ]);
    }

    // Add args
    for arg in args {
        unshare_cmd.arg(arg);
    }

    // Add CLI flags
    if cli.allow_network {
        unshare_cmd.arg("--allow-network");
    }

    if cli.user {
        unshare_cmd.arg("--user");
    }

    // Add bind mounts
    for bind_mount in &cli.bind {
        unshare_cmd.arg("--bind");
        unshare_cmd.arg(bind_mount);
    }


    let status = unshare_cmd
        .status()
        .context("Failed to run container setup")?;

    if !status.success() {
        anyhow::bail!("Container failed with status: {}", status);
    }

    // Clean up temporary container directory
    if std::path::Path::new(&temp_container_path).exists() {
        std::fs::remove_dir_all(&temp_container_path).ok();
    }

    // Also cleanup any temporary containers from registry
    if let Ok(mut registry) = crate::registry::ContainerRegistry::load() {
        registry.cleanup_temporary().ok();
        registry.save().ok();
    }

    Ok(())
}

// This function runs inside the container after unshare --map-root-user
pub fn init_container(
    command: &str,
    args: &[String],
    cli: &LegacyCli,
    container_id: Option<&str>,
) -> Result<()> {
    println!("Initializing container environment...");

    // We're now root inside the user namespace
    println!("Running as root inside user namespace");

    // Create additional namespaces
    namespaces::create_namespaces(cli).context("Failed to create namespaces")?;

    // Set up container filesystem
    filesystem::setup_container(cli, container_id)
        .context("Failed to setup container filesystem")?;

    // Set container hostname
    nix::unistd::sethostname("kakuri").context("Failed to set hostname")?;

    // Execute the command
    execution::exec_command(command, args, cli).context("Failed to execute command")?;

    Ok(())
}

pub fn start_persistent_container(
    container_id: &str,
    command: &str,
    args: &[String],
    config: &ContainerConfig,
) -> Result<u32> {
    println!("Starting persistent container: {}", container_id);

    // Convert ContainerConfig to LegacyCli for compatibility

    // Get current executable path before unshare (since /proc/self/exe won't be available after)
    let current_exe = std::env::current_exe()
        .context("Failed to get current executable path")?
        .to_str()
        .context("Invalid executable path")?
        .to_string();

    // Use unshare command to set up user namespace with mapping
    let mut unshare_cmd = Command::new("unshare");
    unshare_cmd.args(&[
        "--user",
        "--map-root-user",
        "--pid",
        "--fork",
        "--",
        &current_exe,
        "--internal-container-init",
        command,
    ]);

    // Add args
    for arg in args {
        unshare_cmd.arg(arg);
    }

    // Add CLI flags
    if config.allow_network {
        unshare_cmd.arg("--allow-network");
    }

    // Add bind mounts (for persistent containers, these come from the registry)
    for bind_mount in &config.bind_mounts {
        unshare_cmd.arg("--bind");
        unshare_cmd.arg(&bind_mount.host_path);
    }

    // Add container ID for persistent container handling
    unshare_cmd.arg("--container-id");
    unshare_cmd.arg(container_id);

    let child = unshare_cmd
        .spawn()
        .context("Failed to start persistent container")?;

    let pid = child.id();
    
    // Don't wait for the child - let it run independently
    // The PID will be tracked in the registry for later cleanup
    
    Ok(pid)
}

pub fn exec_in_container(
    container_id: &str,
    command: &str,
    args: &[String],
    config: &ContainerConfig,
) -> Result<()> {
    println!("Executing in container: {}", container_id);

    // Extract container name from container_id (remove the random suffix)
    let container_name = container_id.split('_').next().unwrap_or(container_id);

    // Create a modified command for bash with custom prompt
    let actual_command;
    let actual_args;

    if command == "/bin/bash" && args.is_empty() {
        // Create interactive bash session
        actual_command = "/bin/bash";
        actual_args = vec![
            "-i".to_string(), // Interactive mode
        ];
    } else {
        actual_command = command;
        actual_args = args.to_vec();
    }

    // Convert ContainerConfig to LegacyCli for compatibility

    // Get current executable path before unshare (since /proc/self/exe won't be available after)
    let current_exe = std::env::current_exe()
        .context("Failed to get current executable path")?
        .to_str()
        .context("Invalid executable path")?
        .to_string();

    // Use unshare command to set up user namespace with mapping
    let mut unshare_cmd = Command::new("unshare");
    unshare_cmd.args(&[
        "--user",
        "--map-root-user",
        "--pid",
        "--fork",
        "--",
        &current_exe,
        "--internal-container-init",
        actual_command,
    ]);

    // Add args
    for arg in &actual_args {
        unshare_cmd.arg(arg);
    }

    // Add CLI flags
    if config.allow_network {
        unshare_cmd.arg("--allow-network");
    }

    // Add bind mounts (for persistent containers, these come from the registry)
    for bind_mount in &config.bind_mounts {
        unshare_cmd.arg("--bind");
        unshare_cmd.arg(&bind_mount.host_path);
    }

    // Add container ID for persistent container handling
    unshare_cmd.arg("--container-id");
    unshare_cmd.arg(container_id);

    // Set up environment variables for the container
    unshare_cmd.env("CONTAINER_NAME", container_name);
    unshare_cmd.env("CONTAINER_ID", container_id);

    // For bash sessions, disable job control to prevent process cleanup issues
    if actual_command == "/bin/bash" && actual_args.len() == 1 && actual_args[0] == "-i" {
        unshare_cmd.env("BASH_EXECUTION_STRING", "set +m");
    }

    // Preserve terminal-related environment variables
    if let Ok(term) = std::env::var("TERM") {
        unshare_cmd.env("TERM", term);
    }
    if let Ok(terminfo) = std::env::var("TERMINFO") {
        unshare_cmd.env("TERMINFO", terminfo);
    }

    // If this is a bash session, set up custom prompt via environment
    if command == "/bin/bash" && args.is_empty() {
        // Set custom prompt and welcome message via environment
        let ps1 = format!(
            r"\[\033[1;34m\][{}]\[\033[0m\] \[\033[1;32m\]\w\[\033[0m\] ",
            container_name
        );
        unshare_cmd.env("PS1", ps1);

        // Set default directory to /home/user
        unshare_cmd.env("HOME", "/home/user");

        // We'll use PROMPT_COMMAND to show the welcome message once
        unshare_cmd.env(
            "PROMPT_COMMAND",
            format!(
                r#"if [ -z "$CONTAINER_WELCOMED" ]; then
    echo "Welcome to container: {}"
    echo "Container ID: {}"
    echo "Type 'exit' to leave the container"
    echo ""
    alias ll='ls -la'
    alias la='ls -A'
    alias l='ls -CF'
    export CONTAINER_WELCOMED=1
fi"#,
                container_name, container_id
            ),
        );
    }

    // Execute the command
    let status = unshare_cmd
        .status()
        .context("Failed to execute in container")?;

    if !status.success() {
        anyhow::bail!("Container exec failed with status: {}", status);
    }

    Ok(())
}
