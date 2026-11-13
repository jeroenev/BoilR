//! Ubisoft Connect's game catalogue: which program starts each game.
//!
//! Ubisoft Connect keeps each game's launch data (YAML) in
//! `%LOCALAPPDATA%\Ubisoft Game Launcher\cache\configuration\configurations.vdat`, encrypted
//! with AES-256-GCM. The key is in `store\<key id>.bin` next to it, protected for the current
//! Windows user (DPAPI). The decrypted file is a list of records: product id (the number in
//! the registry's `Launcher\Installs\<id>` and in `uplay://launch/<id>`), and its YAML. See
//! `docs/launcher-investigations/ubisoft-connect.md` ("Exe paths").
//!
//! Everything here fails soft: a game whose program isn't found keeps its `uplay://` link.
//!
//! Only Windows reads the catalogue (`game_programs`); elsewhere the parsing below is only
//! used by the tests, which still run there.
#![cfg_attr(not(windows), allow(dead_code))]

/// A game's own program, as Ubisoft Connect starts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GameProgram {
    pub(crate) exe: std::path::PathBuf,
    pub(crate) working_directory: std::path::PathBuf,
}

/// Where a path comes from in the YAML: written out, or the value of a registry entry.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Relative(String),
    Register(String),
}

/// The first executable in a game's `start_game.online.executables`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Executable {
    path: Source,
    /// The registry entry holding the folder it starts in, and what to add to that.
    working_directory: Option<(String, Option<String>)>,
    /// `arguments:`, written out.
    arguments: Option<String>,
    /// `arguments_register:`, the registry entry holding the arguments.
    arguments_register: Option<String>,
}

/// One field of a protobuf message: its number and value (other wire types are skipped).
enum Field<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}

fn varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

/// The fields of a protobuf message; `None` when it isn't one.
fn fields(bytes: &[u8]) -> Option<Vec<(u64, Field<'_>)>> {
    let mut at = 0;
    let mut found = vec![];
    while at < bytes.len() {
        let tag = varint(bytes, &mut at)?;
        let value = match tag & 7 {
            0 => Field::Number(varint(bytes, &mut at)?),
            2 => {
                let length = usize::try_from(varint(bytes, &mut at)?).ok()?;
                let end = at.checked_add(length)?;
                let value = bytes.get(at..end)?;
                at = end;
                Field::Bytes(value)
            }
            1 => {
                at = at.checked_add(8)?;
                continue;
            }
            5 => {
                at = at.checked_add(4)?;
                continue;
            }
            _ => return None,
        };
        found.push((tag >> 3, value));
    }
    Some(found)
}

fn bytes_of<'a>(fields: &[(u64, Field<'a>)], number: u64) -> Option<&'a [u8]> {
    fields.iter().find_map(|(at, field)| match field {
        Field::Bytes(bytes) if *at == number => Some(*bytes),
        _ => None,
    })
}

fn number_of(fields: &[(u64, Field<'_>)], number: u64) -> Option<u64> {
    fields.iter().find_map(|(at, field)| match field {
        Field::Number(value) if *at == number => Some(*value),
        _ => None,
    })
}

/// The encrypted catalogue: `{1: {1: header {2: {2: key id, 3: nonce, 4: tag}}, 2: data}}`.
#[derive(Debug, PartialEq, Eq)]
struct Encrypted<'a> {
    key_id: String,
    nonce: &'a [u8],
    tag: &'a [u8],
    data: &'a [u8],
}

fn encrypted(file: &[u8]) -> Option<Encrypted<'_>> {
    let outer = fields(bytes_of(&fields(file)?, 1)?)?;
    let header = fields(bytes_of(&outer, 1)?)?;
    let encryption = fields(bytes_of(&header, 2)?)?;
    Some(Encrypted {
        key_id: String::from_utf8(bytes_of(&encryption, 2)?.to_vec()).ok()?,
        nonce: bytes_of(&encryption, 3)?,
        tag: bytes_of(&encryption, 4)?,
        data: bytes_of(&outer, 2)?,
    })
}

