//! Préférences de l'application, stockées en TOML dans
//! `~/.config/carbon-copy/config.toml`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const APP_DIR: &str = "carbon-copy";
/// Ancien nom du dossier de configuration (application « MKV Ripper »).
const LEGACY_APP_DIR: &str = "mkv-ripper";

/// Reprend `~/.config/mkv-ripper` (réglages, clé, règle de sélection à restaurer)
/// s'il existe et que le nouveau dossier n'existe pas encore.
pub fn migrate_legacy_dir() {
    let base = gtk::glib::user_config_dir();
    let (old, new) = (base.join(LEGACY_APP_DIR), base.join(APP_DIR));
    if old.is_dir() && !new.exists() {
        let _ = std::fs::rename(old, new);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Langue préférée (ISO 639-2, ex. `fra`) pour l'audio et les sous-titres.
    pub language: String,
    /// Conserver aussi la piste audio originale (VO) par défaut.
    pub keep_original_audio: bool,
    /// Clé API TMDB (v3) ou jeton de lecture (v4, « eyJ… »). Vide = désactivé.
    pub tmdb_api_key: String,
    /// Dossier de destination par défaut.
    pub output_dir: PathBuf,
    /// Durée minimale (s) des titres listés par MakeMKV.
    pub min_length_secs: u32,
    /// Éjecter le disque à la fin du rip.
    pub eject_when_done: bool,
    /// Sources de la clé bêta, essayées dans l'ordre.
    pub beta_key_sources: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        let videos = gtk::glib::user_special_dir(gtk::glib::UserDirectory::Videos)
            .unwrap_or_else(|| gtk::glib::home_dir().join("Vidéos"));
        Self {
            language: "fra".into(),
            keep_original_audio: true,
            tmdb_api_key: String::new(),
            output_dir: videos,
            min_length_secs: 120,
            eject_when_done: false,
            beta_key_sources: vec![
                // Source officielle : le fil du forum où l'auteur de MakeMKV publie la clé.
                "https://forum.makemkv.com/forum/viewtopic.php?t=1053".into(),
            ],
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        gtk::glib::user_config_dir().join(APP_DIR).join("config.toml")
    }

    /// Charge la configuration ; en cas d'absence ou d'erreur, valeurs par défaut.
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&p, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}
