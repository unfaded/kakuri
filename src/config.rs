use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub storage: StorageConfig,
    pub defaults: DefaultsConfig,
    pub bind_profiles: Option<std::collections::HashMap<String, Vec<String>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub containers_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultsConfig {
    pub allow_network: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            storage: StorageConfig {
                containers_dir: "~/.local/kakuri/containers".to_string(),
            },
            defaults: DefaultsConfig {
                allow_network: false,
            },
            bind_profiles: Some({
                let mut profiles = std::collections::HashMap::new();

                // Common development profile
                profiles.insert(
                    "dev".to_string(),
                    vec![
                        "~/.config".to_string(),
                        "~/.local".to_string(),
                        "~/.cache".to_string(),
                        "~/.ssh".to_string(),
                    ],
                );

                // Minimal profile for secure containers
                profiles.insert("minimal".to_string(), vec!["~/.cache".to_string()]);

                profiles
            }),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;

        if config_path.exists() {
            let content = fs::read_to_string(&config_path).context("Failed to read config file")?;
            toml::from_str(&content).context("Failed to parse config file")
        } else {
            // Create default config
            let config = Config::default();
            config.save()?;
            Ok(config)
        }
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path()?;

        // Create config directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).context("Failed to create config directory")?;
        }

        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        fs::write(&config_path, content).context("Failed to write config file")?;

        Ok(())
    }

    fn config_path() -> Result<PathBuf> {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        Ok(PathBuf::from(home).join(".config/container/config.toml"))
    }

    pub fn containers_dir(&self) -> Result<PathBuf> {
        let path = if self.storage.containers_dir.starts_with("~/") {
            let home = std::env::var("HOME").context("HOME environment variable not set")?;
            self.storage
                .containers_dir
                .replace("~/", &format!("{}/", home))
        } else {
            self.storage.containers_dir.clone()
        };
        Ok(PathBuf::from(path))
    }
}
