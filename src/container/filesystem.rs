use crate::{
    LegacyCli,
    registry::{BindMount, ContainerRegistry},
};
use anyhow::{Context, Result};
use nix::mount::{MsFlags, mount};
use nix::unistd::{chdir, chroot};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub fn setup_container(cli: &LegacyCli, container_id: Option<&str>) -> Result<()> {
    println!("Setting up container filesystem...");

    // Make root mount private to avoid affecting host
    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .context("Failed to make root private")?;

    // Create container root - either in registry or temporary
    let container_root = if let Some(id) = container_id {
        // Persistent container in registry
        let registry = ContainerRegistry::load()?;
        let container_dir = registry.get_container_dir(id)?;
        fs::create_dir_all(&container_dir)?;
        container_dir.join("rootfs")
    } else {
        // Temporary container
        PathBuf::from(format!("/tmp/container_{}", std::process::id()))
    };

    fs::create_dir_all(&container_root)?;

    let container_root_str = container_root
        .to_str()
        .context("Invalid container root path")?;

    // For persistent containers, don't use tmpfs - use actual directories with overlays
    // For temporary containers, still use tmpfs
    if container_id.is_none() {
        mount(
            Some("tmpfs"),
            container_root_str,
            Some("tmpfs"),
            MsFlags::empty(),
            None::<&str>,
        )
        .context("Failed to mount container tmpfs")?;
    }

    // Set up basic directory structure
    create_dirs(container_root_str)?;

    // Mount essential binary for the command
    mount_command_binary(&cli.command, container_root_str)?;

    // Set up overlay filesystem for container-created files
    let overlay_id = container_id.unwrap_or("temp");
    setup_container_overlay(container_root_str, overlay_id)?;

    // Set up bind mounts
    setup_bind_mounts(container_root_str, cli, container_id)?;

    // Set up user if --user flag is specified
    // For persistent containers, user is created during container creation
    // For temporary containers, create user on-the-fly
    if cli.user && container_id.is_none() {
        // Only create user for temporary containers
        setup_container_user(container_root_str)?;
    }

    // Chroot into container
    chroot(container_root_str).context("Failed to chroot")?;
    chdir("/").context("Failed to chdir to /")?;

    println!("Container filesystem ready");
    Ok(())
}

fn create_dirs(root: &str) -> Result<()> {
    let dirs = [
        "bin",
        "lib",
        "lib64",
        "usr",
        "usr/bin",
        "usr/lib",
        "usr/share",
        "tmp",
        "proc",
        "dev",
        "etc",
        "var",
        "home",
        "root",
        "opt",
        "srv",
        "mnt",
        "media",
        "run",
        "sys",
    ];

    for dir in &dirs {
        fs::create_dir_all(format!("{}/{}", root, dir)).ok();
    }

    // Create common user directories including config/cache/local
    let user_dirs = [
        "home/user",
        "home/user/.config",
        "home/user/.local",
        "home/user/.local/share",
        "home/user/.local/bin",
        "home/user/.cache",
        "home/user/.ssh",
        "home/user/Desktop",
        "home/user/Documents",
        "home/user/Downloads",
        "home/user/Pictures",
        "home/user/Videos",
        "home/user/Music",
    ];

    for dir in &user_dirs {
        fs::create_dir_all(format!("{}/{}", root, dir)).ok();
    }

    // Create essential files for better Linux emulation
    create_essential_files(root)?;

    Ok(())
}

