//! Orchestration des tâches de fond (runtime tokio).
//!
//! GTK impose que l'interface ne soit manipulée QUE depuis le thread principal.
//! On fait donc tourner un runtime tokio multi-thread à part ; les tâches y
//! sont lancées avec `runtime().spawn(...)` et communiquent avec l'interface
//! exclusivement via des [`AppEvent`] envoyés sur un canal `async_channel`,
//! lu côté GTK par `glib::spawn_future_local`.

use std::path::{Path, PathBuf};
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
    let mut source = "nom du disque".to_string();

    if !cfg.tmdb_api_key.trim().is_empty() {
        match metadata::tmdb_search(http(), cfg.tmdb_api_key.trim(), &title, year, &cfg.language).await {
            Ok(Some((t, y))) => {
                log(&tx, format!("TMDB : « {t} » ({})", y.map(|y| y.to_string()).unwrap_or("année inconnue".into())));
                title = t;
                year = y.or(year);
                source = "TMDB".into();
            }
            Ok(None) => log(&tx, format!("TMDB : aucun résultat pour « {title} »")),
            Err(e) => log(&tx, format!("⚠ TMDB : {e:#}")),
        }
    }
    emit(&tx, AppEvent::Metadata { title, year, source });
}

// --- Rip -------------------------------------------------------------------

pub struct RipJob {
    pub makemkv: Tool,
    pub mkvmerge: Tool,
    pub device: String,
    pub title: Title,
    pub selection: Selection,
    pub output: PathBuf,
    pub movie_title: String,
    pub cfg: Config,
}

/// Part de la barre de progression globale consacrée à l'extraction MakeMKV.
const RIP_WEIGHT: f64 = 0.9;

pub async fn rip(tx: Tx, job: RipJob, cancel: CancellationToken) {
    let res = rip_inner(&tx, &job, cancel).await;
    if res.is_ok() && job.cfg.eject_when_done {
        let dev = job.device.clone();
        let _ = tokio::task::spawn_blocking(move || crate::drive::eject(&dev)).await;
    }
    emit(&tx, AppEvent::RipFinished(res.map_err(|e| format!("{e:#}"))));
}

async fn rip_inner(tx: &Tx, job: &RipJob, cancel: CancellationToken) -> Result<PathBuf> {
    let out_dir = job.output.parent().unwrap_or(Path::new(".")).to_path_buf();
    tokio::fs::create_dir_all(&out_dir).await?;
    // Dossier temporaire sur le MÊME système de fichiers que la destination :
    // pas de copie inter-disques de dizaines de Go.
    let tmp = out_dir.join(format!(".carbon-copy-{}", std::process::id()));
    let _cleanup = TmpDir(tmp.clone());

    // Le garde restaure la règle de sélection MakeMKV dès la fin de l'extraction,
    // y compris en cas d'erreur ou d'annulation.
    let raw = {
        let _guard = settings::override_selection()?;
        log(tx, format!("Extraction du titre {} vers {}", job.title.index, tmp.display()));
        emit(tx, AppEvent::RipProgress { label: "Démarrage de MakeMKV…".into(), fraction: 0.0 });
        with_key_retry(tx, &job.cfg, || {
            cli::rip(
                &job.makemkv,
                &job.device,
                job.title.index,
                job.cfg.min_length_secs,
                &tmp,
                tx,
                cancel.clone(),
                |label, f| emit(tx, AppEvent::RipProgress { label, fraction: f * RIP_WEIGHT }),
            )
        })
        .await?
    };
    log(tx, format!("Extraction terminée : {}", raw.display()));

    emit(tx, AppEvent::RipProgress { label: "Remux des pistes sélectionnées…".into(), fraction: RIP_WEIGHT });
    let out = remux::remux(
        &job.mkvmerge,
        &job.title,
        &job.selection,
        &raw,
        &job.output,
        &job.movie_title,
        tx,
        cancel,
        |f| {
            emit(
                tx,
                AppEvent::RipProgress {
                    label: "Remux des pistes sélectionnées…".into(),
                    fraction: RIP_WEIGHT + f * (1.0 - RIP_WEIGHT),
                },
            )
        },
    )
    .await?;
    emit(tx, AppEvent::RipProgress { label: "Terminé".into(), fraction: 1.0 });
    Ok(out)
}

/// Supprime le dossier temporaire quand il sort de portée.
struct TmpDir(PathBuf);
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
