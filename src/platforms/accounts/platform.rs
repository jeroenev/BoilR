use boilr_stores::{self as stores, OwnedLibrary, PendingLogin, StoreId};
use serde::{Deserialize, Serialize};

use super::{
    account_code_name, block_on_fresh,
    targets::{owned_shortcut, Launchers},
    LIBRARY_MAX_AGE_SECS,
};
use crate::platforms::{load_settings, GamesPlatform, ShortcutToImport};

/// What the shortcut of an owned, not installed game opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallWith {
    /// The store's launcher when it is installed, then Heroic (Linux), then the store page.
    #[default]
    Auto,
    /// Heroic first (Epic, GOG and Amazon games on Linux).
    Heroic,
    /// Always the store page in the browser.
    Browser,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccountSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub install_with: InstallWith,
}

#[derive(Clone)]
pub struct AccountPlatform {
    pub store: StoreId,
    pub settings: AccountSettings,
    // Sign-in state of the egui settings panel.
    #[cfg_attr(not(feature = "egui-ui"), allow(dead_code))]
    pending: Option<PendingLogin>,
    #[cfg_attr(not(feature = "egui-ui"), allow(dead_code))]
    pasted: String,
    #[cfg_attr(not(feature = "egui-ui"), allow(dead_code))]
    message: Option<String>,
}

impl AccountPlatform {
    pub fn new<S: AsRef<str>>(store: StoreId, settings: S) -> Self {
        AccountPlatform {
            store,
            settings: load_settings(settings),
            pending: None,
            pasted: String::new(),
            message: None,
        }
    }

    /// The cached library, fetched again first when it is older than [`LIBRARY_MAX_AGE_SECS`].
    /// A failed fetch falls back to the cached library, so a network problem never removes
    /// the account's games from Steam.
    pub fn library(&self) -> eyre::Result<OwnedLibrary> {
        let store = self.store;
        let cached = stores::load_library(store)?;
        let signed_in = stores::load_session(store)?.is_some();
        if !signed_in {
            return Err(eyre::format_err!(
                "Not signed in to {}. Sign in from this source's settings.",
                store.display_name()
            ));
        }
        if stores::is_stale(cached.as_ref(), LIBRARY_MAX_AGE_SECS) {
            let fetched = block_on_fresh(stores::refresh_library(store)).and_then(|r| r);
            match (fetched, cached) {
                (Ok(library), _) => return Ok(library),
                (Err(err), Some(cached)) => {
                    eprintln!(
                        "Could not refresh the {} library, using the one from before: {err:?}",
                        store.display_name()
                    );
                    return Ok(cached);
                }
                (Err(err), None) => return Err(err),
            }
        }
        cached.ok_or_else(|| eyre::format_err!("No library fetched yet"))
    }
}

impl GamesPlatform for AccountPlatform {
    fn name(&self) -> &str {
        match self.store {
            StoreId::Epic => "Epic Games account",
            StoreId::Gog => "GOG account",
            StoreId::Amazon => "Amazon Games account",
            StoreId::Ea => "EA account",
            StoreId::BattleNet => "Battle.net account",
            StoreId::Humble => "Humble Bundle account",
            StoreId::Itch => "itch.io account",
        }
    }

    fn code_name(&self) -> &str {
        account_code_name(self.store)
    }

    fn enabled(&self) -> bool {
        self.settings.enabled
    }

    fn get_shortcut_info(&self) -> eyre::Result<Vec<ShortcutToImport>> {
        let library = self.library()?;
        let launchers = Launchers::detect();
        Ok(library
            .games
            .iter()
            .map(|game| ShortcutToImport {
                shortcut: owned_shortcut(game, self.settings.install_with, &launchers),
                needs_proton: false,
                needs_symlinks: false,
                installed: false,
            })
            .collect())
    }

    fn get_settings_serializable(&self) -> String {
        toml::to_string(&self.settings).unwrap_or_default()
    }

    #[cfg(feature = "egui-ui")]
    fn render_ui(&mut self, ui: &mut egui::Ui) {
        self.render_account_ui(ui);
    }
}

