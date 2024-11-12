use std::error::Error;

use config::Config;
use log::info;
use mac_address::MacAddress;
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq, Eq, Clone)]
pub struct ConfiguredHost {
    pub name: String,
    pub mac_address: MacAddress,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone)]
pub struct AppConfig {
    pub host_ip: String,
    pub port: String,
    pub token: String,
    pub hosts: Vec<ConfiguredHost>,
    pub occupation_level_percentage: u8,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host_ip: "0.0.0.0".to_string(),
            port: "8080".to_string(),
            token: String::default(),
            hosts: Vec::new(),
            occupation_level_percentage: 80,
        }
    }
}

pub fn read_config() -> Result<AppConfig, Box<dyn Error>> {
    info!("Reading config...");
    let settings = Config::builder()
        .add_source(config::File::with_name("/config/dyn-wol-config"))
        .add_source(config::Environment::with_prefix("DYN_WOL"))
        .build()?;

    let conf: AppConfig = settings.try_deserialize()?;
    info!("Successfully read config!");

    if conf.token == String::default() {
        return Err(
            "The token seems to be empty. Please make sure to configure a secure token!".into(),
        );
    }

    if conf.token.len() < 32 {
        return Err("The token is too short, it must have at least 32 chars!".into());
    }
    Ok(conf)
}
