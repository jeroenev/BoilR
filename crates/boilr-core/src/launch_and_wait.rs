//! Keeps a Steam shortcut "running" for as long as its game runs.
//!
//! Steam counts a non-Steam shortcut as running while the program it started, or any program
//! that one starts, is alive. Some launchers start their games themselves: the EA app's
//! `EALauncher.exe` hands the game's link to the EA app and exits within seconds, so Steam
//! shows the game as stopped while it runs. Play then starts it a second time, and the
//! controller's Home button finds no game to return to. A shortcut can instead run BoilR in
//! this mode: BoilR starts the launcher with the game's link, as its own child so Steam keeps
//! tracking it, and stays alive while any program from the game's folder runs.
//!
//! With [`LaunchAndWait::restart_launcher`] BoilR also closes the launcher before and after
//! the game. Ubisoft Connect needs this for the Steam overlay: the overlay only reaches a game
//! when Steam started Ubisoft Connect itself, and Steam then counts the game as running until
//! Ubisoft Connect closes. Ubisoft Connect has no way to be asked to quit from outside (a
//! window close request only hides it), so it is ended after a grace period for cloud saves.
//!
//! What happens is written to `launch-and-wait.log` in BoilR's config folder.

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    thread::sleep,
    time::{Duration, Instant},
};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// The command-line flag that starts BoilR in this mode instead of its interface.
pub const FLAG: &str = "--launch-and-wait";
const GAME_FOLDER: &str = "--game-folder";
const LAUNCHER: &str = "--launcher";
const LINK: &str = "--link";
const RESTART_LAUNCHER: &str = "--restart-launcher";

/// How long the launcher gets to start the game: it may log in or update the game first.
const START_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// How long no program from the game's folder may run before the game counts as closed.
/// Some games restart themselves or hand over from a small launcher to the game.
const EXIT_GRACE: Duration = Duration::from_secs(5);
/// How long a restarted launcher gets after the game, to upload cloud saves, before it is
/// closed.
const CLOUD_SAVE_GRACE: Duration = Duration::from_secs(10);
/// How long a launcher that is being closed gets to go away.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_secs(1);

/// The program shortcuts run in this mode: the BoilR that is importing them, on Windows.
/// On Linux, launchers run under Proton, where Steam tracks the whole prefix.
pub fn helper() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::current_exe().ok()
    } else {
        None
    }
}

/// Runs this mode when `args` (the arguments after the program name) ask for it, and logs
/// what went wrong. `None` when they don't: the caller starts its interface instead. Call it
/// before anything else, in particular before taking BoilR's single-instance lock, since
/// the game can run while BoilR is open.
pub fn run_if_asked(args: &[String]) -> Option<eyre::Result<()>> {
    let request = LaunchAndWait::from_args(args)?;
    let result = request.and_then(|request| request.run());
    if let Err(err) = &result {
        Log::open().line(&format!("error: {err}"));
    }
    Some(result)
}

/// One game to start through its launcher and wait for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchAndWait {
    /// The program that starts the game, such as `EALauncher.exe` or `upc.exe`.
    pub launcher: PathBuf,
    /// The launcher's argument: the game's link (`origin2://...`, `uplay://...`).
    pub link: String,
    /// The game's install folder. The game counts as running while a program from it runs.
    pub game_folder: PathBuf,
    /// Close the launcher before the game, so this run starts it, and after the game.
    pub restart_launcher: bool,
}

/// What a Steam shortcut runs: its `Exe`, `StartDir` and `LaunchOptions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutTarget {
    pub exe: String,
    pub start_dir: String,
    pub launch_options: String,
}

impl LaunchAndWait {
    /// The shortcut that runs `helper` (a BoilR executable) in this mode.
    pub fn shortcut_target(&self, helper: &Path) -> ShortcutTarget {
        let mut launch_options = format!(
            "{FLAG} {GAME_FOLDER} {} {LAUNCHER} {} {LINK} {}",
            quoted(&self.game_folder.to_string_lossy()),
            quoted(&self.launcher.to_string_lossy()),
            quoted(&self.link),
        );
        if self.restart_launcher {
            launch_options.push(' ');
            launch_options.push_str(RESTART_LAUNCHER);
        }
        ShortcutTarget {
            exe: quoted(&helper.to_string_lossy()),
            start_dir: helper
                .parent()
                .map(|folder| quoted(&folder.to_string_lossy()))
                .unwrap_or_default(),
            launch_options,
        }
    }

