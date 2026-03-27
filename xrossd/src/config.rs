use serde::Deserialize;
use std::fs;
use anyhow::{Context, Result};
use crate::cli::config_path;
use xross_core::constants::DEFAULT_SOCKET;

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub mixer_addr: String,      // Keep as string for the TOML
                                    //
    #[serde(default = "default_socket_path")]
    pub socket_path: String,

    pub timeout: Option<ConfigTimeout>
}

#[derive(Deserialize, Debug, Clone)]
pub struct ConfigTimeout {
    pub command: String,
    pub after_mins: usize,
    pub db_threshold: f32
}

fn default_socket_path() -> String {
    DEFAULT_SOCKET.to_string()
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        let content = fs::read_to_string(path)
            .context("Could not find xrossd.toml")?;
        
        let cfg: Config = toml::from_str(&content)
            .context("Failed to parse xrossd.toml")?;
            
        Ok(cfg)
    }
}
