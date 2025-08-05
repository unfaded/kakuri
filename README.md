# Kakuri

Lightweight containerization built on Linux namespaces and filesystem isolation.

## Features

- Unprivileged containers using Linux namespaces
- Persistent and temporary container modes
- Filesystem isolation with overlay mounts
- Optional user namespace mapping
- Network isolation
- Automatic PATH resolution for commands
- Automatic path mounting for file arguments
- Bind mount profiles for development workflows

## Quick Start

```bash
# Direct execution (temporary container)
kakuri
kakuri python3 script.py

# Automatic path mounting - files/directories in arguments are auto-mounted
kakuri python3 /path/to/script.py
kakuri python3 ~/my_project/main.py

# Commands use PATH resolution - no need for full paths
kakuri python3 --version
kakuri node --version  
kakuri gcc --version

# Container management
kakuri create mycontainer
kakuri start mycontainer bash
kakuri exec mycontainer ls /home
kakuri shell mycontainer
```

## Installation

```bash
cargo build --release
sudo cp target/release/kakuri /usr/local/bin/
```

## Usage

### Temporary Containers

Run commands directly in isolated containers:

```bash
# Basic isolation
kakuri bash

# With network access
kakuri --allow-network curl https://example.com

# With bind mounts
kakuri --bind ~/projects:/projects bash

# As non-root user
kakuri --user bash
```

### Persistent Containers

Create and manage long-lived containers:

```bash
# Create container
kakuri create --allow-network container_name

# Start with command
kakuri start container_name bash

# Execute in running container
kakuri exec container_name ls

# Interactive shell
kakuri shell container_name

# List containers
kakuri list

# Remove container
kakuri remove container_name
```

## Configuration

Default config location: "~/.config/container/config.toml"

```toml
[storage]
containers_dir = "~/.local/kakuri/containers"

[defaults]
allow_network = false

[bind_profiles]
dev = [
    "~/.config",
    "~/.local", 
    "~/.cache",
    "~/.ssh"
]
minimal = ["~/.cache"]
```

### Bind Profiles

Use predefined bind mount sets:

```bash
# Development profile
kakuri --bind-profile dev bash

# Custom binds + profile
kakuri --bind ~/src:/src --bind-profile minimal bash
```

### Automatic Path Mounting

Kakuri automatically detects file and directory paths in command arguments and mounts them into the container. This allows seamless access to files without explicitly specifying bind mounts.

```bash
# These paths are automatically mounted:
kakuri python3 /path/to/script.py          # /path/to/script.py mounted
kakuri node ~/project/app.js               # ~/project/app.js mounted  
kakuri gcc -o app ~/src/main.c ~/src/lib.c # ~/src/ mounted
kakuri vim /etc/hosts                      # /etc/hosts mounted

# Works with any file extensions and directory paths
kakuri python3 ~/data/process.py ~/output/results.txt
```

The automatic mounting:
- Only affects arguments, not the command itself
- Detects absolute paths (`/path/to/file`)
- Detects home directory paths (`~/file`) 
- Detects relative paths (`./file`, `../file`)
- Detects common file extensions

## Network Isolation

### Default Behavior
- No network access (complete isolation)
- Use "--allow-network" for host network access


## Security Model

Kakuri uses Linux user namespaces to provide unprivileged containerization:

- Root inside container maps to your user outside
- Filesystem access controlled via bind mounts
- Network isolation via network namespaces
- Process isolation via PID namespaces

## Container Lifecycle

1. Create namespace (user, mount, PID, network, UTS, IPC)
2. Set up container filesystem with overlays
3. Apply bind mounts and network configuration
4. Execute target command
5. Clean up temporary resources


## Troubleshooting

### Permission Issues
Ensure your user can create user namespaces:
```bash
sysctl kernel.unprivileged_userns_clone
```

If the value is 0, enable unprivileged user namespaces:
```bash
sudo sysctl kernel.unprivileged_userns_clone=1
```

To make it persistent across reboots, add to /etc/sysctl.conf:
```bash
echo 'kernel.unprivileged_userns_clone = 1' | sudo tee -a /etc/sysctl.conf
```

### Mount Failures
Check if overlay filesystem is supported:
```bash
mount -t overlay overlay /tmp/test
```

### Network Problems
Verify network namespace creation:
```bash
unshare --net --map-root-user ip link
```