    /// The request in a command line, when it has [`FLAG`]. `args` are the arguments after
    /// the program name.
    pub fn from_args(args: &[String]) -> Option<eyre::Result<Self>> {
        if !args.iter().any(|arg| arg == FLAG) {
            return None;
        }
        let value = |name: &str| {
            args.iter()
                .position(|arg| arg == name)
                .and_then(|at| args.get(at + 1))
                .filter(|value| !value.starts_with("--"))
                .cloned()
                .ok_or_else(|| eyre::format_err!("{FLAG} needs {name} <value>"))
        };
        Some((|| {
            Ok(LaunchAndWait {
                launcher: PathBuf::from(value(LAUNCHER)?),
                link: value(LINK)?,
                game_folder: PathBuf::from(value(GAME_FOLDER)?),
                restart_launcher: args.iter().any(|arg| arg == RESTART_LAUNCHER),
            })
        })())
    }

    /// Starts the game and returns once it has closed (or never started).
    pub fn run(&self) -> eyre::Result<()> {
        let mut log = Log::open();
        log.line(&format!("start: {self:?}"));
        // A drive's root would make every program on the drive count as the game.
        if self.game_folder.parent().is_none() {
            eyre::bail!("{} is not a game folder", self.game_folder.display());
        }
        let game_folder = Folder::new(&self.game_folder);
        let mut processes = Processes::new();
        if processes.any_in(&game_folder) {
            log.line("the game is already running; waiting for it");
        } else {
            if self.restart_launcher {
                self.close_launcher(&mut processes, &mut log);
            }
            let mut command = Command::new(&self.launcher);
            // Quoted, as Steam passes it: `arg` only quotes values with spaces, and
            // explorer.exe ignores an unquoted `origin2://...?offerIds=...` link.
            #[cfg(windows)]
            std::os::windows::process::CommandExt::raw_arg(&mut command, quoted(&self.link));
            #[cfg(not(windows))]
            command.arg(&self.link);
            if let Some(folder) = self.launcher.parent() {
                command.current_dir(folder);
            }
            // Not waited for: EALauncher.exe exits at once, Ubisoft Connect keeps running.
            command.spawn()?;
            let started = Instant::now();
            while !processes.any_in(&game_folder) {
                if started.elapsed() > START_TIMEOUT {
                    log.line("the game did not start; giving up");
                    return Ok(());
                }
                sleep(POLL);
            }
            log.line(&format!("the game started after {:?}", started.elapsed()));
        }

        let mut last_seen = Instant::now();
        while last_seen.elapsed() < EXIT_GRACE {
            sleep(POLL);
            if processes.any_in(&game_folder) {
                last_seen = Instant::now();
            }
        }
        log.line("the game closed");

        if self.restart_launcher {
            sleep(CLOUD_SAVE_GRACE);
            if processes.any_in(&game_folder) {
                log.line("the game started again; leaving the launcher open");
            } else {
                self.close_launcher(&mut processes, &mut log);
            }
        }
        Ok(())
    }

    /// Ends the programs in the launcher's own folder (not in folders below it, where games
    /// can be installed), and waits for them to go.
    fn close_launcher(&self, processes: &mut Processes, log: &mut Log) {
        let Some(folder) = self.launcher.parent() else {
            return;
        };
        // A link opened through Explorer: its folder is Windows' own.
        let through_explorer = self
            .launcher
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("explorer.exe"));
        if through_explorer {
            return;
        }
        let folder = Folder::new(folder);
        let ended = processes.kill_directly_in(&folder);
        if ended == 0 {
            return;
        }
        log.line(&format!("closing the launcher ({ended} programs)"));
        let asked = Instant::now();
        while processes.count_directly_in(&folder) > 0 && asked.elapsed() < CLOSE_TIMEOUT {
            sleep(POLL);
            processes.kill_directly_in(&folder);
        }
    }
}

