use std::path::{Path, PathBuf};

use boilr_core::launch_and_wait::{LaunchAndWait, OpenFirst, SettingLine};
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
    /// Close an open Ubisoft Connect before the game too, so Steam starts it and its overlay
    /// reaches the game ([`super::settings::UbisoftSettings::steam_overlay`]).
    pub(crate) restart_launcher: bool,
    /// The game's own program, for Steam to start instead of the link
    /// ([`super::settings::UbisoftSettings::start_game_exe`]).
    pub(crate) program: Option<super::catalogue::GameProgram>,
    /// Ubisoft Connect's `settings.yaml`, whose `closebehavior` BoilR changes while it has
    /// Ubisoft Connect running, so that a close request quits it instead of hiding it.
    pub(crate) settings_file: Option<PathBuf>,
}

/// In Ubisoft Connect's log line once it has signed in (`AccountStartupUser.cpp (238) User:
/// <id>`). A game started before that gets "Ownership connection is not set up" (11-5002)
/// and Ubisoft Connect asks to restart and sign in again (user tests, 2026-11-13).
const UBISOFT_CONNECT_READY: &str = "AccountStartupUser.cpp";

/// Ubisoft Connect's log: `logs\launcher_log.txt` in its install folder.
/// Built as text: it's a Windows path, and `Path::parent` doesn't split those on Linux, where
/// the tests run too.
fn launcher_log(upc: &Path) -> PathBuf {
    let upc = upc.to_string_lossy();
    let folder = upc
        .rsplit_once(['\\', '/'])
        .map_or("", |(folder, _)| folder);
    PathBuf::from(format!(r"{folder}\logs\launcher_log.txt"))
}

/// The title of Ubisoft Connect's window: a close request to it quits Ubisoft Connect while
/// `closebehavior` is [`CLOSE_QUITS`].
const UBISOFT_CONNECT_WINDOW: &str = "Ubisoft Connect";
/// Ubisoft Connect's default: its window's close button hides it in the tray.
const CLOSE_HIDES: &str = "closebehavior: CloseBehavior_Minimize";
/// The setting under which closing its window quits Ubisoft Connect.
const CLOSE_QUITS: &str = "closebehavior: CloseBehavior_Close";

impl From<UbisoftGame> for ShortcutOwned {
    fn from(game: UbisoftGame) -> Self {
        let link = format!("uplay://launch/{}/{}", game.id, game.launch_id);
        if let (Some(helper), Some(game_folder), None) = (
            &game.helper,
            &game.game_folder,
            &game.launcher_compat_folder,
        ) {
            let request = match &game.program {
                // Steam starts the game, so its overlay reaches it; Ubisoft Connect is opened
                // first, outside Steam: when the game has to start it, the overlay breaks.
                Some(program) => LaunchAndWait {
                    launcher: program.exe.clone(),
                    game_folder: game_folder.clone(),
                    start_in: Some(program.working_directory.clone()),
                    open_first: Some(OpenFirst {
                        program: game.launcher.clone(),
                        log: launcher_log(&game.launcher),
                        ready: UBISOFT_CONNECT_READY.to_string(),
                    }),
                    ..LaunchAndWait::default()
                },
                // A Ubisoft Connect this launch started is asked to quit after the game: a
                // close request quits it while `closebehavior` says so, which BoilR sets
                // while Ubisoft Connect is closed and sets back once it has exited. It logs
                // no end of its cloud sync, so BoilR waits a fixed time first.
                None => LaunchAndWait {
                    launcher: game.launcher.clone(),
                    link: Some(link),
                    game_folder: game_folder.clone(),
                    restart_launcher: game.restart_launcher,
                    close_window: Some(UBISOFT_CONNECT_WINDOW.to_string()),
                    setting: game.settings_file.as_ref().map(|file| SettingLine {
                        file: file.clone(),
                        from: CLOSE_HIDES.to_string(),
                        to: CLOSE_QUITS.to_string(),
                    }),
                    ..LaunchAndWait::default()
                },
            };
            let target = request.shortcut_target(helper);
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
            program: None,
            settings_file: Some(PathBuf::from(
                r"C:\Users\me\Ubisoft Game Launcher\settings.yaml",
            )),
        }
    }

    #[test]
    fn runs_boilr_to_wait_for_the_game() {
        let shortcut: ShortcutOwned = far_cry_3().into();
        assert_eq!(shortcut.exe, r#""C:\BoilR\BoilR.exe""#);
        assert_eq!(
            shortcut.launch_options,
            r#"--launch-and-wait --game-folder "D:\UplayLibrary\Far Cry 3" --launcher "C:\Ubisoft Game Launcher\upc.exe" --link "uplay://launch/46/0" --close-window "Ubisoft Connect" --setting-file "C:\Users\me\Ubisoft Game Launcher\settings.yaml" --setting-from "closebehavior: CloseBehavior_Minimize" --setting-to "closebehavior: CloseBehavior_Close" --restart-launcher"#
        );
    }

    #[test]
    fn starts_the_games_own_program_when_known() {
        let game = UbisoftGame {
            program: Some(super::super::catalogue::GameProgram {
                exe: PathBuf::from(r"D:\UplayLibrary\Far Cry 3\bin\FC3Updater.exe"),
                working_directory: PathBuf::from(r"D:\UplayLibrary\Far Cry 3"),
            }),
            ..far_cry_3()
        };
        let shortcut: ShortcutOwned = game.into();
        assert_eq!(
            shortcut.launch_options,
            r#"--launch-and-wait --game-folder "D:\UplayLibrary\Far Cry 3" --launcher "D:\UplayLibrary\Far Cry 3\bin\FC3Updater.exe" --start-in "D:\UplayLibrary\Far Cry 3" --open-first "C:\Ubisoft Game Launcher\upc.exe" --open-first-log "C:\Ubisoft Game Launcher\logs\launcher_log.txt" --open-first-ready "AccountStartupUser.cpp""#
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
