use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UbisoftSettings {
    pub enabled: bool,
    /// Windows: close an open Ubisoft Connect before each game, so the game's shortcut starts
    /// it inside Steam and Steam's overlay (and Big Picture's Resume) reaches the game.
    /// Without it, the overlay only reaches games started while Ubisoft Connect is closed; an
    /// open Ubisoft Connect takes the game over, outside Steam. Off by default: an open
    /// Ubisoft Connect can only be ended (it hides itself when asked to close), which stops
    /// its downloads and leaves another Ubisoft game running at the time without its
    /// launcher.
    ///
    /// Either way, a Ubisoft Connect that a game's shortcut started is asked to quit 15
    /// seconds after the game (its `closebehavior` is set to quit for that while), and ended
    /// if it doesn't. Its quit doesn't wait for cloud-save uploads; a cut-off upload is
    /// probably retried when that game next starts.
    #[serde(default)]
    pub steam_overlay: bool,
    /// Windows: let Steam start the game's own program, found in Ubisoft Connect's game
    /// catalogue, instead of the game's `uplay://` link. The overlay then reaches the game
    /// whether Ubisoft Connect is open or not, Ubisoft Connect is never closed, and Steam
    /// shows Play again seconds after the game. Off by default: it reads Ubisoft Connect's
    /// encrypted catalogue cache, whose format can change without notice, and newer games
    /// may refuse to start this way (tried with Far Cry 3, Far Cry 4 and Assassin's Creed
    /// IV). Games whose program isn't found, or that need arguments, keep the link. Takes
    /// precedence over [`Self::steam_overlay`] for the games it finds.
    #[serde(default)]
    pub start_game_exe: bool,
}

impl Default for UbisoftSettings {
    fn default() -> Self {
        #[cfg(target_family = "unix")]
        let enabled = false;
        #[cfg(target_family = "windows")]
        let enabled = true;
        Self {
            enabled,
            steam_overlay: false,
            start_game_exe: false,
        }
    }
}