/// `value` in double quotes for a Windows command line. Quotes inside are dropped, and so
/// are trailing backslashes: `\"` would escape the closing quote.
fn quoted(value: &str) -> String {
    let value = value.replace('"', "");
    format!("\"{}\"", value.trim_end_matches(['\\', '/']))
}

/// A path for comparing: lower case (Windows paths ignore case) with `\` separators, without
/// a trailing separator.
fn comparable(path: &Path) -> String {
    path.to_string_lossy()
        .to_lowercase()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_string()
}

/// `path` without the `\\?\` prefix that `std::fs::canonicalize` adds on Windows; programs'
/// paths come without it.
fn without_verbatim_prefix(path: String) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\unc\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path
    }
}

/// A folder to find programs in, as given and as it really is. Windows reports a program's
/// path through the real folder when the given one is a link: the EA app's
/// `EA Desktop\EA Desktop` links to a folder named after its version.
struct Folder {
    names: Vec<String>,
}

impl Folder {
    fn new(path: &Path) -> Self {
        let mut names = vec![comparable(path)];
        if let Ok(real) = std::fs::canonicalize(path) {
            let real = without_verbatim_prefix(comparable(&real));
            if !names.contains(&real) {
                names.push(real);
            }
        }
        Folder { names }
    }

    /// Whether `exe` is in this folder or a folder below it.
    fn holds(&self, exe: &Path) -> bool {
        let exe = comparable(exe);
        self.names.iter().any(|folder| {
            exe.strip_prefix(folder.as_str())
                .is_some_and(|rest| rest.starts_with('\\'))
        })
    }

    /// Whether `exe` is in this folder itself.
    fn holds_directly(&self, exe: &Path) -> bool {
        comparable(exe)
            .rsplit_once('\\')
            .is_some_and(|(parent, _)| self.names.iter().any(|folder| folder == parent))
    }
}

/// The programs running on this computer, by the path of their executable.
struct Processes {
    system: System,
}

impl Processes {
    fn new() -> Self {
        Processes {
            system: System::new(),
        }
    }

    fn refresh(&mut self) {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
        );
    }

    fn any_in(&mut self, folder: &Folder) -> bool {
        self.refresh();
        self.system
            .processes()
            .values()
            .any(|process| process.exe().is_some_and(|exe| folder.holds(exe)))
    }

    fn count_directly_in(&mut self, folder: &Folder) -> usize {
        self.refresh();
        self.system
            .processes()
            .values()
            .filter(|process| process.exe().is_some_and(|exe| folder.holds_directly(exe)))
            .count()
    }

    /// Ends the programs in `folder` itself; returns how many there were.
    fn kill_directly_in(&mut self, folder: &Folder) -> usize {
        self.refresh();
        let mut found = 0;
        for process in self.system.processes().values() {
            if process.exe().is_some_and(|exe| folder.holds_directly(exe)) {
                process.kill();
                found += 1;
            }
        }
        found
    }
}

/// `launch-and-wait.log` in BoilR's config folder. This mode has no window to show errors
/// in. The log starts over once it grows past 256 KB.
struct Log {
    file: Option<std::fs::File>,
}

