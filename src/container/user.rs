use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Create a non-root user in the container
pub fn create_user(container_root: &str, username: &str, uid: u32, gid: u32) -> Result<()> {
    println!("Creating user: {}", username);

    // Create user home directory
    let home_dir = format!("{}/home/{}", container_root, username);
    fs::create_dir_all(&home_dir)
        .with_context(|| format!("Failed to create home directory: {}", home_dir))?;

    // Create /etc/passwd entry - using encrypted password "root"
    let passwd_path = format!("{}/etc/passwd", container_root);
    let passwd_entry = format!(
        "{}:$6$salt$IxDD3jeSOb5eB1CX5LBsqZFVkJdido3OUILO5Ifz5iwMuTS4XMS130MTSuDDl3aCI6WouIL9AjRbLCelDCy.g.:{}:{}:{}:/home/{}:/bin/bash\n",
        username, uid, gid, username, username
    );

    if Path::new(&passwd_path).exists() {
        let mut passwd_content =
            fs::read_to_string(&passwd_path).context("Failed to read /etc/passwd")?;

        // Check if user already exists
        if !passwd_content.contains(&format!("{}:", username)) {
            passwd_content.push_str(&passwd_entry);
            fs::write(&passwd_path, passwd_content).context("Failed to write /etc/passwd")?;
        }
    } else {
        // Create new passwd file
        let passwd_content = format!("root:x:0:0:root:/root:/bin/bash\n{}", passwd_entry);
        fs::write(&passwd_path, passwd_content).context("Failed to create /etc/passwd")?;
    }

    // Create /etc/group entry
    let group_path = format!("{}/etc/group", container_root);
    let group_entry = format!("{}:x:{}:\n", username, gid);

    if Path::new(&group_path).exists() {
        let mut group_content =
            fs::read_to_string(&group_path).context("Failed to read /etc/group")?;

        // Check if group already exists
        if !group_content.contains(&format!("{}:", username)) {
            group_content.push_str(&group_entry);
            fs::write(&group_path, group_content).context("Failed to write /etc/group")?;
        }
    } else {
        // Create new group file
        let group_content = format!("root:x:0:\n{}", group_entry);
        fs::write(&group_path, group_content).context("Failed to create /etc/group")?;
    }

    // Create basic shell profile with user-like experience
    let bashrc_path = format!("{}/home/{}/.bashrc", container_root, username);
    let bashrc_content = format!(
        r#"# Basic bashrc for container user
export PS1="{}@container:\w\$ "
export PATH=/home/{}/.local/bin:/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin
export HOME=/home/{}
export USER={}
export LOGNAME={}

# Custom whoami that returns the username instead of root
function whoami() {{
    echo "{}"
}}

# Aliases for better user experience
alias ll="ls -la"
alias la="ls -A"
alias l="ls -CF"
"#,
        username, username, username, username, username, username
    );
    fs::write(&bashrc_path, bashrc_content).context("Failed to create .bashrc")?;

    // Create /etc/shadow entry for password authentication
    let shadow_path = format!("{}/etc/shadow", container_root);
    let shadow_entry = format!(
        "{}:$6$salt$IxDD3jeSOb5eB1CX5LBsqZFVkJdido3OUILO5Ifz5iwMuTS4XMS130MTSuDDl3aCI6WouIL9AjRbLCelDCy.g.:19000:0:99999:7:::\n",
        username
    );

    if Path::new(&shadow_path).exists() {
        let mut shadow_content =
            fs::read_to_string(&shadow_path).context("Failed to read /etc/shadow")?;

        if !shadow_content.contains(&format!("{}:", username)) {
            shadow_content.push_str(&shadow_entry);
            fs::write(&shadow_path, shadow_content).context("Failed to write /etc/shadow")?;
        }
    } else {
        // Create new shadow file
        let shadow_content = format!("root:*:19000:0:99999:7:::\n{}", shadow_entry);
        fs::write(&shadow_path, shadow_content).context("Failed to create /etc/shadow")?;
    }

    // Set proper permissions on shadow file (0640)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(&shadow_path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o640);
            fs::set_permissions(&shadow_path, perms).ok();
        }
    }

    println!(
        "User {} created with UID {} and GID {} (password: root)",
        username, uid, gid
    );
    Ok(())
}

/// Switch to the specified user before executing commands
pub fn switch_user(username: &str, uid: u32, gid: u32) -> Result<()> {
    use nix::unistd::{Gid, Uid, setgid, setuid};

    // Set the group ID first
    setgid(Gid::from_raw(gid)).with_context(|| format!("Failed to set GID to {}", gid))?;

    // Set the user ID
    setuid(Uid::from_raw(uid)).with_context(|| format!("Failed to set UID to {}", uid))?;

    // Set environment variables
    // SAFETY: We are setting these environment variables in a controlled container environment
    // before exec, which is safe in this context
    unsafe {
        std::env::set_var("USER", username);
        std::env::set_var("LOGNAME", username);
        std::env::set_var("HOME", format!("/home/{}", username));
    }

    println!("Switched to user: {} ({}:{})", username, uid, gid);
    Ok(())
}

/// Default user configuration for containers
/// When --user flag is used, we use UID 1000 with proper user namespace mapping
pub fn get_default_user() -> (&'static str, u32, u32) {
    ("user", 1000, 1000)
}
