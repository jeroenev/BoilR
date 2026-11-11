use std::path::PathBuf;

use boilr_core::launch_and_wait::LaunchAndWait;
use steam_shortcuts_util::{shortcut::ShortcutOwned, Shortcut};

#[derive(Clone)]
pub struct EAGame {
    /// The `.mfst` id (URL-encoded offer ids), or the content ids from the game's installer
    /// data, comma-separated; see [`EAGame::found_by`].
    pub id: String,
    pub title: String,
    pub launcher_location: PathBuf,
    pub launcher_compat_folder: Option<PathBuf>,
    pub found_by: FoundBy,
    /// The game's install folder, when known (games found by their installer data).
    pub game_folder: Option<PathBuf>,
    /// The BoilR the shortcut runs to wait for the game, see
    /// [`boilr_core::launch_and_wait`]: the launcher exits as soon as it has handed the game
    /// to the EA app, so Steam would otherwise count the game as stopped while it runs.
    pub helper: Option<PathBuf>,
}

/// Where BoilR found an installed EA game, which decides what its id is and how it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoundBy {
    /// Origin's `.mfst` manifest: the id holds offer ids.
    Manifest,
    /// The game's installer data (the current EA app): the id holds content ids, which the
    /// EA app's `origin2://` link takes (Playnite's EaLibrary, `EaControllerHelper.cs`). The
    /// launcher is the program Windows opens those links with (EALauncher.exe).
    /// Only found on Windows.
    #[cfg_attr(not(windows), allow(dead_code))]
    InstallerData,
}

fn launch_url(game: &EAGame) -> String {
    match game.found_by {
        FoundBy::Manifest => make_launch_url(&game.id, &game.launcher_location),
        FoundBy::InstallerData => format!("origin2://game/launch?offerIds={}", game.id),
    }
}

fn make_launch_url(id: &str, launcher_location: &std::path::Path) -> String {
    // EA App (EA Desktop) uses link2ea:// — Origin used origin2://
    // Detect which is installed by the exe name so both old and new installs work
    let use_ea_app = launcher_location
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_ascii_lowercase().contains("eadesktop"))
        .unwrap_or(false);

    if use_ea_app {
        // EA App (2022+): link2ea://launchgame/{id}
        format!("link2ea://launchgame/{id}")
    } else {
        // Legacy Origin: origin2://game/launch?offerIds={id}&autoDownload=1&authCode=&cmdParams=
        format!("origin2://game/launch?offerIds={id}&autoDownload=1&authCode=&cmdParams=")
    }
}

impl From<EAGame> for ShortcutOwned {
    fn from(game: EAGame) -> Self {
        let url = launch_url(&game);
        let (exe, start_dir, launch) = match (
            &game.helper,
            &game.game_folder,
            &game.launcher_compat_folder,
        ) {
            (Some(helper), Some(game_folder), None) => {
                let target = LaunchAndWait {
                    launcher: game.launcher_location.clone(),
                    link: url,
                    game_folder: game_folder.clone(),
                    restart_launcher: false,
                }
                .shortcut_target(helper);
                (target.exe, target.start_dir, target.launch_options)
            }
            _ => {
                let launch = match &game.launcher_compat_folder {
                    Some(compat_folder) => format!(
                        "STEAM_COMPAT_DATA_PATH=\"{}\" %command% \"{url}\"",
                        compat_folder.to_string_lossy()
                    ),
                    None => format!("\"{url}\""),
                };
                // Steam on Windows starts a shortcut in its StartDir and fails with OS error
                // 123 ("Failed running GameID ..." in logs/gameprocess_log.txt) when it is
                // empty.
                let start_dir = game
                    .launcher_location
                    .parent()
                    .filter(|folder| !folder.as_os_str().is_empty())
                    .map(|folder| format!("\"{}\"", folder.to_string_lossy()))
                    .unwrap_or_default();
                (
                    format!("\"{}\"", game.launcher_location.to_string_lossy()),
                    start_dir,
                    launch,
                )
            }
        };
        let mut owned_shortcut = Shortcut::new(
            "0",
            game.title.as_str(),
            &exe,
            &start_dir,
            "",
            "",
            launch.as_str(),
        )
        .to_owned();
        owned_shortcut.tags.push("Origin/EA Desktop".to_owned());
        owned_shortcut.tags.push("Ready TO Play".to_owned());
        owned_shortcut.tags.push("Installed".to_owned());

        owned_shortcut
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_found_by_its_installer_data_opens_through_the_ea_app() {
        let game = EAGame {
            id: "alice2_dd,1002139".to_string(),
            title: "Alice: Madness Returns".to_string(),
            launcher_location: PathBuf::from(r"C:\EA Desktop\EALauncher.exe"),
            launcher_compat_folder: None,
            found_by: FoundBy::InstallerData,
            game_folder: None,
            helper: None,
        };
        let shortcut: ShortcutOwned = game.into();
        assert_eq!(shortcut.exe, "\"C:\\EA Desktop\\EALauncher.exe\"");
        // Only Windows reads `C:\` paths as folders.
        #[cfg(windows)]
        assert_eq!(shortcut.start_dir, "\"C:\\EA Desktop\"");
        assert_eq!(
            shortcut.launch_options,
            "\"origin2://game/launch?offerIds=alice2_dd,1002139\""
        );
        assert!(shortcut.tags.contains(&"Installed".to_string()));
    }

    #[test]
    fn a_game_with_a_known_folder_runs_boilr_to_wait_for_it() {
        let game = EAGame {
            id: "alice2_dd".to_string(),
            title: "Alice: Madness Returns".to_string(),
            launcher_location: PathBuf::from(r"C:\EA Desktop\EALauncher.exe"),
            launcher_compat_folder: None,
            found_by: FoundBy::InstallerData,
            game_folder: Some(PathBuf::from(r"D:\Games\Alice Madness Returns\")),
            helper: Some(PathBuf::from(r"C:\BoilR\BoilR.exe")),
        };
        let shortcut: ShortcutOwned = game.into();
        assert_eq!(shortcut.exe, r#""C:\BoilR\BoilR.exe""#);
        assert_eq!(
            shortcut.launch_options,
            r#"--launch-and-wait --game-folder "D:\Games\Alice Madness Returns" --launcher "C:\EA Desktop\EALauncher.exe" --link "origin2://game/launch?offerIds=alice2_dd""#
        );
    }
}
