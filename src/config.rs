use serde::Deserialize;
use std::fs;
use anyhow::{Context, Result};

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub mixer_addr: String,      // Keep as string for the TOML
                                    //
    #[serde(default = "default_socket_path")]
    pub socket_path: String,
}

fn default_socket_path() -> String {
    "/tmp/xrossd.sock".to_string()
}

impl Config {
    pub fn load() -> Result<Self> {
        let content = fs::read_to_string("xrossd.toml")
            .context("Could not find xrossd.toml")?;
        
        let cfg: Config = toml::from_str(&content)
            .context("Failed to parse xrossd.toml")?;
            
        Ok(cfg)
    }
}