fn create_essential_files(root: &str) -> Result<()> {
    // Mount essential files from host if they exist, otherwise create minimal versions
    // Note: We always create fallback passwd/group files since we may need to modify them for user creation
    let essential_files = ["/etc/hosts", "/etc/resolv.conf"];
    for file_path in &essential_files {
        if std::path::Path::new(file_path).exists() {
            match mount_single_file(file_path, root) {
                Ok(_) => println!("Mounted: {}", file_path),
                Err(_) => {
                    // Fallback to creating minimal versions
                    create_fallback_file(file_path, root);
                }
            }
        } else {
            // Create minimal versions if host files don't exist
            create_fallback_file(file_path, root);
        }
    }

    // Always create fallback passwd and group files so we can modify them
    create_fallback_file("/etc/passwd", root);
    create_fallback_file("/etc/group", root);

    // Create a basic terminfo entry for common terminals
    fs::create_dir_all(format!("{}/usr/share/terminfo/x", root)).ok();
    fs::create_dir_all(format!("{}/usr/share/terminfo/s", root)).ok();
    fs::create_dir_all(format!("{}/usr/share/terminfo/l", root)).ok();

    // Try to copy some essential terminfo entries from the host
    let terminfo_entries = [
        ("x/xterm", "/usr/share/terminfo/x/xterm"),
        ("x/xterm-256color", "/usr/share/terminfo/x/xterm-256color"),
        ("s/screen", "/usr/share/terminfo/s/screen"),
        ("l/linux", "/usr/share/terminfo/l/linux"),
    ];

    for (entry, host_path) in &terminfo_entries {
        if std::path::Path::new(host_path).exists() {
            if let Ok(content) = fs::read(host_path) {
                let target_path = format!("{}/usr/share/terminfo/{}", root, entry);
                if let Some(parent) = std::path::Path::new(&target_path).parent() {
                    fs::create_dir_all(parent).ok();
                }
                fs::write(target_path, content).ok();
            }
        }
    }

    Ok(())
}

fn create_fallback_file(file_path: &str, root: &str) {
    match file_path {
        "/etc/passwd" => {
            let passwd_content = "root:x:0:0:root:/root:/bin/bash\nnobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n";
            fs::write(format!("{}/etc/passwd", root), passwd_content).ok();
        }
        "/etc/group" => {
            let group_content = "root:x:0:\nnogroup:x:65534:\n";
            fs::write(format!("{}/etc/group", root), group_content).ok();
        }
        "/etc/hosts" => {
            let hosts_content =
                "127.0.0.1\tlocalhost\n::1\t\tlocalhost ip6-localhost ip6-loopback\n";
            fs::write(format!("{}/etc/hosts", root), hosts_content).ok();
        }
        "/etc/resolv.conf" => {
            let resolv_content = "nameserver 8.8.8.8\nnameserver 8.8.4.4\n";
            fs::write(format!("{}/etc/resolv.conf", root), resolv_content).ok();
        }
        _ => {}
    }
}

fn mount_command_binary(command: &str, container_root: &str) -> Result<()> {
    println!("Mounting: {}", command);

    // For /bin/bash, we need to mount essential directories
    if command == "/bin/bash" || command == "bash" {
        mount_essential_dirs(container_root)?;
        return Ok(());
    }

    // Resolve the command path using PATH if needed
    let resolved_command = resolve_command_path(command)?;
    let command_path = std::path::Path::new(&resolved_command);
    if !command_path.exists() {
        return Err(anyhow::anyhow!("Command not found: {}", command));
    }

    // Show what dependencies this command needs
    println!("Dependencies mounted for: {}", resolved_command);
    show_dependencies(&resolved_command)?;

    // Skip dependency mounting - we already mount essential lib directories
    // mount_dependencies(command, container_root)?;

    // Mount essential directories to ensure execution works
    println!("Mounting essential directories for reliable execution");
    mount_essential_dirs(container_root)?;

    Ok(())
}

