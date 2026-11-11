use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct UbisoftSettings {
    pub enabled: bool,
    /// Windows: close Ubisoft Connect before and after each game, so Steam starts it from the
    /// game's shortcut. Steam's overlay (and Big Picture's Resume) only reaches a Ubisoft game
    /// when Steam started Ubisoft Connect itself. Off by default: it ends Ubisoft Connect,
    /// downloads included, and gives cloud saves only a grace period after the game.
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
