use std::{
    fs,
    io,
    io::{Error, ErrorKind},
    path::PathBuf,
};

use anyhow::anyhow;
use dirs::home_dir;
use log::{info, warn};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tari_crypto::{ristretto::RistrettoSecretKey, tari_utilities::hex::Hex};
use tari_payment_engine::db_types::{Role, SerializedTariAddress};

#[derive(Serialize, Deserialize, Default)]
pub struct UserData {
    pub profiles: Vec<Profile>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Profile {
    pub name: String,
    pub address: SerializedTariAddress,
    pub secret_key: Option<RistrettoSecretKey>,
    pub secret_key_envar: Option<String>,
    pub roles: Vec<Role>,
    pub server: Url,
}

impl Profile {
    pub fn secret_key(&self) -> Option<RistrettoSecretKey> {
        self.secret_key.clone().or_else(|| {
            self.secret_key_envar.as_ref().and_then(|envar| {
                std::env::var(envar).ok().and_then(|s| {
                    RistrettoSecretKey::from_hex(&s)
                        .map_err(|e| warn!("Failed to parse secret key from envar {s} for profile {}. {e}", self.name))
                        .ok()
                })
            })
        })
    }

    pub fn server_url_for(server: &str) -> anyhow::Result<String> {
        let url = Url::parse(server)?;
        url.host_str().map(|s| s.to_string()).ok_or_else(|| anyhow!("Invalid server URL, {}", server))
    }
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            name: "default".to_string(),
            address: SerializedTariAddress::default(),
            secret_key: None,
            secret_key_envar: None,
            roles: vec![Role::User],
            server: default_server_url().expect(
                "Invalid default server URL. Check your TPG_HOST, TPG_PORT and TPG_SCHEMA environment variables",
            ),
        }
    }
}

pub fn get_config_path() -> io::Result<PathBuf> {
    let home = home_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Home directory not found"))?;
    let config_dir = home.join(".taritools");
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
        set_permissions(&config_dir, 0o700)?;
    }
    let config_file = config_dir.join("config.toml");
    if !config_file.exists() {
        info!("Creating default config file");
        let default_config = UserData::default();
        let config_str =
            toml::to_string(&default_config).map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))?;
        fs::write(&config_file, config_str)?;
        set_permissions(&config_file, 0o600)?;
    }
    Ok(config_dir.join("config.toml"))
}

fn set_permissions(config_dir: &PathBuf, perms: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(config_dir)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(perms); // Sets directory to only be accessible by the owner
        fs::set_permissions(config_dir, permissions)?;
    }
    Ok(())
}

pub fn read_config() -> io::Result<UserData> {
    let config_path = get_config_path()?;
    let config_str = fs::read_to_string(config_path)?;
    let config: UserData =
        toml::from_str(&config_str).map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))?;
    Ok(config)
}

pub fn write_config(config: &UserData) -> anyhow::Result<()> {
    let config_path = get_config_path()?;
    let config_str = toml::to_string(config)?;
    fs::write(config_path, config_str)?;
    Ok(())
}

pub fn default_server_url() -> anyhow::Result<Url> {
    let schema = std::env::var("TPG_SCHEMA").unwrap_or_else(|_| "http".to_string());
    let port = std::env::var("TPG_PORT").ok();
    let host = std::env::var("TPG_HOST").ok();
    let server = match (host, port) {
        (Some(host), Some(port)) => format!("{schema}://{host}:{port}"),
        (Some(host), None) => format!("{schema}://{host}"),
        (None, Some(port)) => format!("{schema}://localhost:{port}"),
        (None, None) => format!("{schema}://localhost:4444"),
    };
    let url = Url::parse(&server)?;
    Ok(url)
}
