//! Contournement du blocage « SDF » de MakeMKV 2.x.
//!
//! Avec certains lecteurs (ex. HL-DT-ST DVDRAM GSA-T50N), MakeMKV 2.0 se fige
//! indéfiniment pendant « Balayage des lecteurs CD-ROM » dès qu'un disque est
//! inséré : il exécute la logique spécifique au lecteur contenue dans `sdf.bin`.
//! Le réglage `sdf_Stop = "<identifiant du lecteur>"` désactive cette étape.
//!
//! L'identifiant (ex. `HL-DT-ST_DVDRAM_GSA-T50N_RE06_20090730123456_M009AQA1959`)
//! n'apparaît que dans le journal de débogage `~/MakeMKV_log.txt` (option
//! `--debug`), sur une ligne `SDF auto v0a6: <id>` ou `No  SDF v0a6: <id>`.
//! L'application lance donc toujours makemkvcon avec `--debug` ; si aucune ligne
//! `DRV:` n'arrive dans le délai imparti, le processus est tué, l'identifiant est
//! lu dans le journal, `sdf_Stop` est écrit et la commande est relancée.

use std::path::PathBuf;
use std::time::Duration;

use regex::Regex;

use super::settings;

pub const SDF_STOP_ENTRY: &str = "sdf_Stop";

/// Délai maximal pour le balayage des lecteurs (normalement 1 à 5 s).
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(60);

/// Journal de débogage de MakeMKV (chemin fixe : `--debug=fichier` est ignoré).
pub fn debug_log_path() -> PathBuf {
    gtk::glib::home_dir().join("MakeMKV_log.txt")
}

/// Crée le journal s'il n'existe pas : `flatpak run --filesystem=<fichier>`
/// n'accorde l'accès qu'à un fichier existant.
pub fn ensure_debug_log() -> Option<PathBuf> {
    let p = debug_log_path();
    if !p.exists() {
        std::fs::OpenOptions::new().create(true).append(true).open(&p).ok()?;
    }
    Some(p)
}

/// Identifiants SDF des lecteurs mentionnés dans le journal, dans l'ordre,
/// sans ceux déjà désactivés (`SDF disabled: …`).
pub fn drive_ids(log: &str) -> Vec<String> {
    let re = Regex::new(r"(?m)^(?:No\s+)?SDF\s+\S*\s*v[0-9a-fA-F]+:\s*(\S+)\s*$").expect("regex valide");
    let mut ids: Vec<String> = Vec::new();
    for c in re.captures_iter(log) {
        let id = c[1].to_string();
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// Résultat de la tentative de correction.
#[derive(Debug, PartialEq, Eq)]
pub enum Fix {
    /// `sdf_Stop` vient d'être écrit avec cet identifiant.
    Applied(String),
    /// `sdf_Stop` contenait déjà cet identifiant : le blocage a une autre cause.
    AlreadySet(String),
    /// Aucun identifiant trouvé dans le journal.
    NoId,
}

/// Lit le journal de débogage et écrit `sdf_Stop` si nécessaire.
pub fn apply_from_log() -> anyhow::Result<Fix> {
    let log = std::fs::read(debug_log_path()).unwrap_or_default();
    let log = String::from_utf8_lossy(&log);
    // Le dernier lecteur traité est celui sur lequel MakeMKV s'est figé.
    let Some(id) = drive_ids(&log).pop() else {
        return Ok(Fix::NoId);
    };
    if settings::get(SDF_STOP_ENTRY).as_deref() == Some(id.as_str()) {
        return Ok(Fix::AlreadySet(id));
    }
    settings::set(SDF_STOP_ENTRY, Some(&id))?;
    Ok(Fix::Applied(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "Debug log started at Fri Nov 13 21:28:35 2026\n\
        001003:0020 DEBUG: Code 0 at ZT1Pm.L!,.E|I0:ZpGU.+S:29393631\n\
        No  SDF v0a6: HL-DT-ST_DVDRAM_GSA-T50N_RE06_20090730123456_M009AQA1959\n\
        001003:0020 DEBUG: Code 0 at ZT1Pm.L!,.E|I0:ZpGU.+S:29393631\n\
        SDF auto v0a6: HL-DT-ST_DVDRAM_GSA-T50N_RE06_20090730123456_M009AQA1959\n";

    #[test]
    fn extracts_drive_id() {
        assert_eq!(drive_ids(LOG), vec!["HL-DT-ST_DVDRAM_GSA-T50N_RE06_20090730123456_M009AQA1959".to_string()]);
    }

    #[test]
    fn ignores_disabled_and_noise() {
        assert!(drive_ids("SDF disabled: HL-DT-ST_X\nSDF.bin version: 0x00A6\n").is_empty());
    }
}
