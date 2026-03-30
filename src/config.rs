use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for WebdavBridge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Upstream WebDAV server URL
    pub upstream_url: String,

    /// Upstream authentication username (optional)
    pub upstream_username: Option<String>,

    /// Upstream authentication password (optional)
    pub upstream_password: Option<String>,

    /// Local cache directory for file content
    pub cache_dir: PathBuf,

    /// Path to metadata database (sled)
    pub metadata_db_path: PathBuf,

    /// Maximum concurrent connections to upstream (default: 2)
    pub rate_limit_permits: usize,

    /// Metadata update interval in seconds (default: 300 = 5 minutes)
    pub metadata_update_interval_secs: u64,

    /// Maximum depth for recursive PROPFIND (default: 10)
    pub max_depth: u32,

    /// Server bind address (default: "127.0.0.1:8080")
    pub server_bind: String,

    /// Server prefix path (e.g., "/webdav") (default: "/")
    pub server_prefix: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            upstream_url: "http://localhost:8081".to_string(),
            upstream_username: None,
            upstream_password: None,
            cache_dir: PathBuf::from("./cache"),
            metadata_db_path: PathBuf::from("./metadata.db"),
            rate_limit_permits: 2,
            metadata_update_interval_secs: 300,
            max_depth: 10,
            server_bind: "127.0.0.1:8080".to_string(),
            server_prefix: "/".to_string(),
        }
    }
}

/// Load configuration from a YAML file
pub fn load_config(path: &str) -> anyhow::Result<Config> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = serde_yaml::from_str(&content)?;
    Ok(config)
}

/// Save configuration to a YAML file
pub fn save_config(config: &Config, path: &str) -> anyhow::Result<()> {
    let content = serde_yaml::to_string(config)?;
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.rate_limit_permits, 2);
        assert_eq!(config.metadata_update_interval_secs, 300);
    }

    #[test]
    fn test_save_load_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");

        let config = Config::default();
        save_config(&config, config_path.to_str().unwrap()).unwrap();

        let loaded = load_config(config_path.to_str().unwrap()).unwrap();
        assert_eq!(loaded.upstream_url, config.upstream_url);
        assert_eq!(loaded.rate_limit_permits, config.rate_limit_permits);
    }
}
