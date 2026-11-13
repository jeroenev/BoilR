use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UbisoftSettings {
    pub enabled: bool,
    /// Windows: close an open Ubisoft Connect before each game, so the game's shortcut starts
    /// it inside Steam and Steam's overlay (and Big Picture's Resume) reaches the game.
    /// Without it, the overlay only reaches games started while Ubisoft Connect is closed; an
    /// open Ubisoft Connect takes the game over, outside Steam. Off by default: Ubisoft
    /// Connect can only be ended, which stops its downloads, cuts off a cloud-save upload
    /// that takes longer than the wait after the game, and leaves another Ubisoft game
    /// running at the time without its launcher.
    #[serde(default)]
    pub steam_overlay: bool,
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
        }
    }
}