fn mount_essential_dirs(container_root: &str) -> Result<()> {
    let essential_dirs = [
        "/bin",
        "/usr/bin",
        "/lib",
        "/lib64",
        "/usr/lib",
        "/usr/share/terminfo", // Terminal database for clear, tput, etc.
        "/etc",                // System configuration including SSL certs
    ];

    // Also mount user's .config directory as read-only if it exists
    if let Ok(home) = std::env::var("HOME") {
        let config_dir = format!("{}/.config", home);
        if std::path::Path::new(&config_dir).exists() {
            let target = format!("{}/home/user/.config", container_root);
            
            // Create target directory
            if let Some(parent) = std::path::Path::new(&target).parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::create_dir_all(&target).ok();
            
            // Mount the config directory
            match mount(
                Some(config_dir.as_str()),
                target.as_str(),
                None::<&str>,
                MsFlags::MS_BIND | MsFlags::MS_REC,
                None::<&str>,
            ) {
                Ok(_) => {
                    // Then remount as read-only
                    match mount(
                        None::<&str>,
                        target.as_str(),
                        None::<&str>,
                        MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
                        None::<&str>,
                    ) {
                        Ok(_) => println!("Mounted read-only: ~/.config -> /home/user/.config"),
                        Err(e) => println!("Warning: Failed to remount ~/.config as read-only: {}", e),
                    }
                }
                Err(e) => println!("Warning: Failed to mount ~/.config: {}", e),
            }
        }
    }

    for dir in &essential_dirs {
        if std::path::Path::new(dir).exists() {
            let target = format!("{}{}", container_root, dir);
            
            // Create target directory before mounting
            fs::create_dir_all(&target).ok();
            
            // First, bind mount the directory
            match mount(
                Some(*dir),
                target.as_str(),
                None::<&str>,
                MsFlags::MS_BIND | MsFlags::MS_REC,
                None::<&str>,
            ) {
                Ok(_) => {
                    // Don't remount /etc as read-only - may need to modify some configs
                    if *dir == "/etc" {
                        println!("Mounted: {}", dir);
                    } else {
                        // Then remount as read-only for security (other directories)
                        match mount(
                            None::<&str>,
                            target.as_str(),
                            None::<&str>,
                            MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
                            None::<&str>,
                        ) {
                            Ok(_) => println!("Mounted read-only: {}", dir),
                            Err(e) => {
                                println!("Warning: Failed to remount {} as read-only - {}", dir, e)
                            }
                        }
                    }
                }
                Err(e) => println!("Warning: Failed to mount {} - {}", dir, e),
            }
        } else {
            println!("Skipping non-existent directory: {}", dir);
        }
    }

    Ok(())
}

fn mount_single_file(file_path: &str, container_root: &str) -> Result<()> {
    let target = format!("{}{}", container_root, file_path);

    // Create parent directory
    if let Some(parent) = std::path::Path::new(&target).parent() {
        fs::create_dir_all(parent)?;
    }

    // For files, we need to create an empty file first, then bind mount over it
    if std::path::Path::new(file_path).is_file() {
        // Touch the file
        std::fs::File::create(&target)
            .with_context(|| format!("Failed to create target file {}", target))?;

        // Bind mount the file
        mount(
            Some(file_path),
            target.as_str(),
            None::<&str>,
            MsFlags::MS_BIND,
            None::<&str>,
        )
        .with_context(|| format!("Failed to bind mount file {}", file_path))?;
    } else {
        return Err(anyhow::anyhow!("Source is not a file: {}", file_path));
    }

    Ok(())
}

fn show_dependencies(command: &str) -> Result<()> {
    // Use ldd to find and display dependencies
    let output = std::process::Command::new("ldd")
        .arg(command)
        .output()
        .context("Failed to run ldd")?;

    if !output.status.success() {
        println!("  -> Static binary (no dynamic dependencies)");
        return Ok(());
    }

    let ldd_output = String::from_utf8_lossy(&output.stdout);

    for line in ldd_output.lines() {
        if let Some(lib_path) = parse_ldd_line(line) {
            if std::path::Path::new(&lib_path).exists() {
                println!("  -> {}", lib_path);
            } else {
                println!("  -> {} (not found)", lib_path);
            }
        }
    }

    Ok(())
}

