//! Orchestration des tâches de fond (runtime tokio).
//!
//! GTK impose que l'interface ne soit manipulée QUE depuis le thread principal.
//! On fait donc tourner un runtime tokio multi-thread à part ; les tâches y
//! sont lancées avec `runtime().spawn(...)` et communiquent avec l'interface
//! exclusivement via des [`AppEvent`] envoyés sur un canal `async_channel`,
//! lu côté GTK par `glib::spawn_future_local`.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;
use tokio_util::sync::CancellationToken;

use crate::beta_key::{self, KeyUpdate};
use crate::config::{Config, APP_DIR};
use crate::events::{emit, log, status, AppEvent, Tx};
use crate::makemkv::cli::{self, KeyProblem};
use crate::makemkv::disc::{DiscInfo, Title};
use crate::makemkv::{install, settings};
use crate::metadata;
use crate::process::Tool;
use crate::remux;
use crate::selection::Selection;

pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("carbon-copy-worker")
            .build()
            .expect("création du runtime tokio")
    })
}

pub fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("carbon-copy/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("client HTTP")
    })
}

// --- Clé bêta --------------------------------------------------------------

fn key_check_stamp() -> PathBuf {
    gtk::glib::user_config_dir().join(APP_DIR).join("last-key-check")
}

fn today() -> String {
    gtk::glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%F").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Met à jour la clé bêta en ligne et notifie l'utilisateur.
async fn refresh_key(tx: &Tx, cfg: &Config) -> Result<KeyUpdate> {
    let r = beta_key::update_key(http(), &cfg.beta_key_sources).await;
    match &r {
        Ok(KeyUpdate::Updated(k)) => {
            log(tx, format!("Nouvelle clé bêta installée : {}", beta_key::mask(k)));
            emit(tx, AppEvent::Toast("Clé bêta MakeMKV mise à jour".into()));
        }
        Ok(KeyUpdate::UpToDate) => log(tx, "Clé bêta MakeMKV à jour."),
        Ok(KeyUpdate::PurchasedKey) => log(tx, "Clé MakeMKV enregistrée (achetée) : aucune mise à jour nécessaire."),
        Err(e) => log(tx, format!("⚠ Récupération de la clé bêta impossible : {e:#}")),
    }
    if r.is_ok() {
        let p = key_check_stamp();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, today());
    }
    r
}

/// Exécute `op` ; si MakeMKV signale une clé expirée, rafraîchit la clé et réessaie une fois.
async fn with_key_retry<T, F, Fut>(tx: &Tx, cfg: &Config, op: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    match op().await {
        Err(e) if e.is::<KeyProblem>() => {
            log(tx, format!("{e} → récupération d'une nouvelle clé bêta…"));
            match refresh_key(tx, cfg).await {
                Ok(KeyUpdate::Updated(_)) => op().await,
                _ => Err(e),
            }
        }
        other => other,
    }
}

// --- Démarrage -------------------------------------------------------------

/// Initialisation : MakeMKV, mkvmerge, clé bêta, périphériques.
pub async fn startup(tx: Tx, cfg: Config) {
    match settings::restore_selection_if_needed() {
        Ok(true) => log(&tx, "Règle de sélection MakeMKV restaurée (session précédente interrompue)."),
        Ok(false) => {}
        Err(e) => log(&tx, format!("⚠ Restauration de la règle de sélection impossible : {e}")),
    }

    let res: Result<(Tool, Tool)> = async {
        let makemkv = install::ensure_makemkv(&tx, http()).await?;
        let mkvmerge = install::ensure_mkvmerge(&tx).await?;

        // Vérification proactive de la clé : absente → immédiatement ;
        // sinon au plus une fois par jour (les clés bêta tournent chaque mois).
        let checked_today = std::fs::read_to_string(key_check_stamp()).map(|s| s.trim() == today()).unwrap_or(false);
        if settings::current_key().is_none() || !checked_today {
            status(&tx, "Vérification de la clé bêta MakeMKV…");
            let _ = refresh_key(&tx, &cfg).await;
        }

        // Vérification réactive : MakeMKV accepte-t-il de démarrer ?
        status(&tx, "Vérification de MakeMKV…");
        let mut probe = cli::probe(&makemkv, &tx).await?;
        if let Some(problem) = &probe.key_problem {
            log(&tx, format!("MakeMKV signale : {problem}"));
            if let Ok(KeyUpdate::Updated(_)) = refresh_key(&tx, &cfg).await {
                probe = cli::probe(&makemkv, &tx).await?;
            }
            if let Some(p) = &probe.key_problem {
                emit(&tx, AppEvent::Toast("MakeMKV : clé invalide ou version expirée".into()));
                log(&tx, format!("⚠ Problème persistant : {p}"));
            }
        }
        for d in &probe.drives {
            log(&tx, format!("Lecteur détecté : {} ({})", d.drive_name, d.device));
        }
        if probe.drives.is_empty() {
            log(&tx, "⚠ MakeMKV ne voit aucun lecteur optique.");
        }
        if !install::sg_devices_present() {
            log(&tx, "⚠ Aucun périphérique /dev/sg* : chargez le module « sg » (sudo modprobe sg) pour les Blu-ray.");
        }
        Ok((makemkv, mkvmerge))
    }
    .await;

    match res {
        Ok((makemkv, mkvmerge)) => emit(&tx, AppEvent::Ready { makemkv, mkvmerge }),
        Err(e) => emit(&tx, AppEvent::SetupFailed(format!("{e:#}"))),
    }
}

