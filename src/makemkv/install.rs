//! Vérification, installation et mise à jour de MakeMKV et de mkvmerge.
//!
//! Stratégie (sans droits administrateur) :
//! 1. Si `makemkvcon` natif est présent ET à jour → on l'utilise.
//! 2. Sinon, on passe par le Flatpak officiel de Flathub (`com.makemkv.MakeMKV`),
//!    installé ou mis à jour en mode utilisateur (`--user`).
//! 3. Même logique pour `mkvmerge` : binaire natif (`dnf install mkvtoolnix`)
//!    ou Flatpak `org.bunkus.mkvtoolnix-gui`.
//!
//! Le Flatpak MakeMKV partage `~/.MakeMKV` avec l'hôte : la clé bêta et les
//! réglages sont donc au même endroit quel que soit le mode d'installation.

use anyhow::{bail, Context, Result};
use regex::Regex;
use tokio::process::Command;

use crate::events::{log, status, Tx};
use crate::process::{run_capture, run_streaming, which, Tool};

pub const MAKEMKV_FLATPAK: &str = "com.makemkv.MakeMKV";
pub const MKVTOOLNIX_FLATPAK: &str = "org.bunkus.mkvtoolnix-gui";
const FLATHUB_REPO: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";
const MAKEMKV_DOWNLOAD_PAGE: &str = "https://www.makemkv.com/download/";

/// Compare deux versions « 1.17.8 » numériquement.
pub fn version_tuple(v: &str) -> Vec<u32> {
    v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect()
}

fn is_older(installed: &str, latest: &str) -> bool {
    version_tuple(installed) < version_tuple(latest)
}

/// Dernière version publiée, lue sur la page de téléchargement officielle.
pub async fn latest_makemkv_version(http: &reqwest::Client) -> Option<String> {
    let body = http.get(MAKEMKV_DOWNLOAD_PAGE).send().await.ok()?.text().await.ok()?;
    let re = Regex::new(r"MakeMKV\s+v?(\d+\.\d+\.\d+)").ok()?;
    re.captures_iter(&body)
        .map(|c| c[1].to_string())
        .max_by(|a, b| version_tuple(a).cmp(&version_tuple(b)))
}

/// Version du binaire natif : `makemkvcon` affiche « MakeMKV v1.17.8 linux(…) started »
/// (message 1005) dès son démarrage ; on arrête le processus dès qu'on l'a lu.
async fn native_version(path: &std::path::Path) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.args(["-r", "--cache=1", "info", "disc:9999"]);
    let (_, out) = tokio::time::timeout(std::time::Duration::from_secs(60), run_capture(cmd)).await.ok()?.ok()?;
    let re = Regex::new(r"MakeMKV v(\d+\.\d+\.\d+)").ok()?;
    re.captures(&out).map(|c| c[1].to_string())
}

async fn flatpak_available() -> bool {
    which("flatpak").is_some()
}

/// Version d'une application Flatpak installée (`None` si absente).
async fn flatpak_version(app_id: &str) -> Option<String> {
    let mut c = Command::new("flatpak");
    c.args(["info", app_id]);
    let (st, out) = run_capture(c).await.ok()?;
    if !st.success() {
        return None;
    }
    // La sortie est localisée (« Version: » ou « Version : ») : on prend le
    // premier motif x.y.z trouvé sur une ligne contenant « Version ».
    let re = Regex::new(r"(?m)^\s*Version\s*:\s*([0-9][0-9.]*)").ok()?;
    re.captures(&out).map(|c| c[1].to_string()).or_else(|| Some(String::from("?")))
}

