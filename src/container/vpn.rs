use crate::registry::VpnConfig;
use anyhow::{Context, Result};
use std::process::Command;
use std::fs;

pub fn setup_vpn_network(vpn_config: &VpnConfig) -> Result<()> {
    println!("Setting up VPN network isolation");

    // Check for required tools
    check_required_tools()?;

    // First, ensure we have a loopback interface
    setup_loopback()?;

    // Get the WireGuard configuration content
    let config_content = get_vpn_config_content(vpn_config)?;

    // Parse essential config for interface setup
    let (private_key, address, dns) = parse_wg_config(&config_content)?;
    
    // Extract endpoint for connectivity setup
    let endpoint = extract_endpoint(&config_content)?;
    
    // Set up connectivity to VPN endpoint before creating tunnel
    setup_endpoint_connectivity(&endpoint)?;

    // Create WireGuard interface
    create_wg_interface(&vpn_config.interface_name)?;

    // Configure the interface with private key
    configure_wg_interface(&vpn_config.interface_name, &private_key, &config_content)?;

    // Set up IP address
    set_interface_address(&vpn_config.interface_name, &address)?;

    // Bring interface up
    bring_interface_up(&vpn_config.interface_name)?;

    // Set up routing (default route through VPN)
    setup_vpn_routing(&vpn_config.interface_name)?;

    // Configure DNS if specified
    if let Some(dns_servers) = dns {
        configure_dns(&dns_servers)?;
    }


    println!("VPN network isolation configured successfully");
    Ok(())
}

fn setup_loopback() -> Result<()> {
    // Bring up loopback interface
    let status = Command::new("ip")
        .args(&["link", "set", "dev", "lo", "up"])
        .status()
        .context("Failed to execute ip command")?;

    if !status.success() {
        anyhow::bail!("Failed to bring up loopback interface");
    }

    Ok(())
}

fn get_vpn_config_content(vpn_config: &VpnConfig) -> Result<String> {
    if let Some(config_path) = &vpn_config.config_path {
        // Read from file path
        fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read VPN config file: {}", config_path))
    } else if let Some(config_name) = &vpn_config.config_name {
        // Look for named config in standard locations
        let home = std::env::var("HOME")?;
        let possible_paths = [
            format!("/etc/wireguard/{}.conf", config_name),
            format!("{}/.config/wireguard/{}.conf", home, config_name),
            format!("{}/.wireguard/{}.conf", home, config_name),
        ];

        for path in &possible_paths {
            if std::path::Path::new(path).exists() {
                return fs::read_to_string(path)
                    .with_context(|| format!("Failed to read VPN config file: {}", path));
            }
        }

        anyhow::bail!("VPN config not found: {}", config_name);
    } else {
        anyhow::bail!("No VPN config path or name specified");
    }
}

fn parse_wg_config(config: &str) -> Result<(String, String, Option<Vec<String>>)> {
    let mut private_key = None;
    let mut address = None;
    let mut dns = None;

    for line in config.lines() {
        let line = line.trim();
        if line.starts_with("PrivateKey") {
            if let Some((_, value)) = line.split_once('=') {
                private_key = Some(value.trim().to_string());
            }
        } else if line.starts_with("Address") {
            if let Some((_, value)) = line.split_once('=') {
                // Take only the first address if multiple are specified (comma-separated)
                let first_addr = value.split(',').next().unwrap_or(value).trim();
                address = Some(first_addr.to_string());
            }
        } else if line.starts_with("DNS") {
            if let Some((_, value)) = line.split_once('=') {
                let dns_servers: Vec<String> = value
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                dns = Some(dns_servers);
            }
        }
    }

    let private_key = private_key.ok_or_else(|| anyhow::anyhow!("No PrivateKey found in config"))?;
    let address = address.ok_or_else(|| anyhow::anyhow!("No Address found in config"))?;

    Ok((private_key, address, dns))
}

fn create_wg_interface(interface_name: &str) -> Result<()> {
    let status = Command::new("ip")
        .args(&["link", "add", "dev", interface_name, "type", "wireguard"])
        .status()
        .context("Failed to create WireGuard interface")?;

    if !status.success() {
        anyhow::bail!("Failed to create WireGuard interface: {}", interface_name);
    }

    Ok(())
}