#[cfg(feature = "egui-ui")]
impl AccountPlatform {
    fn render_account_ui(&mut self, ui: &mut egui::Ui) {
        use boilr_stores::{LoginCapture, ManualLogin};

        let store = self.store;
        ui.heading(self.name().to_string());
        ui.checkbox(
            &mut self.settings.enabled,
            "Add every game this account owns, installed or not",
        );
        let status = stores::account_status(store);
        if status.signed_in {
            let who = status
                .account_name
                .clone()
                .unwrap_or_else(|| "your account".to_string());
            let games = match (status.game_count, status.fetched_at) {
                (Some(count), Some(_)) => format!("{count} games in the library"),
                _ => "library not fetched yet".to_string(),
            };
            ui.label(format!("Signed in as {who}, {games}."));
            ui.horizontal(|ui| {
                if ui.button("Refresh library").clicked() {
                    self.message = Some(match block_on_fresh(stores::refresh_library(store)) {
                        Ok(Ok(library)) => format!("Found {} games.", library.games.len()),
                        Ok(Err(err)) | Err(err) => format!("Could not refresh: {err}"),
                    });
                }
                if ui.button("Sign out").clicked() {
                    self.message = Some(match stores::delete_account(store) {
                        Ok(()) => "Signed out.".to_string(),
                        Err(err) => format!("Could not sign out: {err}"),
                    });
                }
            });
        } else {
            let pending = self
                .pending
                .get_or_insert_with(|| stores::begin_login(store))
                .clone();
            match &pending.manual {
                ManualLogin::Paste {
                    url,
                    instructions,
                    placeholder,
                } => {
                    ui.hyperlink_to("Open the sign-in page in your browser", url);
                    ui.label(instructions);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.pasted)
                                .hint_text(placeholder.as_str()),
                        );
                        if ui.button("Sign in").clicked() {
                            let capture = LoginCapture {
                                pasted: Some(self.pasted.trim().to_string()),
                                ..Default::default()
                            };
                            let result = block_on_fresh(stores::complete_login(&pending, &capture))
                                .and_then(|r| r);
                            self.message = Some(match result {
                                Ok(name) => {
                                    self.pasted.clear();
                                    self.pending = None;
                                    self.settings.enabled = true;
                                    format!(
                                        "Signed in{}.",
                                        name.map(|n| format!(" as {n}")).unwrap_or_default()
                                    )
                                }
                                Err(err) => format!("Could not sign in: {err}"),
                            });
                        }
                    });
                }
                ManualLogin::Unsupported => {
                    ui.label(format!(
                        "{} needs its own sign-in window. Sign in from the BoilR 2 app; this app then uses that sign-in too.",
                        store.display_name()
                    ));
                }
            }
        }
        egui::ComboBox::from_label("Games that are not installed open")
            .selected_text(match self.settings.install_with {
                InstallWith::Auto => "the store's launcher, or its web page",
                InstallWith::Heroic => "Heroic first",
                InstallWith::Browser => "the store's web page",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.settings.install_with,
                    InstallWith::Auto,
                    "the store's launcher, or its web page",
                );
                if cfg!(not(windows)) {
                    ui.selectable_value(
                        &mut self.settings.install_with,
                        InstallWith::Heroic,
                        "Heroic first",
                    );
                }
                ui.selectable_value(
                    &mut self.settings.install_with,
                    InstallWith::Browser,
                    "the store's web page",
                );
            });
        if let Some(message) = &self.message {
            ui.label(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_to_disabled_and_survive_unknown_fields() {
        let platform = AccountPlatform::new(StoreId::Gog, "");
        assert!(!platform.enabled());
        let platform = AccountPlatform::new(StoreId::Gog, "enabled = true\nfuture = 1\n");
        assert!(platform.enabled());
        assert_eq!(platform.settings.install_with, InstallWith::Auto);
        let platform =
            AccountPlatform::new(StoreId::Gog, "enabled = true\ninstall_with = \"browser\"\n");
        assert_eq!(platform.settings.install_with, InstallWith::Browser);
    }
}
