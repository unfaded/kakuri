use crate::registry::{BindMount, ContainerConfig, ContainerRegistry, ContainerStatus};
use anyhow::{Context, Result};
use std::fs;

pub fn create_container(
    name: String,
    init: bool,
    allow_network: bool,
    bind: Vec<String>,
) -> Result<()> {
    let mut registry = ContainerRegistry::load()?;

    // Check for existing containers with the same name
    let existing = registry.find_by_name(&name);
    if !existing.is_empty() {
        println!("Existing containers with name {}:", name);
        for container in existing {
            println!(
                "  {} ({})",
                container.full_id(),
                match container.status {
                    ContainerStatus::Created => "created",
                    ContainerStatus::Running => "running",
                    ContainerStatus::Stopped => "stopped",
                    ContainerStatus::Temporary => "temporary",
                }
            );
        }
        anyhow::bail!(
            "Container name {} already exists. Use a different name or remove existing containers.",
            name
        );
    }

    // Parse bind mounts
    let mut bind_mounts = Vec::new();
    for bind_str in bind {
        let bind_mount = BindMount::from_string(&bind_str)
            .with_context(|| format!("Invalid bind mount: {}", bind_str))?;

        // Expand ~ to home directory
        let expanded_host_path = if bind_mount.host_path.starts_with("~/") {
            let home = std::env::var("HOME").context("HOME environment variable not set")?;
            bind_mount.host_path.replacen("~", &home, 1)
        } else {
            bind_mount.host_path.clone()
        };

        // Create host directory if it does not exist and create_if_missing is true
        if bind_mount.create_if_missing {
            if let Some(parent) = std::path::Path::new(&expanded_host_path).parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Failed to create parent directories for {}",
                        expanded_host_path
                    )
                })?;
            }
            if !std::path::Path::new(&expanded_host_path).exists() {
                if expanded_host_path.ends_with("/")
                    || std::path::Path::new(&expanded_host_path)
                        .extension()
                        .is_none()
                {
                    // Treat as directory
                    fs::create_dir_all(&expanded_host_path).with_context(|| {
                        format!("Failed to create directory {}", expanded_host_path)
                    })?;
                } else {
                    // Treat as file
                    if let Some(parent) = std::path::Path::new(&expanded_host_path).parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&expanded_host_path, "")
                        .with_context(|| format!("Failed to create file {}", expanded_host_path))?;
                }
            }
        }

        let final_bind_mount = BindMount {
            host_path: expanded_host_path,
            container_path: bind_mount.container_path,
            create_if_missing: bind_mount.create_if_missing,
        };

        bind_mounts.push(final_bind_mount);
    }

    // Create container configuration
    let config = ContainerConfig {
        allow_network,
        init,
        command: None,
        args: vec![],
        bind_mounts,
    };

    // Add container to registry
    let container_id = registry.add_container(name.clone(), config, false)?;

    // Create container directory structure
    let container_dir = registry.get_container_dir(&container_id)?;
    fs::create_dir_all(&container_dir)?;

    // Create subdirectories
    fs::create_dir_all(container_dir.join("rootfs"))?;
    fs::create_dir_all(container_dir.join("logs"))?;

    // Create container config file
    let container_info = registry
        .get_container(&container_id)
        .ok_or_else(|| anyhow::anyhow!("Container disappeared after creation"))?;
    let config_content = serde_json::to_string_pretty(container_info)?;
    fs::write(container_dir.join("config.json"), config_content)?;

    println!("Created container: {}", container_id);
    Ok(())
}