fn parse_ldd_line(line: &str) -> Option<String> {
    // Handle different ldd output formats
    if line.contains(" => ") {
        // Format: "libname.so => /path/to/lib (0x...)"
        let parts: Vec<&str> = line.split(" => ").collect();
        if parts.len() >= 2 {
            let path_part = parts[1].trim();
            if let Some(space_pos) = path_part.find(" ") {
                return Some(path_part[..space_pos].to_string());
            }
        }
    } else if line.starts_with("\t/") {
        // Format: "\t/lib64/ld-linux-x86-64.so.2 (0x...)"
        let trimmed = line.trim();
        if let Some(space_pos) = trimmed.find(" ") {
            return Some(trimmed[..space_pos].to_string());
        }
    }

    None
}

fn setup_container_overlay(container_root: &str, container_id: &str) -> Result<()> {
    let home_dir = std::env::var("HOME").context("HOME environment variable not set")?;
    let container_data_dir = format!("{}/.local/containers/{}", home_dir, container_id);

    // For persistent containers, use a different approach
    if container_id != "temp" {
        setup_persistent_overlay(container_root, &container_data_dir)?;
        return Ok(());
    }

    // For temporary containers, use the old overlay approach
    let overlay_dirs = ["files", "work"];

    // Create container data directories
    for dir in &overlay_dirs {
        let dir_path = format!("{}/{}", container_data_dir, dir);
        fs::create_dir_all(&dir_path)
            .with_context(|| format!("Failed to create directory: {}", dir_path))?;
    }

    // Create writable overlay for directories where users commonly create files
    let writable_dirs = ["/tmp", "/var/tmp", "/home", "/root", "/opt"];

    for dir in &writable_dirs {
        let target = format!("{}{}", container_root, dir);
        let upper_dir = format!("{}/files{}", container_data_dir, dir);
        let work_dir = format!("{}/work{}", container_data_dir, dir);

        // Create directories
        fs::create_dir_all(&target)
            .with_context(|| format!("Failed to create target directory: {}", target))?;
        fs::create_dir_all(&upper_dir)
            .with_context(|| format!("Failed to create upper directory: {}", upper_dir))?;
        fs::create_dir_all(&work_dir)
            .with_context(|| format!("Failed to create work directory: {}", work_dir))?;

        // Create overlay mount
        let options = format!(
            "lowerdir={},upperdir={},workdir={}",
            dir, upper_dir, work_dir
        );
        match mount(
            Some("overlay"),
            target.as_str(),
            Some("overlay"),
            MsFlags::empty(),
            Some(options.as_str()),
        ) {
            Ok(_) => println!("Created writable overlay for: {} -> {}", dir, upper_dir),
            Err(_) => {
                // Overlay filesystem failed - this is expected in unprivileged containers
                // Fallback to tmpfs for /tmp, skip others silently
                if *dir == "/tmp" {
                    match mount(
                        Some("tmpfs"),
                        target.as_str(),
                        Some("tmpfs"),
                        MsFlags::empty(),
                        Some("size=100M"),
                    ) {
                        Ok(_) => println!("Created tmpfs for: {}", dir),
                        Err(e2) => println!("Warning: Failed to create writable space for {} - {}", dir, e2),
                    }
                }
                // For other directories (/var/tmp, /home, /root, /opt), we silently skip
                // since they're not critical and overlay failure is expected in unprivileged mode
            }
        }
    }

    Ok(())
}

