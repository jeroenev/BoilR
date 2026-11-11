use std::path::{Path, PathBuf};

use boilr_core::launch_and_wait::LaunchAndWait;
use steam_shortcuts_util::shortcut::{Shortcut, ShortcutOwned};

#[derive(Clone)]
pub(crate) struct UbisoftGame {
    pub(crate) name: String,
    pub(crate) icon: String,
    pub(crate) id: String,
    pub(crate) launcher: PathBuf,
    pub(crate) launcher_compat_folder: Option<PathBuf>,
    pub(crate) launch_id: usize,
    /// The game's install folder (the registry's `InstallDir`), when known.
    pub(crate) game_folder: Option<PathBuf>,
    /// The BoilR the shortcut runs to wait for the game, see
    /// [`boilr_core::launch_and_wait`]. Ubisoft Connect hands the game over and keeps
    /// running, so Steam would otherwise count the game as running until Ubisoft Connect
    /// closes, or (when it was already open) as stopped at once.
    pub(crate) helper: Option<PathBuf>,
    /// Close Ubisoft Connect before and after the game, so Steam starts it and its overlay
    /// reaches the game ([`super::settings::UbisoftSettings::steam_overlay`]).
    pub(crate) restart_launcher: bool,
}

impl From<UbisoftGame> for ShortcutOwned {
    fn from(game: UbisoftGame) -> Self {
        let link = format!("uplay://launch/{}/{}", game.id, game.launch_id);
        if let (Some(helper), Some(game_folder), None) = (
            &game.helper,
            &game.game_folder,
            &game.launcher_compat_folder,
        ) {
            let target = LaunchAndWait {
                launcher: game.launcher.clone(),
                link,
                game_folder: game_folder.clone(),
                restart_launcher: game.restart_launcher,
            }
            .shortcut_target(helper);
            return Shortcut::new(
                "0",
                &game.name,
                &target.exe,
                &target.start_dir,
                &game.icon,
                "",
                &target.launch_options,
            )
            .to_owned();
        }
        let launch = match game.launcher_compat_folder {
            Some(compat_folder) => format!(
                "STEAM_COMPAT_DATA_PATH=\"{}\" %command% \"{link}\"",
                compat_folder.to_string_lossy(),
            ),
            None => format!("\"{link}\""),
        };
        let start_dir = game
            .launcher
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_string_lossy();
        let exe = format!("\"{}\"", game.launcher.to_string_lossy());
        Shortcut::new("0", &game.name, &exe, &start_dir, &game.icon, "", &launch).to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn far_cry_3() -> UbisoftGame {
        UbisoftGame {
            name: "Far Cry 3".to_string(),
            icon: String::new(),
            id: "46".to_string(),
            launcher: PathBuf::from(r"C:\Ubisoft Game Launcher\upc.exe"),
            launcher_compat_folder: None,
            launch_id: 0,
            game_folder: Some(PathBuf::from(r"D:\UplayLibrary\Far Cry 3\")),
            helper: Some(PathBuf::from(r"C:\BoilR\BoilR.exe")),
            restart_launcher: true,
        }
    }

    #[test]
    fn runs_boilr_to_wait_for_the_game() {
        let shortcut: ShortcutOwned = far_cry_3().into();
        assert_eq!(shortcut.exe, r#""C:\BoilR\BoilR.exe""#);
        assert_eq!(
            shortcut.launch_options,
            r#"--launch-and-wait --game-folder "D:\UplayLibrary\Far Cry 3" --launcher "C:\Ubisoft Game Launcher\upc.exe" --link "uplay://launch/46/0" --restart-launcher"#
        );
    }

    #[test]
    fn without_a_folder_opens_the_link_in_ubisoft_connect() {
        let game = UbisoftGame {
            game_folder: None,
            ..far_cry_3()
        };
        let shortcut: ShortcutOwned = game.into();
        assert_eq!(shortcut.exe, r#""C:\Ubisoft Game Launcher\upc.exe""#);
        assert_eq!(shortcut.launch_options, r#""uplay://launch/46/0""#);
    }
}