/// The decrypted catalogue's records: product id and YAML.
fn records(plain: &[u8]) -> Vec<(u64, String)> {
    let Some(top) = fields(plain) else {
        return vec![];
    };
    top.iter()
        .filter_map(|(number, field)| match field {
            Field::Bytes(record) if *number == 1 => fields(record),
            _ => None,
        })
        .filter_map(|record| {
            let id = number_of(&record, 1)?;
            let yaml = String::from_utf8_lossy(bytes_of(&record, 3)?).into_owned();
            Some((id, yaml))
        })
        .collect()
}

/// A YAML line: its indent and its text without the indent.
type Line<'a> = (usize, &'a str);

fn indent(line: &str) -> Line<'_> {
    let text = line.trim_start();
    (line.len() - text.len(), text.trim_end())
}

/// The lines below `lines[at]` that are indented further, or are list items ("- ") at the
/// same indent: YAML allows both for a list.
fn children<'a>(lines: &[Line<'a>], at: usize) -> Vec<Line<'a>> {
    let Some((own, _)) = lines.get(at) else {
        return vec![];
    };
    lines
        .iter()
        .skip(at + 1)
        .take_while(|(indent, text)| {
            text.is_empty() || indent > own || (indent == own && text.starts_with("- "))
        })
        .filter(|(_, text)| !text.is_empty())
        .copied()
        .collect()
}

/// The lines of the block `key:` among `lines` at their first line's indent.
fn block<'a>(lines: &[Line<'a>], key: &str) -> Option<Vec<Line<'a>>> {
    let top = lines.first()?.0;
    let at = lines
        .iter()
        .position(|(indent, text)| *indent == top && text.strip_suffix(':') == Some(key))?;
    Some(children(lines, at))
}