fn setup_persistent_overlay(container_root: &str, container_data_dir: &str) -> Result<()> {
    // Create the container data directory
    fs::create_dir_all(container_data_dir).with_context(|| {
        format!(
            "Failed to create container data directory: {}",
            container_data_dir
        )
    })?;

    let files_dir = format!("{}/files", container_data_dir);
    fs::create_dir_all(&files_dir)
        .with_context(|| format!("Failed to create files directory: {}", files_dir))?;

    // For persistent containers, bind mount the persistent files directory as /home
    // This way files created in /home persist directly to disk
    let home_target = format!("{}/home", container_root);
    let persistent_home = format!("{}/home", files_dir);

    // Create the persistent home directory structure
    fs::create_dir_all(&persistent_home)?;
    fs::create_dir_all(format!("{}/user", persistent_home))?;

    // Create common user directories in the persistent location
    let user_dirs = [
        "user/Desktop",
        "user/Documents",
        "user/Downloads",
        "user/Pictures",
        "user/Videos",
        "user/Music",
    ];

    for dir in &user_dirs {
        fs::create_dir_all(format!("{}/{}", persistent_home, dir))?;
    }

    // Bind mount the persistent home
    match mount(
        Some(persistent_home.as_str()),
        home_target.as_str(),
        None::<&str>,
        MsFlags::MS_BIND,
        None::<&str>,
    ) {
        Ok(_) => println!(
            "Mounted persistent home: {} -> {}",
            persistent_home, home_target
        ),
        Err(e) => println!("Warning: Failed to mount persistent home: {}", e),
    }

    // Also handle /root directory for root user files
    let root_target = format!("{}/root", container_root);
    let persistent_root = format!("{}/root", files_dir);
    fs::create_dir_all(&persistent_root)?;

    match mount(
        Some(persistent_root.as_str()),
        root_target.as_str(),
        None::<&str>,
        MsFlags::MS_BIND,
        None::<&str>,
    ) {
        Ok(_) => println!(
            "Mounted persistent root: {} -> {}",
            persistent_root, root_target
        ),
        Err(e) => println!("Warning: Failed to mount persistent root: {}", e),
    }

    Ok(())
}

fn setup_bind_mounts(
    container_root: &str,
    cli: &LegacyCli,
    container_id: Option<&str>,
) -> Result<()> {
    let bind_mounts = if let Some(id) = container_id {
        // Get bind mounts from persistent container config
        let registry = ContainerRegistry::load()?;
        let container = registry
            .get_container(id)
            .ok_or_else(|| anyhow::anyhow!("Container not found: {}", id))?;
        container.config.bind_mounts.clone()
    } else {
        // Parse bind mounts from CLI for temporary container
        let mut mounts = Vec::new();
        for bind_str in &cli.bind {
            let (bind_mount, _is_auto_detected) = if bind_str.starts_with("__AUTO_DETECTED__:") {
                // This is an auto-detected path - don't create if missing
                let actual_bind_str = &bind_str["__AUTO_DETECTED__:".len()..];
                (BindMount::from_string_with_create_missing(actual_bind_str, false)
                    .with_context(|| format!("Invalid auto-detected bind mount: {}", actual_bind_str))?, true)
            } else {
                // This is a user-specified bind mount - create if missing
                (BindMount::from_string(bind_str)
                    .with_context(|| format!("Invalid bind mount: {}", bind_str))?, false)
            };

            // Expand ~ to home directory
            let expanded_host_path = if bind_mount.host_path.starts_with("~/") {
                let home = std::env::var("HOME").context("HOME environment variable not set")?;
                bind_mount.host_path.replacen("~", &home, 1)
            } else {
                bind_mount.host_path.clone()
            };

            let final_mount = BindMount {
                host_path: expanded_host_path,
                container_path: bind_mount.container_path,
                create_if_missing: bind_mount.create_if_missing,
            };


            mounts.push(final_mount);
        }
        mounts
    };

    // Apply each bind mount
    for bind_mount in bind_mounts {
        apply_bind_mount(container_root, &bind_mount)?;
    }

    Ok(())
}

