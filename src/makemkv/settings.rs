//! Lecture / écriture du fichier de réglages MakeMKV (`~/.MakeMKV/settings.conf`).
//!
//! Format : une entrée par ligne, `nom = "valeur"`. On ne modifie que les
//! lignes qui nous concernent et on conserve tout le reste à l'identique.
//! Une sauvegarde `settings.conf.bak` est faite avant chaque écriture.

use std::path::PathBuf;

use anyhow::Result;
use regex::Regex;

pub const KEY_ENTRY: &str = "app_Key";
pub const SELECTION_ENTRY: &str = "app_DefaultSelectionString";
/// Règle de sélection « tout extraire » : le tri fin est fait ensuite par mkvmerge.
pub const SELECT_ALL: &str = "+sel:all";

pub fn settings_path() -> PathBuf {
    gtk::glib::home_dir().join(".MakeMKV").join("settings.conf")
}

fn read_file() -> String {
    std::fs::read_to_string(settings_path()).unwrap_or_default()
}

fn entry_regex(name: &str) -> Regex {
    Regex::new(&format!(r#"(?m)^[ \t]*{}[ \t]*=[ \t]*"([^"]*)"[ \t]*\r?$"#, regex::escape(name))).expect("regex valide")
}

/// Valeur d'une entrée (`None` si absente).
pub fn get(name: &str) -> Option<String> {
    entry_regex(name).captures(&read_file()).map(|c| c[1].to_string())
}

/// Écrit (ou supprime si `value` est `None`) une entrée, puis sauvegarde.
pub fn set(name: &str, value: Option<&str>) -> Result<()> {
    let path = settings_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let original = read_file();
    if !original.is_empty() {
        std::fs::write(path.with_extension("conf.bak"), &original)?;
    }
    let content = set_in(&original, name, value);
    // Écriture atomique : fichier temporaire puis renommage.
    let tmp = path.with_extension("conf.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// Transformation pure (testable) du contenu du fichier.
fn set_in(content: &str, name: &str, value: Option<&str>) -> String {
    let re = entry_regex(name);
    let line = value.map(|v| format!(r#"{name} = "{v}""#));
    let mut out = match (&line, re.is_match(content)) {
        (Some(l), true) => re.replace(content, regex::NoExpand(l)).into_owned(),
        (None, true) => re.replace(content, "").into_owned(),
        (Some(l), false) => {
            let mut s = content.trim_end().to_string();
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(l);
            s
        }
        (None, false) => content.to_string(),
    };
    // Nettoyage des lignes vides en double laissées par une suppression.
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Clé actuellement enregistrée.
pub fn current_key() -> Option<String> {
    get(KEY_ENTRY).filter(|k| !k.trim().is_empty())
}

/// Une clé bêta publique commence par `T-` ; une clé achetée a un autre format
/// et ne doit JAMAIS être écrasée.
pub fn is_beta_key(key: &str) -> bool {
    key.starts_with("T-")
}

// --- Surcharge temporaire de la règle de sélection ---------------------------
//
// `makemkvcon` n'accepte pas de sélection de pistes en ligne de commande : il
// applique `app_DefaultSelectionString`. Pendant un rip on la remplace donc par
// `+sel:all`, puis on restaure la valeur d'origine. La valeur d'origine est
// aussi mémorisée sur disque pour être restaurée au prochain lancement si
// l'application est tuée en plein rip.

fn restore_file() -> PathBuf {
    gtk::glib::user_config_dir().join(crate::config::APP_DIR).join("selection.restore")
}

const ABSENT: &str = "\u{0}absent";

/// Remplace la règle de sélection par `+sel:all` ; renvoie un garde qui la
/// restaure automatiquement lorsqu'il est détruit.
pub fn override_selection() -> Result<SelectionGuard> {
    let original = get(SELECTION_ENTRY);
    let rf = restore_file();
    if let Some(d) = rf.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&rf, original.as_deref().unwrap_or(ABSENT))?;
    set(SELECTION_ENTRY, Some(SELECT_ALL))?;
    Ok(SelectionGuard)
}

/// Restaure la règle mémorisée, s'il y en a une (appelé au démarrage).
pub fn restore_selection_if_needed() -> Result<bool> {
    let rf = restore_file();
    let Ok(saved) = std::fs::read_to_string(&rf) else {
        return Ok(false);
    };
    set(SELECTION_ENTRY, if saved == ABSENT { None } else { Some(saved.as_str()) })?;
    std::fs::remove_file(rf)?;
    Ok(true)
}

pub struct SelectionGuard;

impl Drop for SelectionGuard {
    fn drop(&mut self) {
        let _ = restore_selection_if_needed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_append_remove() {
        let c = "app_DataDir = \"/x\"\napp_Key = \"T-old\"\n";
        assert_eq!(set_in(c, KEY_ENTRY, Some("T-new")), "app_DataDir = \"/x\"\napp_Key = \"T-new\"\n");
        assert_eq!(set_in("app_DataDir = \"/x\"\n", KEY_ENTRY, Some("T-a")), "app_DataDir = \"/x\"\napp_Key = \"T-a\"\n");
        assert_eq!(set_in("", KEY_ENTRY, Some("T-a")), "app_Key = \"T-a\"\n");
        assert_eq!(set_in(c, KEY_ENTRY, None), "app_DataDir = \"/x\"\n\n");
    }

    #[test]
    fn dollar_signs_are_literal() {
        // Une valeur contenant « $1 » ne doit pas être interprétée par regex.
        let out = set_in("app_Key = \"x\"\n", KEY_ENTRY, Some("T-a$1b"));
        assert_eq!(out, "app_Key = \"T-a$1b\"\n");
    }
}