/// The value of `key: value` among `lines` at their first line's indent, without quotes;
/// `None` when it is missing or empty.
fn value(lines: &[Line<'_>], key: &str) -> Option<String> {
    let top = lines.first()?.0;
    lines.iter().find_map(|(indent, text)| {
        let value = text.strip_prefix(key)?.strip_prefix(':')?.trim();
        let value = value.trim_matches('"');
        (*indent == top && !value.is_empty()).then(|| value.to_string())
    })
}

/// The first executable of `start_game: online: executables:` in a game's YAML.
fn first_executable(yaml: &str) -> Option<Executable> {
    let lines: Vec<Line> = yaml.lines().map(indent).collect();
    let start = lines.iter().position(|(_, text)| *text == "start_game:")?;
    let online = block(&children(&lines, start), "online")?;
    let executables = block(&online, "executables")?;
    // The first item: "- key: ..." and the lines indented like that key, or further.
    let (dash, first) = executables.first()?;
    let first = first.strip_prefix("- ")?;
    let own = dash + 2;
    let mut item = vec![(own, first)];
    item.extend(
        executables
            .iter()
            .skip(1)
            .take_while(|(indent, _)| *indent >= own),
    );

    let path = block(&item, "path")?;
    let path = match (value(&path, "relative"), value(&path, "register")) {
        (Some(relative), _) => Source::Relative(relative),
        (None, Some(register)) => Source::Register(register),
        (None, None) => return None,
    };
    let working_directory = block(&item, "working_directory")
        .and_then(|lines| Some((value(&lines, "register")?, value(&lines, "append"))));
    Some(Executable {
        path,
        working_directory,
        arguments: value(&item, "arguments"),
        arguments_register: value(&item, "arguments_register"),
    })
}

/// A path from the catalogue or the registry, with Windows separators.
fn windows_path(path: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(path.replace('/', "\\"))
}

impl Executable {
    /// Where the program is and the folder it starts in, given a way to read registry
    /// entries. `None` for programs that need arguments: the shortcut can't pass them.
    fn locate(&self, read: impl Fn(&str) -> Option<String>) -> Option<GameProgram> {
        if self.arguments.is_some() {
            return None;
        }
        let has_arguments = self
            .arguments_register
            .as_deref()
            .and_then(&read)
            .is_some_and(|arguments| !arguments.trim().is_empty());
        if has_arguments {
            return None;
        }
        let (register, append) = self.working_directory.as_ref()?;
        let mut working_directory = windows_path(&read(register)?);
        if let Some(append) = append {
            working_directory.push(windows_path(append));
        }
        let exe = match &self.path {
            Source::Relative(path) => working_directory.join(windows_path(path)),
            Source::Register(register) => working_directory.join(windows_path(&read(register)?)),
        };
        Some(GameProgram {
            exe,
            working_directory,
        })
    }
}

/// The programs of the games in Ubisoft Connect's catalogue, by product id. Empty when it
/// can't be read.
#[cfg(windows)]
pub(crate) fn game_programs() -> std::collections::HashMap<String, GameProgram> {
    let Some(local) = std::env::var_os("LOCALAPPDATA") else {
        return Default::default();
    };
    let launcher = std::path::Path::new(&local).join("Ubisoft Game Launcher");
    let Ok(file) = std::fs::read(
        launcher
            .join("cache")
            .join("configuration")
            .join("configurations.vdat"),
    ) else {
        return Default::default();
    };
    let Some(encrypted) = encrypted(&file) else {
        return Default::default();
    };
    // The key id names a file: only plain names.
    if !encrypted
        .key_id
        .chars()
        .all(|char| char.is_ascii_alphanumeric())
    {
        return Default::default();
    }
    let plain = std::fs::read(
        launcher
            .join("store")
            .join(format!("{}.bin", encrypted.key_id)),
    )
    .ok()
    .and_then(|blob| windows::unprotect(&blob))
    .and_then(|key| windows::decrypt_aes_gcm(&key, encrypted.nonce, encrypted.tag, encrypted.data));
    let Some(plain) = plain else {
        return Default::default();
    };
    records(&plain)
        .into_iter()
        .filter_map(|(id, yaml)| {
            let program = first_executable(&yaml)?.locate(windows::registry_value)?;
            program.exe.is_file().then(|| (id.to_string(), program))
        })
        .collect()
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        BCryptCloseAlgorithmProvider, BCryptDecrypt, BCryptDestroyKey, BCryptGenerateSymmetricKey,
        BCryptOpenAlgorithmProvider, BCryptSetProperty, CryptUnprotectData, BCRYPT_AES_ALGORITHM,
        BCRYPT_ALG_HANDLE, BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO,
        BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION, BCRYPT_CHAINING_MODE, BCRYPT_KEY_HANDLE,
        CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    /// `blob` unprotected for the current Windows user (DPAPI).
    pub(super) fn unprotect(blob: &[u8]) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(blob.len()).ok()?,
            pbData: blob.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        // SAFETY: `input` points at `blob`, which outlives the call; Windows only reads it.
        let done = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if done == 0 || output.pbData.is_null() {
            return None;
        }
        // SAFETY: Windows allocated `cbData` bytes at `pbData`; copied, then freed once.
        let key = unsafe {
            let key = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            LocalFree(output.pbData as *mut c_void);
            key
        };
        Some(key)
    }

    /// `data` decrypted with AES-256-GCM (Windows' CNG), checked against `tag`.
    pub(super) fn decrypt_aes_gcm(
        key: &[u8],
        nonce: &[u8],
        tag: &[u8],
        data: &[u8],
    ) -> Option<Vec<u8>> {
        let mut algorithm: BCRYPT_ALG_HANDLE = std::ptr::null_mut();
        // SAFETY: a valid out pointer and Windows' own algorithm name.
        if unsafe {
            BCryptOpenAlgorithmProvider(&mut algorithm, BCRYPT_AES_ALGORITHM, std::ptr::null(), 0)
        } != 0
        {
            return None;
        }
        let plain = decrypt_with(algorithm, key, nonce, tag, data);
        // SAFETY: opened above, closed once.
        unsafe { BCryptCloseAlgorithmProvider(algorithm, 0) };
        plain
    }

    fn decrypt_with(
        algorithm: BCRYPT_ALG_HANDLE,
        key: &[u8],
        nonce: &[u8],
        tag: &[u8],
        data: &[u8],
    ) -> Option<Vec<u8>> {
        let mode: Vec<u16> = "ChainingModeGCM\0".encode_utf16().collect();
        // SAFETY: `mode` is a NUL-terminated UTF-16 string of the length given.
        let set = unsafe {
            BCryptSetProperty(
                algorithm,
                BCRYPT_CHAINING_MODE,
                mode.as_ptr() as *const u8,
                u32::try_from(mode.len() * 2).ok()?,
                0,
            )
        };
        if set != 0 {
            return None;
        }
        let mut handle: BCRYPT_KEY_HANDLE = std::ptr::null_mut();
        // SAFETY: `key` is read for the length given; Windows allocates the key object.
        let made = unsafe {
            BCryptGenerateSymmetricKey(
                algorithm,
                &mut handle,
                std::ptr::null_mut(),
                0,
                key.as_ptr(),
                u32::try_from(key.len()).ok()?,
                0,
            )
        };
        if made != 0 {
            return None;
        }
        let mut nonce = nonce.to_vec();
        let mut tag = tag.to_vec();
        let info = BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
            cbSize: std::mem::size_of::<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO>() as u32,
            dwInfoVersion: BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION,
            pbNonce: nonce.as_mut_ptr(),
            cbNonce: u32::try_from(nonce.len()).ok()?,
            pbTag: tag.as_mut_ptr(),
            cbTag: u32::try_from(tag.len()).ok()?,
            ..Default::default()
        };
        let mut plain = vec![0u8; data.len()];
        let mut written = 0u32;
        // SAFETY: every buffer lives until the call returns and its length is passed along;
        // `info` points at `nonce` and `tag`, which outlive the call.
        let decrypted = unsafe {
            BCryptDecrypt(
                handle,
                data.as_ptr(),
                u32::try_from(data.len()).ok()?,
                &info as *const BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO as *const c_void,
                std::ptr::null_mut(),
                0,
                plain.as_mut_ptr(),
                u32::try_from(plain.len()).ok()?,
                &mut written,
                0,
            )
        };
        // SAFETY: made above, destroyed once.
        unsafe { BCryptDestroyKey(handle) };
        if decrypted != 0 {
            return None;
        }
        plain.truncate(written as usize);
        Some(plain)
    }

    /// The text value of a registry entry written as `HKEY_...\Key\Path\ValueName`, read
    /// like Ubisoft Connect (a 32-bit program) reads it.
    pub(super) fn registry_value(entry: &str) -> Option<String> {
        use winreg::enums::{
            HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY,
        };
        let (hive, rest) = entry.split_once('\\')?;
        let (key, name) = rest.rsplit_once('\\')?;
        let hive = match hive.to_ascii_uppercase().as_str() {
            "HKEY_LOCAL_MACHINE" | "HKLM" => HKEY_LOCAL_MACHINE,
            "HKEY_CURRENT_USER" | "HKCU" => HKEY_CURRENT_USER,
            "HKEY_CLASSES_ROOT" | "HKCR" => HKEY_CLASSES_ROOT,
            _ => return None,
        };
        winreg::RegKey::predef(hive)
            .open_subkey_with_flags(key, KEY_READ | KEY_WOW64_32KEY)
            .and_then(|key| key.get_value::<String, _>(name))
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAR_CRY_3: &str = r#"version: 2.0
root:
  name: "Far Cry® 3"
  start_game:
    online:
      overlay_supported: no
      executables:
        - path:
            register: HKEY_CURRENT_USER\SOFTWARE\UBISOFT\Far Cry 3\ExecutableName
          working_directory:
            register: HKEY_LOCAL_MACHINE\SOFTWARE\UBISOFT\Far Cry 3\InstallDir
          arguments_register: HKEY_CURRENT_USER\SOFTWARE\UBISOFT\Far Cry 3\StartArguments
          description: FC3_GAME
          evil_updater:
            path:
              relative: bin\FC3Updater.exe
            arguments: -write_registry
        - path:
            relative: bin\FC3Editor.exe
    offline:
      executables:
        - path:
            relative: bin\Offline.exe
"#;

    const BLACK_FLAG: &str = r#"root:
  start_game:
    online:
      executables:
      - shortcut_name:
        description: Singleplayer
        path:
          relative: AC4BFSP.exe
        working_directory:
          register: HKEY_LOCAL_MACHINE\SOFTWARE\Ubisoft\Launcher\Installs\273\InstallDir
      - shortcut_name:
        path:
          relative: AC4BFMP.exe
"#;

    const FAR_CRY_4: &str = r#"root:
  start_game:
    online:
      executables:
        - path:
            relative: FarCry4.exe
          working_directory:
            register: HKEY_LOCAL_MACHINE\SOFTWARE\Ubisoft\Launcher\Installs\420\InstallDir
            append: bin
"#;

    const ORIGINS: &str = r#"root:
  start_game:
    online:
      executables:
      - shortcut_name: Assassin's Creed Origins
        path:
          relative: ACOrigins.exe
        working_directory:
          register: HKEY_LOCAL_MACHINE\SOFTWARE\Ubisoft\Launcher\Installs\3539\InstallDir
        arguments: /appid:{AppId}
"#;

    fn registry(entry: &str) -> Option<String> {
        match entry {
            r"HKEY_CURRENT_USER\SOFTWARE\UBISOFT\Far Cry 3\ExecutableName" => {
                Some("bin/FC3Updater.exe".to_string())
            }
            r"HKEY_LOCAL_MACHINE\SOFTWARE\UBISOFT\Far Cry 3\InstallDir" => {
                Some("D:/UplayLibrary/Far Cry 3".to_string())
            }
            r"HKEY_CURRENT_USER\SOFTWARE\UBISOFT\Far Cry 3\StartArguments" => Some(String::new()),
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Ubisoft\Launcher\Installs\273\InstallDir" => {
                Some("D:/UplayLibrary/Black Flag/".to_string())
            }
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Ubisoft\Launcher\Installs\420\InstallDir" => {
                Some("D:/UplayLibrary/Far Cry 4/".to_string())
            }
            _ => None,
        }
    }

    fn program(yaml: &str) -> Option<GameProgram> {
        first_executable(yaml)?.locate(registry)
    }

    #[test]
    fn finds_the_first_online_program() {
        assert_eq!(
            program(FAR_CRY_3),
            Some(GameProgram {
                exe: windows_path("D:/UplayLibrary/Far Cry 3/bin/FC3Updater.exe"),
                working_directory: windows_path("D:/UplayLibrary/Far Cry 3"),
            })
        );
        assert_eq!(
            program(BLACK_FLAG).map(|program| program.exe),
            Some(windows_path("D:/UplayLibrary/Black Flag/AC4BFSP.exe"))
        );
        assert_eq!(
            program(FAR_CRY_4),
            Some(GameProgram {
                exe: windows_path("D:/UplayLibrary/Far Cry 4/bin/FarCry4.exe"),
                working_directory: windows_path("D:/UplayLibrary/Far Cry 4/bin"),
            })
        );
    }

    #[test]
    fn skips_programs_that_need_arguments() {
        assert!(first_executable(ORIGINS).is_some());
        assert_eq!(program(ORIGINS), None);
        let with_arguments = |entry: &str| {
            if entry.ends_with("StartArguments") {
                Some("-dx9".to_string())
            } else {
                registry(entry)
            }
        };
        assert_eq!(
            first_executable(FAR_CRY_3).and_then(|exe| exe.locate(with_arguments)),
            None
        );
        assert_eq!(program("root:\n  name: no start_game\n"), None);
    }

    /// Lists what this computer's Ubisoft Connect catalogue gives:
    /// `cargo test -p boilr --lib this_computers_catalogue -- --ignored --nocapture`.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs Ubisoft Connect with installed games"]
    fn this_computers_catalogue() {
        let programs = game_programs();
        for (id, program) in &programs {
            println!("{id}: {program:?}");
        }
        assert!(!programs.is_empty());
    }

    fn message(fields: &[(u64, &[u8])]) -> Vec<u8> {
        let mut bytes = vec![];
        for (number, value) in fields {
            bytes.push(((number << 3) | 2) as u8);
            bytes.push(value.len() as u8);
            bytes.extend_from_slice(value);
        }
        bytes
    }

    #[test]
    fn reads_the_encrypted_file_and_its_records() {
        let encryption = message(&[(2, b"abc123"), (3, b"nonce"), (4, b"tag")]);
        let header = message(&[(2, &encryption)]);
        let outer = message(&[(1, &header), (2, b"data")]);
        let file = message(&[(1, &outer)]);
        assert_eq!(
            encrypted(&file),
            Some(Encrypted {
                key_id: "abc123".to_string(),
                nonce: b"nonce",
                tag: b"tag",
                data: b"data",
            })
        );
        assert_eq!(encrypted(b"\xff\xff"), None);

        // A record: product id 46 (a number field), an id, the YAML.
        let mut record = vec![1 << 3, 46];
        record.extend(message(&[(2, b"46"), (3, b"root:")]));
        let plain = message(&[(1, &record)]);
        assert_eq!(records(&plain), vec![(46, "root:".to_string())]);
    }
}
