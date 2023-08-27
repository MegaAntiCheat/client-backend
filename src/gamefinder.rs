use std::path::PathBuf;

use anyhow::{anyhow, Result};
use steamid_ng::SteamID;
use steamlocate::SteamDir;

pub const TF2_GAME_ID: u32 = 440;

pub fn locate_steam_logged_in_users() -> Result<PathBuf> {
    tracing::debug!("Fetching Steam loginusers.vdf");
    let mut base_folder: PathBuf = SteamDir::locate()
        .ok_or(anyhow!("Failed to locate Steam directory"))?
        .path;
    base_folder.push::<PathBuf>("config/loginusers.vdf".into());
    if base_folder.as_path().exists() {
        Ok(base_folder)
    } else {
        Err(anyhow!("Could not locate loginusers.vdf in the Steam dir"))
    }
}

pub fn locate_steam_launch_configs(steam_user: SteamID) -> Result<PathBuf> {
    let a_id = steam_user.account_id();
    let local_config_path = format!("userdata/{}/config/localconfig.vdf", a_id);
    tracing::debug!("Fetching Steam {}", local_config_path);

    let steam = SteamDir::locate().ok_or(anyhow!("Failed to locate Steam directory."))?;
    let mut base_folder: PathBuf = steam.path;
    base_folder.push(local_config_path);
    if base_folder.as_path().exists() {
        Ok(base_folder)
    } else {
        Err(anyhow!("Could not find local configs (player not found)."))
    }
}

/// Attempts to open the TF2 directory or locate it if it's not in the expected place
pub fn locate_tf2_folder() -> Result<PathBuf> {
    Ok(SteamDir::locate()
        .ok_or(anyhow!("Failed to locate Steam directory"))?
        .app(&TF2_GAME_ID)
        .ok_or(anyhow!("Failed to locate TF2 installation."))?
        .path
        .clone())
}