impl Log {
    fn open() -> Self {
        let folder = crate::config::get_config_folder();
        let _ = std::fs::create_dir_all(&folder);
        let path = folder.join("launch-and-wait.log");
        let too_big = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 256 * 1024);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(!too_big)
            .write(true)
            .truncate(too_big)
            .open(path)
            .ok();
        Log { file }
    }

    fn line(&mut self, message: &str) {
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "{now} {message}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &[&str]) -> Vec<String> {
        line.iter().map(|arg| arg.to_string()).collect()
    }

    fn alice() -> LaunchAndWait {
        LaunchAndWait {
            launcher: PathBuf::from(r"C:\EA Desktop\EALauncher.exe"),
            link: "origin2://game/launch?offerIds=alice2_dd".to_string(),
            game_folder: PathBuf::from(r"D:\OriginLibrary\Alice Madness Returns\"),
            restart_launcher: false,
        }
    }

    #[test]
    fn the_shortcut_runs_boilr_with_quoted_values() {
        let target = alice().shortcut_target(Path::new(r"C:\BoilR\BoilR.exe"));
        assert_eq!(target.exe, r#""C:\BoilR\BoilR.exe""#);
        assert_eq!(
            target.launch_options,
            r#"--launch-and-wait --game-folder "D:\OriginLibrary\Alice Madness Returns" --launcher "C:\EA Desktop\EALauncher.exe" --link "origin2://game/launch?offerIds=alice2_dd""#
        );
        // Only Windows reads `C:\` paths as folders.
        #[cfg(windows)]
        assert_eq!(target.start_dir, r#""C:\BoilR""#);

        let ubisoft = LaunchAndWait {
            restart_launcher: true,
            ..alice()
        };
        assert!(ubisoft
            .shortcut_target(Path::new(r"C:\BoilR\BoilR.exe"))
            .launch_options
            .ends_with(" --restart-launcher"));
    }

    #[test]
    fn reads_its_command_line() -> eyre::Result<()> {
        // As Windows splits the shortcut's command line: the quotes are gone.
        let line = args(&[
            "--launch-and-wait",
            "--game-folder",
            r"D:\OriginLibrary\Alice Madness Returns",
            "--launcher",
            r"C:\EA Desktop\EALauncher.exe",
            "--link",
            "origin2://game/launch?offerIds=alice2_dd",
        ]);
        let request = LaunchAndWait::from_args(&line).ok_or(eyre::format_err!("no request"))??;
        assert_eq!(
            request.launcher,
            PathBuf::from(r"C:\EA Desktop\EALauncher.exe")
        );
        assert_eq!(request.link, "origin2://game/launch?offerIds=alice2_dd");
        assert_eq!(
            request.game_folder,
            PathBuf::from(r"D:\OriginLibrary\Alice Madness Returns")
        );
        assert!(!request.restart_launcher);

        let mut restart = line.clone();
        restart.push("--restart-launcher".to_string());
        let restart =
            LaunchAndWait::from_args(&restart).ok_or(eyre::format_err!("no request"))??;
        assert!(restart.restart_launcher);

        assert!(LaunchAndWait::from_args(&args(&["--no-ui"])).is_none());
        let missing_link = args(&["--launch-and-wait", "--game-folder", "D:\\Game", "--link"]);
        assert!(matches!(
            LaunchAndWait::from_args(&missing_link),
            Some(Err(_))
        ));
        Ok(())
    }

    #[test]
    fn programs_belong_to_folders_by_path() {
        let game = Folder::new(Path::new(r"D:\UplayLibrary\Far Cry 3\"));
        assert!(game.holds(Path::new(r"d:\uplaylibrary\far cry 3\bin\farcry3.exe")));
        assert!(!game.holds(Path::new(
            r"D:\UplayLibrary\Far Cry 3 Blood Dragon\bin\fc3.exe"
        )));
        assert!(!game.holds(Path::new(r"D:\UplayLibrary\Far Cry 3")));

        let launcher = Folder::new(Path::new(
            r"C:\Program Files (x86)\Ubisoft\Ubisoft Game Launcher",
        ));
        assert!(launcher.holds_directly(Path::new(
            r"C:\Program Files (x86)\Ubisoft\Ubisoft Game Launcher\upc.exe"
        )));
        // Games can be installed below the launcher's folder; those are not the launcher.
        assert!(!launcher.holds_directly(Path::new(
            r"C:\Program Files (x86)\Ubisoft\Ubisoft Game Launcher\games\Far Cry 3\bin\farcry3.exe"
        )));
    }

    #[test]
    fn a_folder_also_matches_where_it_really_is() -> eyre::Result<()> {
        // "." stands in for a link: the given name differs from the real path.
        let real = std::env::current_dir()?;
        let folder = Folder::new(Path::new("."));
        assert!(folder.holds(&real.join("bin").join("game.exe")));
        assert!(folder.holds_directly(&real.join("launcher.exe")));
        Ok(())
    }

    #[test]
    fn canonical_paths_lose_their_verbatim_prefix() {
        assert_eq!(
            without_verbatim_prefix(r"\\?\c:\program files\ea".to_string()),
            r"c:\program files\ea"
        );
        assert_eq!(
            without_verbatim_prefix(r"\\?\unc\server\share\ea".to_string()),
            r"\\server\share\ea"
        );
        assert_eq!(without_verbatim_prefix(r"c:\ea".to_string()), r"c:\ea");
    }
}
