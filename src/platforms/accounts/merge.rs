use std::collections::HashSet;

use crate::platforms::ShortcutToImport;
use crate::sync::identity_of;

/// The Steam collection that holds owned games that are not installed.
pub const NOT_INSTALLED_COLLECTION: &str = "Not installed";

/// Drops owned (not installed) entries for games that an installed-games platform also
/// reports: the installed shortcut wins, and the sync keeps the app id Steam knew the owned
/// entry by. Also drops repeated owned entries of the same game.
pub fn drop_owned_duplicates<'a, I>(lists: I)
where
    I: IntoIterator<Item = &'a mut Vec<ShortcutToImport>>,
{
    let mut lists: Vec<&mut Vec<ShortcutToImport>> = lists.into_iter().collect();
    let installed: HashSet<String> = lists
        .iter()
        .flat_map(|list| list.iter())
        .filter(|entry| entry.installed)
        .filter_map(|entry| identity_of(&entry.shortcut).map(str::to_string))
        .collect();
    let mut seen_owned: HashSet<String> = HashSet::new();
    for list in lists.iter_mut() {
        list.retain(|entry| {
            if entry.installed {
                return true;
            }
            match identity_of(&entry.shortcut) {
                Some(identity) => {
                    !installed.contains(identity) && seen_owned.insert(identity.to_string())
                }
                None => true,
            }
        });
    }
}

/// The "Not installed" collection: every entry that is not installed, by its platform app id.
pub fn not_installed_collection<'a, I>(lists: I) -> Option<(String, Vec<u32>)>
where
    I: IntoIterator<Item = &'a Vec<ShortcutToImport>>,
{
    let ids: Vec<u32> = lists
        .into_iter()
        .flat_map(|list| list.iter())
        .filter(|entry| !entry.installed)
        .map(|entry| entry.shortcut.app_id)
        .collect();
    (!ids.is_empty()).then(|| (NOT_INSTALLED_COLLECTION.to_string(), ids))
}

#[cfg(test)]
mod tests {
    use super::*;
    use steam_shortcuts_util::Shortcut;

    fn entry(name: &str, identity: Option<&str>, installed: bool) -> ShortcutToImport {
        let mut shortcut = Shortcut::new("0", name, name, "", "", "", "").to_owned();
        if let Some(identity) = identity {
            shortcut.dev_kit_game_id = identity.to_string();
        }
        ShortcutToImport {
            shortcut,
            needs_proton: false,
            needs_symlinks: false,
            installed,
        }
    }

    fn names(list: &[ShortcutToImport]) -> Vec<&str> {
        list.iter().map(|e| e.shortcut.app_name.as_str()).collect()
    }

    #[test]
    fn installed_copy_replaces_the_owned_entry() {
        let mut installed = vec![entry("Installed", Some("boilr:gog:1"), true)];
        let mut owned = vec![
            entry("Owned copy", Some("boilr:gog:1"), false),
            entry("Only owned", Some("boilr:gog:2"), false),
            entry("Only owned again", Some("boilr:gog:2"), false),
            entry("No identity", None, false),
        ];

        drop_owned_duplicates([&mut installed, &mut owned]);

        assert_eq!(names(&installed), ["Installed"]);
        assert_eq!(names(&owned), ["Only owned", "No identity"]);
    }

    #[test]
    fn collection_lists_only_not_installed_games() {
        let installed = vec![entry("Installed", None, true)];
        let owned = vec![entry("Owned", Some("boilr:epic:x"), false)];
        let owned_id = owned.first().map(|e| e.shortcut.app_id).unwrap_or_default();

        let collection = not_installed_collection([&installed, &owned]);

        assert_eq!(
            collection,
            Some((NOT_INSTALLED_COLLECTION.to_string(), vec![owned_id]))
        );
        assert_eq!(not_installed_collection([&installed]), None);
    }
}