// --- Analyse ---------------------------------------------------------------

pub async fn analyze(tx: Tx, cfg: Config, makemkv: Tool, device: String, cancel: CancellationToken) {
    let res = with_key_retry(&tx, &cfg, || cli::analyze(&makemkv, &device, cfg.min_length_secs, &tx, cancel.clone())).await;
    let ok = res.as_ref().ok().cloned();
    emit(&tx, AppEvent::DiscAnalyzed(res.map_err(|e| format!("{e:#}"))));
    if let Some(info) = ok {
        lookup_metadata(tx, cfg, info).await;
    }
}

/// Déduit titre et année du nom du disque, puis interroge TMDB si possible.
pub async fn lookup_metadata(tx: Tx, cfg: Config, info: DiscInfo) {
    let raw = if info.volume_name().is_empty() { info.name() } else { info.volume_name().to_string() };
    let (mut title, mut year) = metadata::clean_disc_name(&raw);
    // Un nom de disque « propre » (CINFO 2) est souvent meilleur que le volume.
    let (named, named_year) = metadata::clean_disc_name(&info.name());
    if !named.is_empty() && named.len() >= title.len() {
        title = named;
        year = year.or(named_year);
    }
    let mut movie = None;
    if cfg.tmdb_api_key.trim().is_empty() {
        log(&tx, "TMDB non configuré : titre déduit du nom du disque.");
    } else {
        match metadata::tmdb_search(http(), cfg.tmdb_api_key.trim(), &title, year, &cfg.language).await {
            Ok(results) => match results.into_iter().next() {
                Some(m) => {
                    log(&tx, format!("TMDB : « {} » ({}) [tmdbid-{}]", m.title, m.year().map(|y| y.to_string()).unwrap_or("année inconnue".into()), m.id));
                    movie = Some(m);
                }
                None => log(&tx, format!("TMDB : aucun résultat pour « {title} »")),
            },
            Err(e) => log(&tx, format!("⚠ TMDB : {e:#}")),
        }
    }
    emit(&tx, AppEvent::Metadata { movie, title, year });
}

// --- Rip -------------------------------------------------------------------

/// Extraction MakeMKV d'un titre (toutes pistes) dans un dossier temporaire.
///
/// Elle ne dépend QUE du titre : le tri des pistes est fait ensuite par
/// mkvmerge. On peut donc la lancer dès la fin de l'analyse, pendant que
/// l'utilisateur vérifie la fiche et les pistes ; le rip proprement dit n'a
/// alors plus qu'à attendre la fin de la copie puis remuxer.
pub struct Extraction {
    /// Numéro de la tentative (permet d'ignorer les événements périmés).
    pub id: u64,
    pub makemkv: Tool,
    pub device: String,
    pub title: usize,
    pub tmp: PathBuf,
    pub cfg: Config,
}

pub async fn extract(tx: Tx, job: Extraction, cancel: CancellationToken) {
    let res = extract_inner(&tx, &job, cancel).await;
    if res.is_err() {
        // Annulée ou en échec : rien à garder.
        let _ = tokio::fs::remove_dir_all(&job.tmp).await;
    }
    emit(&tx, AppEvent::Extracted { id: job.id, result: res.map_err(|e| format!("{e:#}")) });
}

