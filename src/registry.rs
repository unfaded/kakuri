use crate::config::Config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerRegistry {
    pub containers: HashMap<String, ContainerInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub status: ContainerStatus,
    pub config: ContainerConfig,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContainerStatus {
    Created,
    Running,
    Stopped,
    Temporary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerConfig {
    pub allow_network: bool,
    pub init: bool,
    pub command: Option<String>,
    pub args: Vec<String>,
    #[serde(default)]
    pub bind_mounts: Vec<BindMount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindMount {
    pub host_path: String,
    pub container_path: Option<String>, // If None, use same as host_path
    pub create_if_missing: bool,
}

impl BindMount {
    pub fn container_path(&self) -> &str {
        self.container_path.as_ref().unwrap_or(&self.host_path)
    }

    pub fn from_string(bind_str: &str) -> Result<Self> {
        if let Some((host, container)) = bind_str.split_once(":") {
            // Format: host_path:container_path
            Ok(BindMount {
                host_path: host.to_string(),
                container_path: Some(container.to_string()),
                create_if_missing: true,
            })
        } else {
            // Format: path (same for both host and container)
            Ok(BindMount {
                host_path: bind_str.to_string(),
                container_path: None,
                create_if_missing: true,
            })
        }
    }
}

impl ContainerRegistry {
    pub fn load() -> Result<Self> {
        let config = Config::load()?;
        let registry_path = Self::registry_path(&config)?;

        if registry_path.exists() {
            let content =
                fs::read_to_string(&registry_path).context("Failed to read registry file")?;
            serde_json::from_str(&content).context("Failed to parse registry file")
        } else {
            Ok(Self {
                containers: HashMap::new(),
            })
        }
    }

    pub fn save(&self) -> Result<()> {
        let config = Config::load()?;
        let registry_path = Self::registry_path(&config)?;

        // Create containers directory if it doesn't exist
        let containers_dir = config.containers_dir()?;
        fs::create_dir_all(&containers_dir).context("Failed to create containers directory")?;

        let content = serde_json::to_string_pretty(self).context("Failed to serialize registry")?;
        fs::write(&registry_path, content).context("Failed to write registry file")?;

        Ok(())
    }

    fn registry_path(config: &Config) -> Result<PathBuf> {
        Ok(config.containers_dir()?.join("registry.json"))
    }

    pub fn generate_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Use last 6 chars of timestamp + random component for uniqueness
        format!("{:x}", timestamp).chars().rev().take(6).collect()
    }

    pub fn add_container(
        &mut self,
        name: String,
        config: ContainerConfig,
        is_temporary: bool,
    ) -> Result<String> {
        let id = Self::generate_id();
        let full_id = format!("{}_{}", name, id);

        let container_info = ContainerInfo {
            id: id.clone(),
            name: name.clone(),
            status: if is_temporary {
                ContainerStatus::Temporary
            } else {
                ContainerStatus::Created
            },
            config,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            started_at: None,
            pid: None,
        };

        self.containers.insert(full_id.clone(), container_info);

        if !is_temporary {
            self.save()?;
        }

        Ok(full_id)
    }

    pub fn find_by_name(&self, name: &str) -> Vec<&ContainerInfo> {
        self.containers
            .values()
            .filter(|container| {
                container.name == name && !matches!(container.status, ContainerStatus::Temporary)
            })
            .collect()
    }

    pub fn get_container(&self, full_id: &str) -> Option<&ContainerInfo> {
        self.containers.get(full_id)
    }

    pub fn get_container_mut(&mut self, full_id: &str) -> Option<&mut ContainerInfo> {
        self.containers.get_mut(full_id)
    }

    pub fn remove_container(&mut self, full_id: &str) -> Result<()> {
        self.containers.remove(full_id);
        self.save()
    }

    pub fn get_container_dir(&self, full_id: &str) -> Result<PathBuf> {
        let config = Config::load()?;
        Ok(config.containers_dir()?.join(full_id))
    }

    pub fn cleanup_temporary(&mut self) -> Result<()> {
        let temp_containers: Vec<String> = self
            .containers
            .iter()
            .filter(|(_, info)| matches!(info.status, ContainerStatus::Temporary))
            .map(|(id, _)| id.clone())
            .collect();

        for id in temp_containers {
            self.containers.remove(&id);
            // Also cleanup filesystem
            let container_dir = self.get_container_dir(&id)?;
            if container_dir.exists() {
                fs::remove_dir_all(container_dir).ok(); // Don't fail if cleanup fails
            }
        }

        Ok(())
    }
}

impl ContainerInfo {
    pub fn full_id(&self) -> String {
        format!("{}_{}", self.name, self.id)
    }
}
