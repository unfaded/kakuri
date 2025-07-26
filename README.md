# Kakuri

Lightweight containerization built on Linux namespaces and filesystem isolation.

## Features

- Unprivileged containers using Linux namespaces
- Persistent and temporary container modes
- Filesystem isolation with overlay mounts
- Optional user namespace mapping
- Network isolation with WireGuard VPN support
- Bind mount profiles for development workflows

## Quick Start

```bash
# Direct execution (temporary container)
kakuri
kakuri python3 script.py

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

## Network Isolation

### Default Behavior
- No network access (complete isolation)
- Use "--allow-network" for host network access

### VPN Isolation
```bash
# Route all container traffic through VPN
kakuri --vpn ~/wireguard/vpn-config.conf bash
kakuri --vpn ~/wireguard/vpn-config.conf python script.py

# Persistent VPN containers
kakuri create --vpn vpn-config secure-env
kakuri start secure-env --vpn different-config bash

# VPN management for existing containers
kakuri vpn set mycontainer vpn-config
kakuri vpn show mycontainer
kakuri vpn remove mycontainer
```

## Security Model

Kakuri uses Linux user namespaces to provide unprivileged containerization:

- Root inside container maps to your user outside
- Filesystem access controlled via bind mounts
- Network isolation via network namespaces
- Process isolation via PID namespaces

## Examples

### Development Environment

```bash
# Create development container with common tools
kakuri create --bind-profile dev --allow-network devenv

# Start with your project mounted
kakuri start devenv --bind ~/myproject:/workspace bash
```

### Secure Testing

```bash
# Isolated environment for untrusted code
kakuri --bind ~/downloads:/data python3 suspicious_script.py
```

### Network Services

```bash
# Run service with network but filesystem isolation
kakuri --allow-network --bind ./config:/etc/service service-binary
```

## Container Lifecycle

1. Create namespace (user, mount, PID, network, UTS, IPC)
2. Set up container filesystem with overlays
3. Apply bind mounts and network configuration
4. Execute target command
5. Clean up temporary resources

## VPN Configuration

Kakuri supports WireGuard VPN configurations in two ways:

### Named Configurations
Place ".conf" files in standard locations:
- /etc/wireguard/myconfig.conf
- ~/.config/wireguard/myconfig.conf
- ~/.wireguard/myconfig.conf

Then reference by name:
```bash
kakuri --vpn myconfig bash
```

### Direct File Paths
```bash
kakuri --vpn ~/vpn-configs/wireguard-location.conf bash
kakuri --vpn /etc/wireguard/secure.conf bash
```

### Requirements
- "wireguard-tools" package ("wg" command)
- "iproute2" package ("ip" command)
- Valid WireGuard configuration file

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

### VPN Issues
Check WireGuard tools installation:
```bash
wg --version
```

Verify configuration file format:
```bash
wg show all
```
