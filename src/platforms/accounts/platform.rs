use boilr_stores::{self as stores, OwnedLibrary, StoreId};
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
    // State of the egui settings panel, shared with its background tasks.
    #[cfg(feature = "egui-ui")]
    panel: std::sync::Arc<std::sync::Mutex<Panel>>,
    #[cfg(feature = "egui-ui")]
    pasted: String,
}

impl AccountPlatform {
    pub fn new<S: AsRef<str>>(store: StoreId, settings: S) -> Self {
        AccountPlatform {
            store,
            settings: load_settings(settings),
            #[cfg(feature = "egui-ui")]
            panel: Default::default(),
            #[cfg(feature = "egui-ui")]
            pasted: String::new(),
        }
    }

    /// The cached library, fetched again first when it is older than
    /// [`LIBRARY_MAX_AGE_SECS`]. A failed fetch falls back to the cached library, so a
    /// network problem never removes the account's games from Steam.
    pub fn library(&self) -> eyre::Result<OwnedLibrary> {
        let store = self.store;
        if stores::load_session(store)?.is_none() {
            return Err(eyre::format_err!(
                "Not signed in to {}. Sign in from this source's settings.",
                store.display_name()
            ));
        }
        block_on_fresh(stores::refresh_if_stale(store, LIBRARY_MAX_AGE_SECS))?
    }
}

/// EA lists games the account has through a linked Steam or Epic account too; those belong
/// to that store (Steam has them already, the Epic account adds them).
fn include_game(game: &boilr_stores::OwnedGame) -> bool {
    !matches!(game.extra("access"), Some("steam") | Some("epic"))
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
            .filter(|game| include_game(game))
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

/// What the egui settings panel shows. Sign-in and refresh run on a background thread and
/// report here, so the window never waits on the network.
#[cfg(feature = "egui-ui")]
#[derive(Default)]
struct Panel {
    /// Read from disk once, and again after every change.
    status: Option<stores::AccountStatus>,
    busy: bool,
    message: Option<String>,
    /// A sign-in finished: turn the account on as a source.
    signed_in: bool,
}

#[cfg(feature = "egui-ui")]
fn lock_panel(panel: &std::sync::Mutex<Panel>) -> std::sync::MutexGuard<'_, Panel> {
    panel
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(feature = "egui-ui")]
impl AccountPlatform {
    /// Runs `work` on a background thread; its message shows in the panel when it is done.
    /// `work` returns the message and whether the account is now signed in.
    fn run_in_background<F>(&self, ctx: &egui::Context, work: F)
    where
        F: FnOnce() -> (String, bool) + Send + 'static,
    {
        {
            let mut panel = lock_panel(&self.panel);
            if panel.busy {
                return;
            }
            panel.busy = true;
            panel.message = None;
        }
        let panel = self.panel.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let (message, signed_in) = work();
            {
                let mut panel = lock_panel(&panel);
                panel.busy = false;
                panel.message = Some(message);
                panel.signed_in |= signed_in;
                panel.status = None;
            }
            ctx.request_repaint();
        });
    }

    fn render_account_ui(&mut self, ui: &mut egui::Ui) {
        use boilr_stores::{LoginCapture, ManualLogin};

        let store = self.store;
        let (status, busy, message) = {
            let mut panel = lock_panel(&self.panel);
            if panel.signed_in {
                panel.signed_in = false;
                self.settings.enabled = true;
                self.pasted.clear();
            }
            let status = panel
                .status
                .get_or_insert_with(|| stores::account_status(store))
                .clone();
            (status, panel.busy, panel.message.clone())
        };

        ui.heading(self.name().to_string());
        ui.checkbox(
            &mut self.settings.enabled,
            "Add every game this account owns, installed or not",
        );
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
            if status.needs_sign_in {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "The store no longer accepts this sign-in, so the games listed are from \
                     before. Sign out and sign in again.",
                );
            } else if let Some(error) = &status.last_error {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "The last refresh failed, so the games listed are from before: {error}"
                    ),
                );
            }
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    if ui.button("Refresh library").clicked() {
                        self.run_in_background(ui.ctx(), move || {
                            let message = match block_on_fresh(stores::refresh_library(store)) {
                                Ok(Ok(library)) => format!("Found {} games.", library.games.len()),
                                Ok(Err(err)) | Err(err) => format!("Could not refresh: {err}"),
                            };
                            (message, false)
                        });
                    }
                    if ui.button("Sign out").clicked() {
                        self.run_in_background(ui.ctx(), move || {
                            let message = match block_on_fresh(stores::sign_out(store)) {
                                Ok(Ok(None)) => "Signed out.".to_string(),
                                Ok(Ok(Some(warning))) => warning,
                                Ok(Err(err)) | Err(err) => format!("Could not sign out: {err}"),
                            };
                            (message, false)
                        });
                    }
                });
            });
        } else {
            match &status.manual {
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
                        ui.add_enabled_ui(!busy, |ui| {
                            if ui.button("Sign in").clicked() {
                                // The same pending login as the link above: Amazon's sign-in
                                // link carries its PKCE challenge.
                                let pending = stores::pending_login(store);
                                let capture = LoginCapture {
                                    pasted: Some(self.pasted.trim().to_string()),
                                    ..Default::default()
                                };
                                self.run_in_background(ui.ctx(), move || {
                                    let signed_in = block_on_fresh(async {
                                        let name =
                                            stores::complete_login(&pending, &capture).await?;
                                        if let Err(err) = stores::refresh_library(store).await {
                                            eprintln!(
                                                "Could not fetch the {} library: {err:?}",
                                                store.key()
                                            );
                                        }
                                        Ok::<_, eyre::Report>(name)
                                    })
                                    .and_then(|r| r);
                                    match signed_in {
                                        Ok(name) => (
                                            format!(
                                                "Signed in{}.",
                                                name.map(|n| format!(" as {n}"))
                                                    .unwrap_or_default()
                                            ),
                                            true,
                                        ),
                                        Err(err) => (format!("Could not sign in: {err}"), false),
                                    }
                                });
                            }
                        });
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
        if busy {
            ui.spinner();
        }
        if let Some(message) = &message {
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
