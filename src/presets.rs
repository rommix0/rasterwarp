//! The presets folder: one `<name>.rwpreset` file per look, listed by name.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::params::Params;
use crate::save::{self, Loaded, PRESET_EXTENSION};

/// Characters Windows doesn't allow in file names.
const FORBIDDEN: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// The name a preset would be saved under (trimmed), or why it can't be used.
pub fn check_name(name: &str) -> Result<&str, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        Err("Type a name for the preset.")
    } else if name.contains(FORBIDDEN) {
        Err("Names can't contain \\ / : * ? \" < > |")
    } else {
        Ok(name)
    }
}

pub fn path(folder: &Path, name: &str) -> PathBuf {
    folder.join(format!("{name}.{PRESET_EXTENSION}"))
}

/// The presets in `folder`, sorted by name ignoring case. A missing folder has none.
pub fn list(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let ext = path.extension()?.to_str()?;
            if !path.is_file() || !ext.eq_ignore_ascii_case(PRESET_EXTENSION) {
                return None;
            }
            Some(path.file_stem()?.to_str()?.to_owned())
        })
        .collect();
    names.sort_by_key(|name| (name.to_lowercase(), name.clone()));
    names
}

pub fn exists(folder: &Path, name: &str) -> bool {
    path(folder, name).is_file()
}

/// Saves `params` as preset `name`, replacing any preset of that name. Creates the
/// folder if needed.
pub fn save(folder: &Path, name: &str, params: &Params) -> Result<()> {
    let name = check_name(name).map_err(anyhow::Error::msg)?;
    save::save_preset(&path(folder, name), params)
}

pub fn load(folder: &Path, name: &str) -> Result<Loaded<Params>> {
    save::load_preset(&path(folder, name))
}

/// Renames preset `from` to `to`. Refuses a name another preset already has.
pub fn rename(folder: &Path, from: &str, to: &str) -> Result<()> {
    let to = check_name(to).map_err(anyhow::Error::msg)?;
    // Changing only the case renames the same file.
    if exists(folder, to) && !to.eq_ignore_ascii_case(from) {
        bail!("A preset named {to} already exists.");
    }
    fs::rename(path(folder, from), path(folder, to))
        .with_context(|| format!("could not rename {from} to {to}"))
}

pub fn delete(folder: &Path, name: &str) -> Result<()> {
    fs::remove_file(path(folder, name)).with_context(|| format!("could not delete {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::temp_dir;

    #[test]
    fn names_are_trimmed_and_checked() {
        assert_eq!(check_name("  Night drive "), Ok("Night drive"));
        assert!(check_name("   ").is_err());
        for bad in ["a/b", "a\\b", "c:", "why?", "\"q\"", "<x>", "a|b", "*"] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn presets_are_listed_by_name_ignoring_case() {
        let dir = temp_dir("list");
        assert!(list(&dir.join("missing")).is_empty());
        for name in ["beta", "Alpha", "gamma"] {
            save(&dir, name, &Params::default()).unwrap();
        }
        fs::write(dir.join("notes.txt"), "not a preset").unwrap();
        fs::create_dir(dir.join("folder.rwpreset")).unwrap();
        assert_eq!(list(&dir), ["Alpha", "beta", "gamma"]);
    }

    #[test]
    fn presets_save_load_rename_and_delete() {
        let dir = temp_dir("presets").join("new folder");
        let mut look = Params::default();
        look.warp.zoom = 3.0;
        save(&dir, " Wide ", &look).unwrap();
        assert!(exists(&dir, "Wide"));
        assert_eq!(load(&dir, "Wide").unwrap().value, look);

        save(&dir, "Other", &Params::default()).unwrap();
        let err = rename(&dir, "Wide", "Other").unwrap_err();
        assert_eq!(err.to_string(), "A preset named Other already exists.");
        assert!(rename(&dir, "Wide", "a/b").is_err());
        rename(&dir, "Wide", "Zoomed").unwrap();
        rename(&dir, "Zoomed", "ZOOMED").unwrap();
        assert_eq!(list(&dir), ["Other", "ZOOMED"]);
        assert_eq!(load(&dir, "ZOOMED").unwrap().value, look);

        delete(&dir, "Other").unwrap();
        assert_eq!(list(&dir), ["ZOOMED"]);
        assert!(delete(&dir, "Other").is_err());
    }
}
