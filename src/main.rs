use anyhow::Result;
use clap::Parser;

mod config;
mod container;
mod container_manager;
mod registry;

use container::{init_container, run_container};

fn handle_container_init() -> Result<()> {
    // This is the internal call after unshare
    // Parse raw args since we're bypassing clap
    let raw_args: Vec<String> = std::env::args().collect();

    // Find the position of --internal-container-init
    let init_pos = raw_args
        .iter()
        .position(|arg| arg == "--internal-container-init")
        .ok_or_else(|| anyhow::anyhow!("Could not find --internal-container-init in args"))?;

    if init_pos + 1 >= raw_args.len() {
        anyhow::bail!("Internal container init call missing command");
    }

    let command = &raw_args[init_pos + 1];
    let mut command_args = Vec::new();
    let mut allow_network = false;
    let mut container_id = None;
    let mut bind = Vec::new();
    let mut user = false;
    let mut i = init_pos + 2;

    // Parse remaining args, filtering out flags
    while i < raw_args.len() {
        match raw_args[i].as_str() {
            "--allow-network" => {
                allow_network = true;
                i += 1;
            }
            "--container-id" => {
                if i + 1 < raw_args.len() {
                    container_id = Some(raw_args[i + 1].clone());
                    i += 2;
                } else {
                    anyhow::bail!("--container-id requires a value");
                }
            }
            "--bind" => {
                if i + 1 < raw_args.len() {
                    bind.push(raw_args[i + 1].clone());
                    i += 2;
                } else {
                    anyhow::bail!("--bind requires a value");
                }
            }
            "--user" => {
                user = true;
                i += 1;
            }
            _ => {
                command_args.push(raw_args[i].clone());
                i += 1;
            }
        }
    }

    let legacy_cli = LegacyCli {
        command: command.clone(),
        args: command_args.clone(),
        allow_network,
        bind,
        user,
    };

    init_container(command, &command_args, &legacy_cli, container_id.as_deref())
}

fn should_use_direct_execution(raw_args: &[String]) -> bool {
    if raw_args.len() < 2 {
        return false;
    }

    let known_subcommands = [
        "run", "create", "start", "exec", "shell", "list", "stop", "remove",
    ];
    let first_non_flag_arg = raw_args
        .iter()
        .skip(1)
        .find(|arg| !arg.starts_with("-"))
        .map(|s| s.as_str());

    // If the first non-flag argument is not a known subcommand, treat as direct execution
    match first_non_flag_arg {
        Some(arg) => !known_subcommands.contains(&arg),
        None => false,
    }
}

fn handle_direct_execution(raw_args: &[String]) -> Result<()> {
    // Parse container options and separate command + args
    let mut command = None;
    let mut command_args = Vec::new();
    let mut allow_network = false;
    let mut bind = Vec::new();
    let mut user = false;
    let mut i = 1;

    // Parse container options first
    while i < raw_args.len() {
        match raw_args[i].as_str() {
            "--allow-network" => {
                allow_network = true;
                i += 1;
            }
            "--bind" => {
                if i + 1 < raw_args.len() {
                    bind.push(raw_args[i + 1].clone());
                    i += 2;
                } else {
                    anyhow::bail!("--bind requires a value");
                }
            }
            "--user" => {
                user = true;
                i += 1;
            }
            "--" => {
                i += 1;
                if i < raw_args.len() && command.is_none() {
                    command = Some(raw_args[i].clone());
                    i += 1;
                }
                command_args.extend_from_slice(&raw_args[i..]);
                break;
            }
            arg if arg.starts_with("-") => {
                i += 1;
            }
            _ => {
                command = Some(raw_args[i].clone());
                i += 1;
                command_args.extend_from_slice(&raw_args[i..]);
                break;
            }
        }
    }

    let actual_command = command.unwrap_or_else(|| "/bin/bash".to_string());

    // Auto-detect and add paths from command arguments
    let mut auto_bind = detect_paths_in_args(&actual_command, &command_args);
    bind.append(&mut auto_bind);

    let legacy_cli = LegacyCli {
        command: actual_command.clone(),
        args: command_args.clone(),
        allow_network,
        bind,
        user,
    };

    run_container(&actual_command, &command_args, &legacy_cli)
}

#[derive(Parser, Debug, Clone)]
#[command(name = "kakuri")]
#[command(about = "Unprivileged container runtime")]
struct Cli {
    #[arg(long, hide = true)]
    internal_stage2: bool,

