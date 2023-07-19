use std::{
    fs,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use keyvalues_parser::Vdf;
use steamid_ng::SteamID;

pub const TF2_GAME_ID: &str = "440";

pub fn locate_steam_logged_in_users() -> Option<PathBuf> {
    tracing::debug!("Fetching Steam loginusers.vdf");
    let mut base_folder: PathBuf = find_base_lib();
    base_folder.push::<PathBuf>("config/loginusers.vdf".into());
    if base_folder.as_path().exists() {
        tracing::info!("Located current Steam user data.");
        Some(base_folder)
    } else {
        tracing::error!("Could not locate loginusers.vdf in the Steam dir");
        None
    }
}

pub fn read_steam_launch_configs(user: SteamID, app_id: &str) -> Result<String, anyhow::Error> {
    tracing::debug!(
        "Reading Steam localconfig.vdf for launch configs of app {}",
        app_id
    );
    let path = locate_steam_launch_configs(user)
        .context("Failed to find localconfig.vdf in Steam dir.")?;
    let mut data: Vec<u8> = Vec::new();
    tracing::debug!("Found Steam local app config.");

    if let Ok(mut f) = File::open(&path) {
        f.read_to_end(&mut data)
            .context(format!("Failed to read {:?}", path))?;
    }
    let binding = &String::from_utf8_lossy(&data);
    let vdf = Vdf::parse(binding).context("Failed to parse localconfig.vdf (invalid VDF data).")?;

    let key = vdf.key;
    anyhow::ensure!(
        key == "UserLocalConfigStore",
        "Did not find expected data in localconfig.vdf."
    );
    tracing::info!("Extracted Steam local app config.");

    // nested objects... "Software" -> "valve" -> "Steam" -> "apps" -> [<app_id>]
    let inner = vdf.value.unwrap_obj();

    let software = inner
        .get_key_value("Software")
        .context("Failed to find 'Software' object in localconfig.vdf")?;
    tracing::debug!("Parsed 'Software' key out of localconfig.vdf");

    let valve = software
        .1
        .iter()
        .find_map(|obj| obj.get_obj().unwrap().get_key_value("valve"))
        .context("Failed to find 'valve' object in 'Software'")?;
    tracing::debug!("Parsed 'valve' key out of 'Software'");

    let steam = valve
        .1
        .iter()
        .find_map(|obj| obj.get_obj().unwrap().get_key_value("Steam"))
        .context("Failed to find 'Steam' object in 'valve'")?;
    tracing::debug!("Parsed 'Steam' key out of 'valve'");

    let apps = steam
        .1
        .iter()
        .find_map(|obj| obj.get_obj().unwrap().get_key_value("apps"))
        .context("Failed to find 'apps' object in 'Steam'")?;
    tracing::debug!("Parsed 'apps' key out of 'Steam'");

    let app = apps
        .1
        .iter()
        .find_map(|obj| obj.get_obj().unwrap().get_key_value(app_id))
        .context(format!(
            "Failed to find app of id '{}' object in 'apps'",
            app_id
        ))?;
    tracing::debug!("Parsed app id '{}' key out of 'apps'", app_id);

    let launch_options = app
        .1
        .iter()
        .find_map(|obj| obj.get_obj().unwrap().get_key_value("LaunchOptions"))
        .context("Failed to find launch options of taget app")?;
    tracing::debug!("Parsed 'LaunchOptions' key out of app id '{}'", app_id);

    let unwrapped_lo =
        launch_options.1.get(0).unwrap().get_str().context(
            "Failed to find associated launch options value string with LaunchOptions key.",
        )?;
    tracing::debug!("Parsed launch options value out of 'LaunchOptions'");

    tracing::info!("Extracted launch options associated with app {}", app_id);
    Ok(unwrapped_lo.to_string())
}

pub fn locate_steam_launch_configs(steam_user: SteamID) -> Option<PathBuf> {
    tracing::debug!("Fetching Steam userdata/<player>/config/localconfig.vdf");
    let a_id = steam_user.account_id();
    let mut base_folder: PathBuf = find_base_lib();
    base_folder.push::<PathBuf>(format!("userdata/{}/config/localconfig.vdf", a_id,).into());
    if base_folder.as_path().exists() {
        tracing::info!("Located local launch configs.");
        Some(base_folder)
    } else {
        tracing::error!("Could not find local configs (player not found).");
        None
    }
}

/// Attempts to open the TF2 directory or locate it if it's not in the expected place
pub fn locate_tf2_folder() -> Option<PathBuf> {
    tracing::debug!("Fetching TF2 Folder");
    let libs: Vec<PathBuf> = fetch_libraryfolders();

    for lib in libs {
        let mut path = lib.to_path_buf();
        path.push::<PathBuf>(get_rel_tf2_path().into());
        tracing::debug!("Found TF2 Folder: {:?}", path);

        if path.exists() && verify_tf_location(&path) {
            tracing::info!("Using TF2 directory: {}", path.to_string_lossy());
            return Some(path);
        }
    }
    None
}

fn fetch_libraryfolders() -> Vec<PathBuf> {
    tracing::debug!("Attempting to open libraryfolders.vdf");
    const LIBFILE: &str = "libraryfolders.vdf";
    let libraryfolders = fs::read_to_string(Path::join(&find_default_lib(), LIBFILE));

    match libraryfolders {
        Ok(content) => {
            let mut paths = Vec::new();
            let lines = content.lines();

            for line in lines {
                if line.contains("path") {
                    if let Some(path_str) = line.split('"').nth(3) {
                        paths.push(PathBuf::from(path_str));
                    }
                }
            }

            tracing::debug!("Successfully read libraryfolders");
            paths
        }
        Err(err) => {
            tracing::error!("Failed to read libraryfolders.vdf: {:?}", err);
            Vec::new()
        }
    }
}

fn find_default_lib() -> PathBuf {
    let mut path: PathBuf = find_base_lib();
    path.push::<PathBuf>("steamapps/".into());

    path
}

fn find_base_lib() -> PathBuf {
    #[cfg(target_os = "windows")]
    let default_dir = r"C:\Program Files (x86)\Steam\".into();

    #[cfg(not(target_os = "windows"))]
    let default_dir = {
        use std::env::var_os;
        var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or("~".into())
            .join(".steam/steam/")
    };

    default_dir
}

fn verify_tf_location(lib: &Path) -> bool {
    tracing::debug!("Start TF2 Verification of {:?}", lib.to_string_lossy());
    let gameinfo = "tf/gameinfo.txt";
    let mut path = lib.to_path_buf();
    path.push(gameinfo);

    if path.exists() {
        tracing::debug!("Passed Verification Check");
        return true;
    }
    tracing::debug!("Failed Verification Check");
    false
}

#[cfg(target_os = "windows")]
fn get_rel_tf2_path() -> String {
    r"steamapps\common\Team Fortress 2".to_string()
}

#[cfg(not(target_os = "windows"))]
fn get_rel_tf2_path() -> String {
    r"steamapps/common/Team Fortress 2".to_string()
}
