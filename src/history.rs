//! Historique des rips : vidéothèque de l'écran d'accueil et alerte « déjà
//! copié ».
//!
//! Chaque rip réussi est ajouté à `~/.local/share/intact-bitstream/history.json`.
//! Les films déjà présents dans le dossier de destination (rips antérieurs à
//! l'historique) sont repris d'après leur nom :
//! `Titre (Année) [tmdbid-N] - caractéristiques.mkv`.

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::config::APP_DIR;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub title: String,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub tmdb_id: Option<u64>,
    /// Chemin TMDB de l'affiche (`/abc.jpg`).
    #[serde(default)]
    pub poster_path: Option<String>,
    pub path: PathBuf,
    #[serde(default)]
    pub size: u64,
    /// Date du rip, `AAAA-MM-JJ`.
    pub date: String,
}

impl Entry {
    /// « 14/11/2026 ».
    pub fn date_label(&self) -> String {
        let p: Vec<&str> = self.date.split('-').collect();
        match p.as_slice() {
            [y, m, d] => format!("{d}/{m}/{y}"),
            _ => self.date.clone(),
        }
    }

    /// « 4,1 Go ».
    pub fn size_label(&self) -> String {
        format!("{:.1} Go", self.size as f64 / 1e9).replace('.', ",")
    }
}

fn data_file() -> PathBuf {
    gtk::glib::user_data_dir().join(APP_DIR).join("history.json")
}

/// Dossier de cache des affiches (`<tmdbid>.jpg`).
pub fn poster_cache(tmdb_id: u64) -> PathBuf {
    gtk::glib::user_cache_dir().join(APP_DIR).join("posters").join(format!("{tmdb_id}.jpg"))
}

pub fn today() -> String {
    gtk::glib::DateTime::now_local().ok().and_then(|d| d.format("%F").ok()).map(|s| s.to_string()).unwrap_or_default()
}

pub fn load() -> Vec<Entry> {
    std::fs::read(data_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Ajoute (ou remplace, même fichier) une entrée en tête de l'historique.
pub fn add(entry: Entry) -> anyhow::Result<()> {
    let mut all = load();
    all.retain(|e| e.path != entry.path);
    all.insert(0, entry);
    all.truncate(1000);
    let p = data_file();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, serde_json::to_vec_pretty(&all)?)?;
    Ok(())
}

/// Rip précédent du même film (identifiant TMDB), si le fichier existe encore.
pub fn find_by_tmdb(id: u64) -> Option<Entry> {
    load().into_iter().find(|e| e.tmdb_id == Some(id) && e.path.exists())
}

/// « Titre (Année) [tmdbid-N] - … .mkv » → (titre, année, identifiant).
pub fn parse_name(name: &str) -> Option<(String, Option<u32>, Option<u64>)> {
    if !name.to_lowercase().ends_with(".mkv") {
        return None;
    }
    let head = Regex::new(r"^(?P<t>.+?) \((?P<y>\d{4})\)").ok()?;
    let c = head.captures(name)?;
    let id = Regex::new(r"\[tmdbid-(\d+)\]").ok()?.captures(name).and_then(|m| m[1].parse().ok());
    Some((c["t"].trim().to_string(), c["y"].parse().ok(), id))
}

fn mtime_date(meta: &std::fs::Metadata) -> String {
    let secs = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
    secs.and_then(|s| gtk::glib::DateTime::from_unix_local(s).ok())
        .and_then(|d| d.format("%F").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Vidéothèque : historique + films reconnus dans `dir`, du plus récent au
/// plus ancien. Lecture de dossier : à appeler hors du thread GTK.
pub fn library(dir: &Path) -> Vec<Entry> {
    let mut all: Vec<Entry> = load().into_iter().filter(|e| e.path.exists()).collect();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let path = e.path();
            if all.iter().any(|x| x.path == path) {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let Some((title, year, tmdb_id)) = parse_name(&name) else { continue };
            let Ok(meta) = e.metadata() else { continue };
            all.push(Entry { title, year, tmdb_id, poster_path: None, path, size: meta.len(), date: mtime_date(&meta) });
        }
    }
    all.sort_by(|a, b| b.date.cmp(&a.date));
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_names() {
        assert_eq!(
            parse_name("Ice Storm (1997) [tmdbid-68924] - 576i MPEG2 MP2 2.0 FRENCH.mkv"),
            Some(("Ice Storm".into(), Some(1997), Some(68924)))
        );
        assert_eq!(
            parse_name("La Ligne rouge (1998) [The Thin Red Line] [tmdbid-8741] - 576p MPEG2 AC3 5.1 VF.mkv"),
            Some(("La Ligne rouge".into(), Some(1998), Some(8741)))
        );
        assert_eq!(parse_name("IT (1990).mkv"), Some(("IT".into(), Some(1990), None)));
        assert_eq!(parse_name("notes.txt"), None);
        assert_eq!(parse_name("B1_t00.mkv"), None);
    }

    #[test]
    fn labels() {
        let e = Entry { title: "X".into(), year: None, tmdb_id: None, poster_path: None, path: "/x".into(), size: 4_052_305_028, date: "2026-11-14".into() };
        assert_eq!(e.date_label(), "14/11/2026");
        assert_eq!(e.size_label(), "4,1 Go");
    }
}
