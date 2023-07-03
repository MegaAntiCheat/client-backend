use std::{path::PathBuf, sync::Arc};

pub struct Settings {
    pub tf2_directory: PathBuf,
    pub rcon_password: Arc<str>,
}

impl Default for Settings {
    fn default() -> Self {
        #[cfg(target_os = "windows")]
        let tf2_directory = "C:/Program Files (x86)/Steam/steamapps/common/Team Fortress 2".into();

        #[cfg(not(target_os = "windows"))]
        let tf2_directory = {
            use std::env::var_os;
            var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or("~".into())
                .join(".steam/steam/steamapps/common/Team Fortress 2")
        };

        Settings {
            tf2_directory,
            rcon_password: "mac_rcon".into(),
        }
    }
}
