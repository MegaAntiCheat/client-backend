use std::{path::PathBuf, sync::Arc};

use crate::gamefinder;

pub struct Settings {
    pub tf2_directory: PathBuf,
    pub rcon_password: Arc<str>,
    pub port: u16,
}

impl Settings {
    pub fn new() -> Settings {
        Self::default()
    }
}

impl Default for Settings {
    fn default() -> Self {
        let tf2_directory = gamefinder::locate_tf2_folder().unwrap_or(PathBuf::new());

        Settings {
            tf2_directory,
            rcon_password: "mac_rcon".into(),
            port: 3621,
        }
    }
}