    #[arg(long, hide = true)]
    container_id: Option<String>,

    /// Command to run in container (if no subcommand provided)
    command: Option<String>,

    /// Arguments for the command (use -- to separate from container options)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,

    /// Allow network access
    #[arg(long)]
    allow_network: bool,

    /// Bind mount directories into container (format: host_path:container_path or just path for same location)
    #[arg(long, value_name = "PATH[:PATH]")]
    bind: Vec<String>,

    /// Use a predefined bind profile from config (e.g., "dev", "minimal")
    #[arg(long, value_name = "PROFILE")]
    bind_profile: Option<String>,


    /// Run as non-root user in container (username: user, password: root)
    #[arg(long)]
    user: bool,

    #[command(subcommand)]
    subcommand: Option<Commands>,
}

#[derive(clap::Subcommand, Debug, Clone)]
enum Commands {
    /// Run a command directly in a new container (legacy mode)
    Run {
        command: Option<String>,

        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,

        #[arg(long)]
        allow_network: bool,

        #[arg(long, value_name = "PATH[:PATH]")]
        bind: Vec<String>,

        #[arg(long, value_name = "PROFILE")]
        bind_profile: Option<String>,


        #[arg(long)]
        user: bool,
    },

    /// Create a new container
    Create {
        name: String,

        #[arg(long)]
        init: bool,

        #[arg(long)]
        allow_network: bool,

        #[arg(long, value_name = "PATH[:PATH]")]
        bind: Vec<String>,

        #[arg(long, value_name = "PROFILE")]
        bind_profile: Option<String>,

    },

    /// Start a container
    Start {
        name: String,

        #[arg(trailing_var_arg = true)]
        command: Vec<String>,

    },