pub fn list_containers() -> Result<()> {
    let registry = ContainerRegistry::load()?;

    if registry.containers.is_empty() {
        println!("No containers found.");
        return Ok(());
    }

    println!(
        "{:<20} {:<15} {:<10} {:<20}",
        "CONTAINER ID", "NAME", "STATUS", "CREATED"
    );
    println!("{}", "-".repeat(70));

    let mut containers: Vec<_> = registry.containers.values().collect();
    containers.sort_by(|a, b| b.created_at.cmp(&a.created_at)); // Sort by creation time, newest first

    for container in containers {
        if matches!(container.status, ContainerStatus::Temporary) {
            continue; // Skip temporary containers
        }

        let status = match container.status {
            ContainerStatus::Created => "created",
            ContainerStatus::Running => "running",
            ContainerStatus::Stopped => "stopped",
            ContainerStatus::Temporary => continue,
        };

        let created = format_timestamp(container.created_at);
        println!(
            "{:<20} {:<15} {:<10} {:<20}",
            container.full_id(),
            container.name,
            status,
            created
        );
    }

    Ok(())
}

pub fn start_container(name: String, command: Vec<String>) -> Result<()> {
    let mut registry = ContainerRegistry::load()?;

    // Find container by name
    let containers = registry.find_by_name(&name);
    let container_id = match containers.len() {
        0 => anyhow::bail!("No container found with name {}", name),
        1 => containers[0].full_id(),
        _ => {
            println!("Multiple containers found with name {}:", name);
            for container in containers {
                println!(
                    "  {} ({})",
                    container.full_id(),
                    match container.status {
                        ContainerStatus::Created => "created",
                        ContainerStatus::Running => "running",
                        ContainerStatus::Stopped => "stopped",
                        ContainerStatus::Temporary => "temporary",
                    }
                );
            }
            anyhow::bail!("Please specify the full container ID instead of name");
        }
    };

    // Get container info
    let container = registry
        .get_container_mut(&container_id)
        .ok_or_else(|| anyhow::anyhow!("Container not found: {}", container_id))?;

    // Check if already running
    if matches!(container.status, ContainerStatus::Running) {
        anyhow::bail!("Container {} is already running", container_id);
    }

    // Determine command to run
    let actual_command = if command.is_empty() {
        "/bin/bash".to_string()
    } else {
        command[0].clone()
    };
    let args = if command.is_empty() {
        vec![]
    } else {
        command[1..].to_vec()
    };

    // Clone the config before modifying the container
    let config = container.config.clone();

    // Update container status and command
    container.status = ContainerStatus::Running;
    container.started_at = Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    );
    container.config.command = Some(actual_command.clone());
    container.config.args = args.clone();

    // Save registry
    registry.save()?;

    println!(
        "Starting container {} with command: {} {:?}",
        container_id, actual_command, args
    );

    // Start the container using the existing container system
    // We need to modify the container module to support persistent containers
    use crate::container::start_persistent_container;
    start_persistent_container(&container_id, &actual_command, &args, &config)
}

pub fn stop_container(name: String) -> Result<()> {
    let mut registry = ContainerRegistry::load()?;

    // Find container by name
    let containers = registry.find_by_name(&name);
    let container_id = match containers.len() {
        0 => anyhow::bail!("No container found with name {}", name),
        1 => containers[0].full_id(),
        _ => {
            println!("Multiple containers found with name {}:", name);
            for container in containers {
                println!(
                    "  {} ({})",
                    container.full_id(),
                    match container.status {
                        ContainerStatus::Created => "created",
                        ContainerStatus::Running => "running",
                        ContainerStatus::Stopped => "stopped",
                        ContainerStatus::Temporary => "temporary",
                    }
                );
            }
            anyhow::bail!("Please specify the full container ID instead of name");
        }
    };

    // Get container info
    let container = registry
        .get_container_mut(&container_id)
        .ok_or_else(|| anyhow::anyhow!("Container not found: {}", container_id))?;

    // Check if running
    if !matches!(container.status, ContainerStatus::Running) {
        anyhow::bail!("Container {} is not running", container_id);
    }

    // TODO: Actually stop the running process (need PID tracking)
    println!("Stopping container: {}", container_id);

    // Update status
    container.status = ContainerStatus::Stopped;
    container.pid = None;

    // Save registry
    registry.save()?;

    println!("Container {} stopped", container_id);
    Ok(())
}