fn configure_wg_interface(interface_name: &str, private_key: &str, full_config: &str) -> Result<()> {
    // Set private key
    let private_key_status = Command::new("wg")
        .args(&["set", interface_name, "private-key", "/dev/stdin"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(private_key.as_bytes())?;
            }
            child.wait()
        })
        .context("Failed to set WireGuard private key")?;

    if !private_key_status.success() {
        anyhow::bail!("Failed to set WireGuard private key for interface: {}", interface_name);
    }

    // Parse and add peers manually to avoid Address parsing issues
    configure_wg_peers(interface_name, full_config)?;

    Ok(())
}

fn configure_wg_peers(interface_name: &str, config: &str) -> Result<()> {
    let mut in_peer_section = false;
    let mut peer_public_key: Option<String> = None;
    let mut peer_endpoint: Option<String> = None;
    let mut peer_allowed_ips: Option<String> = None;

    for line in config.lines() {
        let line = line.trim();
        
        if line == "[Peer]" {
            // Apply previous peer if exists
            if let Some(public_key) = peer_public_key.take() {
                add_peer(interface_name, &public_key, peer_endpoint.as_deref(), peer_allowed_ips.as_deref())?;
                peer_endpoint = None;
                peer_allowed_ips = None;
            }
            in_peer_section = true;
        } else if line.starts_with("[") {
            in_peer_section = false;
        } else if in_peer_section {
            if line.starts_with("PublicKey") {
                if let Some((_, value)) = line.split_once('=') {
                    peer_public_key = Some(value.trim().to_string());
                }
            } else if line.starts_with("Endpoint") {
                if let Some((_, value)) = line.split_once('=') {
                    peer_endpoint = Some(value.trim().to_string());
                }
            } else if line.starts_with("AllowedIPs") {
                if let Some((_, value)) = line.split_once('=') {
                    peer_allowed_ips = Some(value.trim().to_string());
                }
            }
        }
    }

    // Apply final peer if exists
    if let Some(public_key) = peer_public_key {
        add_peer(interface_name, &public_key, peer_endpoint.as_deref(), peer_allowed_ips.as_deref())?;
    }

    Ok(())
}

fn add_peer(interface_name: &str, public_key: &str, endpoint: Option<&str>, allowed_ips: Option<&str>) -> Result<()> {
    let mut args = vec!["set", interface_name, "peer", public_key];
    
    if let Some(endpoint) = endpoint {
        args.push("endpoint");
        args.push(endpoint);
    }
    
    if let Some(allowed_ips) = allowed_ips {
        args.push("allowed-ips");
        args.push(allowed_ips);
    }
    
    // Add persistent keepalive to maintain connection
    args.push("persistent-keepalive");
    args.push("25");

    let status = Command::new("wg")
        .args(&args)
        .status()
        .context("Failed to add WireGuard peer")?;

    if !status.success() {
        anyhow::bail!("Failed to add WireGuard peer with public key: {}", public_key);
    }

    println!("Added WireGuard peer: {} (keepalive: 25s)", public_key);
    Ok(())
}

fn set_interface_address(interface_name: &str, address: &str) -> Result<()> {
    let status = Command::new("ip")
        .args(&["addr", "add", address, "dev", interface_name])
        .status()
        .context("Failed to set interface address")?;

    if !status.success() {
        anyhow::bail!("Failed to set address {} on interface {}", address, interface_name);
    }

    Ok(())
}

fn bring_interface_up(interface_name: &str) -> Result<()> {
    let status = Command::new("ip")
        .args(&["link", "set", "dev", interface_name, "up"])
        .status()
        .context("Failed to bring interface up")?;

    if !status.success() {
        anyhow::bail!("Failed to bring up interface: {}", interface_name);
    }

    Ok(())
}

fn setup_vpn_routing(interface_name: &str) -> Result<()> {
    // Delete any existing default routes first
    let _ = Command::new("ip")
        .args(&["route", "del", "default"])
        .output();

    // Enable IP forwarding for WireGuard
    let _ = std::fs::write("/proc/sys/net/ipv4/ip_forward", "1");

    // Add simple default route through WireGuard interface
    let status = Command::new("ip")
        .args(&["route", "add", "default", "dev", interface_name])
        .status()
        .context("Failed to add default route")?;

    if !status.success() {
        // Fallback to split routing if default route fails
        let status1 = Command::new("ip")
            .args(&["route", "add", "0.0.0.0/1", "dev", interface_name])
            .status()
            .context("Failed to add route for 0.0.0.0/1")?;
            
        let status2 = Command::new("ip")
            .args(&["route", "add", "128.0.0.0/1", "dev", interface_name])
            .status()
            .context("Failed to add route for 128.0.0.0/1")?;

        if !status1.success() || !status2.success() {
            anyhow::bail!("Failed to set up WireGuard routing");
        }
        println!("VPN routing configured with split default routes");
    } else {
        println!("VPN routing configured with default route");
    }

    Ok(())
}