    /// Execute a command in a running container
    Exec {
        name: String,

        #[arg(required = true)]
        command: String,

        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// Open an interactive shell in a container
    Shell { name: String },

    /// List containers
    List,

    /// Stop a container
    Stop { name: String },

    /// Remove a container
    Remove {
        name: String,

        #[arg(long)]
        force: bool,
    },

}


fn main() -> Result<()> {
    // Check for internal stage2 before clap parsing
    let args: Vec<String> = std::env::args().collect();
    if args.contains(&"--internal-container-init".to_string()) {
        return handle_container_init();
    }

    // Handle direct command execution (non-subcommand mode)
    // If args don't start with known subcommands, parse as direct execution
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.len() > 1 && should_use_direct_execution(&raw_args) {
        return handle_direct_execution(&raw_args);
    }

    let cli = Cli::parse();

    match cli.subcommand {
        None => {
            let actual_command = cli.command.unwrap_or_else(|| "/bin/bash".to_string());
            let mut final_binds = merge_bind_mounts(cli.bind.clone(), cli.bind_profile.clone())?;
            
            // Auto-detect and add paths from command arguments
            let mut auto_bind = detect_paths_in_args(&actual_command, &cli.args);
            final_binds.append(&mut auto_bind);
            
            let legacy_cli = LegacyCli {
                command: actual_command.clone(),
                args: cli.args.clone(),
                allow_network: cli.allow_network,
                bind: final_binds,
                user: cli.user,
            };
            run_container(&actual_command, &cli.args, &legacy_cli)
        }
        Some(Commands::Run {
            command,
            args,
            allow_network,
            bind,
            bind_profile,
            user,
        }) => {
            let actual_command = command.unwrap_or_else(|| "/bin/bash".to_string());
            let mut final_binds = merge_bind_mounts(bind, bind_profile)?;
            
            // Auto-detect and add paths from command arguments
            let mut auto_bind = detect_paths_in_args(&actual_command, &args);
            final_binds.append(&mut auto_bind);
            
            let legacy_cli = LegacyCli {
                command: actual_command.clone(),
                args: args.clone(),
                allow_network,
                bind: final_binds,
                user,
            };
            run_container(&actual_command, &args, &legacy_cli)
        }
        Some(Commands::Create {
            name,
            init,
            allow_network,
            bind,
            bind_profile,
        }) => {
            let final_binds = merge_bind_mounts(bind, bind_profile)?;
            container_manager::create_container(name, init, allow_network, final_binds)
        }
        Some(Commands::Start { name, command }) => {
            container_manager::start_container(name, command)
        }
        Some(Commands::Exec {
            name,
            command,
            args,
        }) => container_manager::exec_container(name, command, args),
        Some(Commands::Shell { name }) => container_manager::shell_container(name),
        Some(Commands::List) => container_manager::list_containers(),
        Some(Commands::Stop { name }) => container_manager::stop_container(name),
        Some(Commands::Remove { name, force }) => container_manager::remove_container(name, force),
    }
}

// Legacy CLI structure for backward compatibility
#[derive(Debug, Clone)]
struct LegacyCli {
    command: String,
    #[allow(dead_code)] // Used indirectly via cloning
    args: Vec<String>,
    allow_network: bool,
    bind: Vec<String>,
    user: bool,
}

fn merge_bind_mounts(bind: Vec<String>, bind_profile: Option<String>) -> Result<Vec<String>> {
    let mut final_binds = bind;

    if let Some(profile_name) = bind_profile {
        let config = crate::config::Config::load()?;
        if let Some(profiles) = &config.bind_profiles {
            if let Some(profile_binds) = profiles.get(&profile_name) {
                final_binds.extend(profile_binds.clone());
            } else {
                anyhow::bail!("Bind profile {} not found in config", profile_name);
            }
        } else {
            anyhow::bail!("No bind profiles configured");
        }
    }

    Ok(final_binds)
}

fn detect_paths_in_args(_command: &str, args: &[String]) -> Vec<String> {
    let mut detected_paths = Vec::new();
    
    // Only check arguments, not the command itself
    // The command (like /usr/bin/python3) is already available in the container
    for arg in args {
        if is_path_like(arg) && path_exists(arg) {
            // For auto-detected paths, we want to mount them as read-only
            // and we definitely don't want create_if_missing since they already exist
            let expanded_path = if arg.starts_with("~/") {
                if let Ok(home) = std::env::var("HOME") {
                    arg.replacen("~", &home, 1)
                } else {
                    arg.to_string()
                }
            } else {
                arg.to_string()
            };
            
            // Use a special prefix to mark auto-detected paths
            // This will help us identify them later and set create_if_missing: false
            detected_paths.push(format!("__AUTO_DETECTED__:{}:{}", expanded_path, expanded_path));
        }
    }
    
    // Remove duplicates while preserving order
    detected_paths.sort();
    detected_paths.dedup();
    
    if !detected_paths.is_empty() {
        println!("Auto-detected {} path(s) for mounting", detected_paths.len());
    }
    
    detected_paths
}

fn is_path_like(s: &str) -> bool {
    // Consider something a path if it:
    // 1. Starts with / (absolute path)
    // 2. Starts with ./ or ../ (relative path)
    // 3. Contains / and looks like a file path
    // 4. Starts with ~ (home directory)
    
    if s.is_empty() {
        return false;
    }
    
    // Absolute paths
    if s.starts_with('/') {
        return true;
    }
    
    // Home directory paths
    if s.starts_with('~') {
        return true;
    }
    
    // Relative paths
    if s.starts_with("./") || s.starts_with("../") {
        return true;
    }
    
    // Paths with directory separators that look like files
    if s.contains('/') {
        // Check if it has a reasonable file extension or looks like a directory
        if s.ends_with('/') {
            return true;
        }
        
        // Common file extensions that suggest this is a file path
        let file_extensions = [
            ".py", ".js", ".rs", ".c", ".cpp", ".h", ".hpp", ".java", ".go",
            ".txt", ".md", ".json", ".yaml", ".yml", ".toml", ".xml", ".html",
            ".css", ".sh", ".bash", ".conf", ".cfg", ".ini", ".log", ".csv",
            ".sql", ".dockerfile", ".docker", ".env", ".properties"
        ];
        
        for ext in &file_extensions {
            if s.to_lowercase().ends_with(ext) {
                return true;
            }
        }
        
        // If it contains a slash and has 2+ components, likely a path
        let components: Vec<&str> = s.split('/').collect();
        if components.len() >= 2 && !components.iter().any(|c| c.is_empty()) {
            return true;
        }
    }
    
    false
}

fn path_exists(path: &str) -> bool {
    // Expand ~ to home directory if needed
    let expanded_path = if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            path.replacen("~", &home, 1)
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    };
    
    std::path::Path::new(&expanded_path).exists()
}