async fn extract_inner(tx: &Tx, job: &Extraction, cancel: CancellationToken) -> Result<PathBuf> {
    tokio::fs::create_dir_all(&job.tmp).await?;
    // Le garde restaure la règle de sélection MakeMKV dès la fin de l'extraction,
    // y compris en cas d'erreur ou d'annulation.
    let _guard = settings::override_selection()?;
    log(tx, format!("Extraction du titre {} vers {}", job.title, job.tmp.display()));
    emit(tx, AppEvent::ExtractProgress { id: job.id, label: "Démarrage de MakeMKV…".into(), fraction: 0.0 });
    let raw = with_key_retry(tx, &job.cfg, || {
        cli::rip(&job.makemkv, &job.device, job.title, job.cfg.min_length_secs, &job.tmp, tx, cancel.clone(), |label, f| {
            emit(tx, AppEvent::ExtractProgress { id: job.id, label, fraction: f })
        })
    })
    .await?;
    log(tx, format!("Extraction terminée : {}", raw.display()));
    Ok(raw)
}

/// Finalisation : remux des pistes choisies, suppression du dossier temporaire,
/// éjection éventuelle.
pub struct FinishJob {
    pub mkvmerge: Tool,
    pub device: String,
    pub title: Title,
    pub selection: Selection,
    pub raw: PathBuf,
    pub tmp: PathBuf,
    pub output: PathBuf,
    pub movie_title: String,
    pub cfg: Config,
}

/// Part de la barre de progression globale consacrée à l'extraction MakeMKV.
pub const RIP_WEIGHT: f64 = 0.9;

pub async fn finish(tx: Tx, job: FinishJob, cancel: CancellationToken) {
    let res = finish_inner(&tx, &job, cancel).await;
    let _ = tokio::fs::remove_dir_all(&job.tmp).await;
    if res.is_ok() && job.cfg.eject_when_done {
        eject(&tx, job.device.clone()).await;
    }
    emit(&tx, AppEvent::RipFinished(res.map_err(|e| format!("{e:#}"))));
}

async fn finish_inner(tx: &Tx, job: &FinishJob, cancel: CancellationToken) -> Result<PathBuf> {
    if let Some(dir) = job.output.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let label = "Remux des pistes sélectionnées…";
    emit(tx, AppEvent::RipProgress { label: label.into(), fraction: RIP_WEIGHT });
    let out = remux::remux(&job.mkvmerge, &job.title, &job.selection, &job.raw, &job.output, &job.movie_title, tx, cancel, |f| {
        emit(tx, AppEvent::RipProgress { label: label.into(), fraction: RIP_WEIGHT + f * (1.0 - RIP_WEIGHT) })
    })
    .await?;
    emit(tx, AppEvent::RipProgress { label: "Terminé".into(), fraction: 1.0 });
    Ok(out)
}

/// Espace libre (octets) du système de fichiers qui contient `path` (ou son
/// plus proche parent existant).
pub fn free_space(path: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let existing = path.ancestors().find(|p| p.exists())?;
    let c = std::ffi::CString::new(existing.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY : chemin C valide, structure de sortie initialisée.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

/// Vérifie qu'il reste de quoi écrire `need` octets (+5 % et 500 Mo de marge)
/// dans `dir` ; renvoie un message d'erreur lisible sinon.
pub fn check_space(dir: &std::path::Path, need: u64) -> Result<(), String> {
    let Some(free) = free_space(dir) else { return Ok(()) };
    let required = need + need / 20 + 500 * 1024 * 1024;
    if free >= required {
        return Ok(());
    }
    let gb = |b: u64| format!("{:.1} Go", b as f64 / 1e9);
    Err(format!("espace insuffisant dans {} : {} libres, {} nécessaires", dir.display(), gb(free), gb(required)))
}

/// Éjecte le disque et consigne le résultat dans le journal.
pub async fn eject(tx: &Tx, device: String) -> bool {
    let dev = device.clone();
    let res = tokio::task::spawn_blocking(move || crate::drive::eject(&dev))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match res {
        Ok(()) => {
            log(tx, format!("Disque éjecté ({device})."));
            true
        }
        Err(e) => {
            log(tx, format!("⚠ Éjection de {device} impossible : {e}"));
            emit(tx, AppEvent::Toast(format!("Éjection impossible : {e}")));
            false
        }
    }
}