fn configure_dns(dns_servers: &[String]) -> Result<()> {
    // Create resolv.conf content
    let mut resolv_content = String::new();
    for server in dns_servers {
        resolv_content.push_str(&format!("nameserver {}\n", server));
    }

    // Write to temporary file first
    let temp_resolv_path = "/tmp/resolv.conf.vpn";
    fs::write(temp_resolv_path, resolv_content)
        .context("Failed to write temporary resolv.conf")?;

    // Mount the temporary file over /etc/resolv.conf
    let status = Command::new("mount")
        .args(&["--bind", temp_resolv_path, "/etc/resolv.conf"])
        .status()
        .context("Failed to bind mount VPN resolv.conf")?;

    if !status.success() {
        anyhow::bail!("Failed to bind mount VPN DNS configuration");
    }

    println!("Configured DNS servers: {}", dns_servers.join(", "));
    Ok(())
}

fn extract_endpoint(config: &str) -> Result<String> {
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with("Endpoint") {
            if let Some((_, value)) = line.split_once('=') {
                return Ok(value.trim().to_string());
            }
        }
    }
    anyhow::bail!("No Endpoint found in WireGuard config");
}

fn setup_endpoint_connectivity(endpoint: &str) -> Result<()> {
    // The fundamental issue: we're in an isolated network namespace but need to reach
    // the VPN server to establish the connection. We need to temporarily inherit 
    // host networking for the initial handshake, or use a different approach.
    
    // For now, let's try to establish a route through the host namespace
    // This is complex because we're already in an isolated namespace
    
    println!("VPN endpoint: {}", endpoint);
    println!("Note: Initial VPN handshake may require host connectivity");
    
    Ok(())
}

pub fn setup_vpn_in_host(vpn_config: &VpnConfig) -> Result<String> {
    println!("Setting up VPN in host namespace");

    // Check for required tools
    check_required_tools()?;

    // Get the WireGuard configuration content
    let config_content = get_vpn_config_content(vpn_config)?;

    // Parse essential config for interface setup
    let (private_key, address, _dns) = parse_wg_config(&config_content)?;

    // Create WireGuard interface in host namespace
    create_wg_interface(&vpn_config.interface_name)?;

    // Configure the interface with private key and peers
    configure_wg_interface(&vpn_config.interface_name, &private_key, &config_content)?;

    // Set up IP address
    set_interface_address(&vpn_config.interface_name, &address)?;

    // Bring interface up
    bring_interface_up(&vpn_config.interface_name)?;

    println!("VPN interface {} configured in host namespace", vpn_config.interface_name);
    Ok(vpn_config.interface_name.clone())
}

pub fn move_vpn_to_namespace(interface_name: &str) -> Result<()> {
    // Set up loopback interface first
    setup_loopback()?;

    // The interface should already be in this namespace since we're in the same process
    // Just need to bring it up and configure routing
    bring_interface_up(interface_name)?;

    // Set up routing in the container namespace
    setup_vpn_routing(interface_name)?;

    // Configure DNS for VPN (use common DNS servers since we can't access the config here)
    let vpn_dns = vec!["1.1.1.1".to_string(), "8.8.8.8".to_string()];
    configure_dns(&vpn_dns)?;

    println!("VPN interface configured in container namespace");
    Ok(())
}

fn check_required_tools() -> Result<()> {
    // Check for ip command
    if Command::new("ip").arg("--version").output().is_err() {
        anyhow::bail!("Required tool 'ip' not found. Please install iproute2 package.");
    }

    // Check for wg command
    if Command::new("wg").arg("--version").output().is_err() {
        anyhow::bail!("Required tool 'wg' not found. Please install wireguard-tools package.");
    }

    Ok(())
}