fn apply_bind_mount(container_root: &str, bind_mount: &BindMount) -> Result<()> {
    let host_path = std::path::Path::new(&bind_mount.host_path);
    let container_path = bind_mount.container_path();
    let target_path = format!("{}{}", container_root, container_path);
    

    // Ensure host path exists if create_if_missing is true
    if bind_mount.create_if_missing && !host_path.exists() {
        if let Some(parent) = host_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create parent directories for {}",
                    bind_mount.host_path
                )
            })?;
        }

        if bind_mount.host_path.ends_with("/") || host_path.extension().is_none() {
            // Treat as directory
            fs::create_dir_all(&bind_mount.host_path)
                .with_context(|| format!("Failed to create directory {}", bind_mount.host_path))?;
        } else {
            // Treat as file
            fs::write(&bind_mount.host_path, "")
                .with_context(|| format!("Failed to create file {}", bind_mount.host_path))?;
        }
    }

    // Ensure target path exists in container
    if let Some(target_parent) = std::path::Path::new(&target_path).parent() {
        fs::create_dir_all(target_parent).with_context(|| {
            format!(
                "Failed to create container target parent: {}",
                target_parent.display()
            )
        })?;
    }

    if host_path.is_file() {
        // For files, create empty file then bind mount over it
        fs::write(&target_path, "")
            .with_context(|| format!("Failed to create target file: {}", target_path))?;
    } else {
        // For directories, just create the directory
        fs::create_dir_all(&target_path)
            .with_context(|| format!("Failed to create target directory: {}", target_path))?;
    }

    // Perform the bind mount
    match mount(
        Some(bind_mount.host_path.as_str()),
        target_path.as_str(),
        None::<&str>,
        MsFlags::MS_BIND,
        None::<&str>,
    ) {
        Ok(_) => println!(
            "Bind mounted: {} -> {}",
            bind_mount.host_path, container_path
        ),
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to bind mount {} to {}: {}",
                bind_mount.host_path,
                container_path,
                e
            ));
        }
    }

    Ok(())
}

fn setup_container_user(container_root: &str) -> Result<()> {
    let (username, uid, gid) = crate::container::user::get_default_user();

    // Create the user account
    crate::container::user::create_user(container_root, username, uid, gid)?;

    // Set up sudo/sudoers configuration
    setup_sudo_configuration(container_root, username)?;

    Ok(())
}

fn setup_sudo_configuration(container_root: &str, username: &str) -> Result<()> {
    // Create /etc/sudoers.d directory if it doesn't exist
    let sudoers_dir = format!("{}/etc/sudoers.d", container_root);
    fs::create_dir_all(&sudoers_dir).context("Failed to create /etc/sudoers.d directory")?;

    // Create sudoers entry for the user with NOPASSWD
    let sudoers_content = format!("{} ALL=(ALL) NOPASSWD:ALL\n", username);
    let sudoers_file = format!("{}/{}", sudoers_dir, username);
    fs::write(&sudoers_file, sudoers_content).context("Failed to create sudoers file")?;

    // Set proper permissions on sudoers file (0440)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&sudoers_file)?.permissions();
        perms.set_mode(0o440);
        fs::set_permissions(&sudoers_file, perms)?;
    }

    println!("Configured sudo access for user: {}", username);
    Ok(())
}

fn resolve_command_path(command: &str) -> Result<String> {
    // If the command is already an absolute path, use it as-is
    if command.starts_with('/') {
        return Ok(command.to_string());
    }
    
    // If the command contains a slash, treat it as a relative path
    if command.contains('/') {
        return Ok(command.to_string());
    }
    
    // For simple command names, use `which` to resolve the path
    let output = Command::new("which")
        .arg(command)
        .output()
        .context("Failed to execute 'which' command")?;
    
    if !output.status.success() {
        return Err(anyhow::anyhow!("Command '{}' not found in PATH", command));
    }
    
    let resolved_path = String::from_utf8(output.stdout)
        .context("Invalid UTF-8 in 'which' output")?
        .trim()
        .to_string();
    
    if resolved_path.is_empty() {
        return Err(anyhow::anyhow!("Command '{}' not found in PATH", command));
    }
    
    Ok(resolved_path)
}