/// Installe (ou met à jour) une application Flatpak en mode utilisateur.
async fn flatpak_install_or_update(tx: &Tx, app_id: &str) -> Result<()> {
    let logger = |_, l: String| log(tx, format!("  flatpak │ {l}"));
    if flatpak_version(app_id).await.is_some() {
        status(tx, format!("Mise à jour de {app_id}…"));
        let mut c = Command::new("flatpak");
        c.args(["update", "-y", "--noninteractive", app_id]);
        // Une mise à jour ratée (hors-ligne…) n'est pas bloquante.
        if let Err(e) = run_streaming(c, None, logger).await {
            log(tx, format!("Mise à jour impossible : {e}"));
        }
        return Ok(());
    }

    status(tx, format!("Installation de {app_id} depuis Flathub…"));
    let mut remote = Command::new("flatpak");
    remote.args(["remote-add", "--user", "--if-not-exists", "flathub", FLATHUB_REPO]);
    run_streaming(remote, None, logger).await?;

    let mut c = Command::new("flatpak");
    c.args(["install", "--user", "-y", "--noninteractive", "flathub", app_id]);
    let st = run_streaming(c, None, logger).await?;
    if !st.success() {
        bail!("l'installation Flatpak de {app_id} a échoué (code {st})");
    }
    Ok(())
}

/// Garantit la présence d'un `makemkvcon` utilisable et à jour.
pub async fn ensure_makemkv(tx: &Tx, http: &reqwest::Client) -> Result<Tool> {
    status(tx, "Recherche de MakeMKV…");
    let latest = latest_makemkv_version(http).await;
    match &latest {
        Some(v) => log(tx, format!("Dernière version publiée de MakeMKV : {v}")),
        None => log(tx, "Impossible de connaître la dernière version de MakeMKV (hors-ligne ?)"),
    }

    let flatpak_tool = Tool::Flatpak { app_id: MAKEMKV_FLATPAK.into(), command: "makemkvcon".into() };
    let has_flatpak = flatpak_available().await;

    // 1. Binaire natif.
    if let Some(path) = which("makemkvcon") {
        let v = native_version(&path).await;
        log(tx, format!("makemkvcon natif : {} (version {})", path.display(), v.as_deref().unwrap_or("?")));
        let outdated = matches!((&v, &latest), (Some(i), Some(l)) if is_older(i, l));
        if !outdated {
            return Ok(Tool::Native(path));
        }
        if !has_flatpak {
            log(tx, "⚠ MakeMKV natif obsolète et Flatpak indisponible : mettez-le à jour manuellement.");
            return Ok(Tool::Native(path));
        }
        log(tx, "MakeMKV natif obsolète : bascule sur la version Flatpak (Flathub).");
    }

    // 2. Flatpak (installation ou mise à jour).
    if !has_flatpak {
        bail!(
            "MakeMKV n'est pas installé et la commande « flatpak » est introuvable.\n\
             Relancez install.sh (il installe Flatpak), ou installez MakeMKV manuellement."
        );
    }
    flatpak_install_or_update(tx, MAKEMKV_FLATPAK).await.context("installation de MakeMKV")?;
    let v = flatpak_version(MAKEMKV_FLATPAK).await.unwrap_or_else(|| "?".into());
    log(tx, format!("MakeMKV Flatpak prêt (version {v})"));
    Ok(flatpak_tool)
}

/// Garantit la présence de `mkvmerge` (remux final des pistes sélectionnées).
pub async fn ensure_mkvmerge(tx: &Tx) -> Result<Tool> {
    if let Some(p) = which("mkvmerge") {
        log(tx, format!("mkvmerge natif : {}", p.display()));
        return Ok(Tool::Native(p));
    }
    if !flatpak_available().await {
        bail!("mkvmerge est introuvable. Relancez install.sh ou installez le paquet « mkvtoolnix » de votre distribution.");
    }
    flatpak_install_or_update(tx, MKVTOOLNIX_FLATPAK).await.context("installation de MKVToolNix")?;
    Ok(Tool::Flatpak { app_id: MKVTOOLNIX_FLATPAK.into(), command: "mkvmerge".into() })
}

/// Vérifie la présence des périphériques SCSI génériques (`/dev/sg*`), requis
/// par MakeMKV pour piloter les lecteurs Blu-ray.
pub fn sg_devices_present() -> bool {
    std::fs::read_dir("/dev")
        .map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("sg")))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions() {
        assert!(is_older("1.17.7", "1.17.8"));
        assert!(is_older("1.9.9", "1.17.0"));
        assert!(!is_older("1.18.0", "1.17.9"));
    }
}