pub fn remove_container(name: String, force: bool) -> Result<()> {
    let mut registry = ContainerRegistry::load()?;

    // Find container by name
    let containers = registry.find_by_name(&name);
    let container_id = match containers.len() {
        0 => anyhow::bail!("No container found with name {}", name),
        1 => containers[0].full_id(),
        _ => {
            println!("Multiple containers found with name {}:", name);
            for container in containers {
                println!(
                    "  {} ({})",
                    container.full_id(),
                    match container.status {
                        ContainerStatus::Created => "created",
                        ContainerStatus::Running => "running",
                        ContainerStatus::Stopped => "stopped",
                        ContainerStatus::Temporary => "temporary",
                    }
                );
            }
            anyhow::bail!("Please specify the full container ID instead of name");
        }
    };

    // Get container info
    let container = registry
        .get_container(&container_id)
        .ok_or_else(|| anyhow::anyhow!("Container not found: {}", container_id))?;

    // Check if running (unless force)
    if matches!(container.status, ContainerStatus::Running) && !force {
        anyhow::bail!(
            "Container {} is running. Stop it first or use --force",
            container_id
        );
    }

    // Remove container directory
    let container_dir = registry.get_container_dir(&container_id)?;
    if container_dir.exists() {
        fs::remove_dir_all(&container_dir).with_context(|| {
            format!("Failed to remove container directory: {:?}", container_dir)
        })?;
    }

    // Remove from registry
    registry.remove_container(&container_id)?;

    println!("Removed container: {}", container_id);
    Ok(())
}

pub fn exec_container(name: String, command: String, args: Vec<String>) -> Result<()> {
    let registry = ContainerRegistry::load()?;

    // Find container by name
    let containers = registry.find_by_name(&name);
    let container_id = match containers.len() {
        0 => anyhow::bail!("No container found with name {}", name),
        1 => containers[0].full_id(),
        _ => {
            println!("Multiple containers found with name {}:", name);
            for container in containers {
                println!(
                    "  {} ({})",
                    container.full_id(),
                    match container.status {
                        ContainerStatus::Created => "created",
                        ContainerStatus::Running => "running",
                        ContainerStatus::Stopped => "stopped",
                        ContainerStatus::Temporary => "temporary",
                    }
                );
            }
            anyhow::bail!("Please specify the full container ID instead of name");
        }
    };

    // Get container info
    let container = registry
        .get_container(&container_id)
        .ok_or_else(|| anyhow::anyhow!("Container not found: {}", container_id))?;

    // For now, since we do not have persistent running containers (they exit after start),
    // let us create a new interactive session in the container context
    println!("Entering container: {}", container_id);

    // Start a new session with the container filesystem and settings
    use crate::container::exec_in_container;
    exec_in_container(&container_id, &command, &args, &container.config)
}

pub fn shell_container(name: String) -> Result<()> {
    let registry = ContainerRegistry::load()?;

    // Find container by name
    let containers = registry.find_by_name(&name);
    let container_id = match containers.len() {
        0 => anyhow::bail!("No container found with name {}", name),
        1 => containers[0].full_id(),
        _ => {
            println!("Multiple containers found with name {}:", name);
            for container in containers {
                println!(
                    "  {} ({})",
                    container.full_id(),
                    match container.status {
                        ContainerStatus::Created => "created",
                        ContainerStatus::Running => "running",
                        ContainerStatus::Stopped => "stopped",
                        ContainerStatus::Temporary => "temporary",
                    }
                );
            }
            anyhow::bail!("Please specify the full container ID instead of name");
        }
    };

    // Get container info
    let container = registry
        .get_container(&container_id)
        .ok_or_else(|| anyhow::anyhow!("Container not found: {}", container_id))?;

    println!("Opening shell in container: {}", container_id);

    // Start an interactive bash session with custom prompt
    use crate::container::exec_in_container;
    exec_in_container(&container_id, "/bin/bash", &[], &container.config)
}

fn format_timestamp(timestamp: u64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let diff = now.saturating_sub(timestamp);

    if diff < 60 {
        format!("{}s ago", diff)
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}
