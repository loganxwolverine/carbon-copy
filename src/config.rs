//! Préférences de l'application, stockées en TOML dans
//! `~/.config/intact-bitstream/config.toml`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const APP_DIR: &str = "intact-bitstream";
/// Anciens noms de l'application (« Carbon Copy », puis « MKV Ripper »),
/// du plus récent au plus ancien.
const LEGACY_APP_DIRS: [&str; 2] = ["carbon-copy", "mkv-ripper"];

/// Reprend les dossiers d'un ancien nom de l'application (réglages, clé,
/// règle de sélection à restaurer, historique, affiches en cache) s'ils
/// existent et que le nouveau dossier n'existe pas encore.
pub fn migrate_legacy_dir() {
    use gtk::glib;
    for base in [glib::user_config_dir(), glib::user_data_dir(), glib::user_cache_dir()] {
        let new = base.join(APP_DIR);
        if new.exists() {
            continue;
        }
        if let Some(old) = LEGACY_APP_DIRS.iter().map(|d| base.join(d)).find(|d| d.is_dir()) {
            let _ = std::fs::rename(old, new);
        }
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
    /// Lancer l'extraction du titre dès la fin de l'analyse (gain de temps).
    pub prepare_in_background: bool,
    /// Démarrer en plein écran (mémorisé à chaque bascule F11).
    pub fullscreen: bool,
    /// L'assistant de clé TMDB a déjà été proposé au premier lancement.
    pub tmdb_prompted: bool,
    /// Sources de la clé bêta, essayées dans l'ordre.
    pub beta_key_sources: Vec<String>,
    /// Télécommande web active (serveur HTTP sur le réseau local).
    pub remote_enabled: bool,
    /// Port de la télécommande web.
    pub remote_port: u16,
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
            fullscreen: true,
            tmdb_prompted: false,
            prepare_in_background: true,
            remote_enabled: false,
            remote_port: crate::remote::DEFAULT_PORT,
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
