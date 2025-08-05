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
#[command(name = "container")]
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
            let final_binds = merge_bind_mounts(cli.bind.clone(), cli.bind_profile.clone())?;
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
            let final_binds = merge_bind_mounts(bind, bind_profile)?;
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

