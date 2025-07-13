use crate::LegacyCli;
use anyhow::{Context, Result};
use nix::sched::{CloneFlags, unshare};

pub fn create_namespaces(cli: &LegacyCli) -> Result<()> {
    println!("Creating namespaces...");

    // Mount namespace (for filesystem isolation)
    unshare(CloneFlags::CLONE_NEWNS).context("Failed to create mount namespace")?;

    // UTS namespace (for hostname isolation)
    unshare(CloneFlags::CLONE_NEWUTS).context("Failed to create UTS namespace")?;

    // IPC namespace
    unshare(CloneFlags::CLONE_NEWIPC).context("Failed to create IPC namespace")?;

    // Network namespace (optional)
    if !cli.allow_network {
        unshare(CloneFlags::CLONE_NEWNET).context("Failed to create network namespace")?;
        println!("Network isolated");
    }

    // PID namespace (for process isolation) - temporarily disabled due to bash fork issues
    // The PID namespace should be created by the outer unshare command, not here
    // unshare(CloneFlags::CLONE_NEWPID).context("Failed to create PID namespace")?;
    println!("PID namespace creation skipped (should be handled by outer unshare)");

    println!("All namespaces created");
    Ok(())
}
