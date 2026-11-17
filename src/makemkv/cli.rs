//! Pilotage de `makemkvcon` : sonde, analyse du disque et extraction (rip).
//!
//! Toutes les commandes utilisent le mode robot (`-r`) et la progression est
//! demandée sur la même sortie (`--progress=-same`). Chaque ligne lue est
//! décodée par [`robot::parse_line`] AU FIL DE L'EAU : on n'attend pas la fin du
//! processus pour mettre à jour l'interface.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use tokio_util::sync::CancellationToken;

use super::disc::DiscInfo;
use super::robot::{parse_line, DriveEntry, RobotLine};
use super::sdf;
use crate::beta_key::is_key_problem;
use crate::events::{emit, log, AppEvent, Tx};
use crate::process::{run_streaming, Tool};

/// Erreur typée : MakeMKV refuse de travailler (clé expirée / version trop ancienne).
#[derive(Debug)]
pub struct KeyProblem(pub String);

impl std::fmt::Display for KeyProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MakeMKV refuse de fonctionner : {}", self.0)
    }
}
impl std::error::Error for KeyProblem {}

#[derive(Debug, Default)]
pub struct ProbeResult {
    pub version: Option<String>,
    pub drives: Vec<DriveEntry>,
    pub key_problem: Option<String>,
}

/// Message MakeMKV « Échec de l'ouverture du disque ».
const MSG_OPEN_FAILED: u32 = 5010;
/// Message MakeMKV « Échec à l'écriture du titre … » : le fichier est inutilisable.
const MSG_TITLE_FAILED: u32 = 5003;
/// Message MakeMKV « Erreur … survenue à la lecture de … » (secteur illisible).
const MSG_READ_ERROR: u32 = 2003;
/// Tentatives d'ouverture du disque avant d'abandonner l'extraction.
const OPEN_ATTEMPTS: u32 = 3;

/// Erreur typée : MakeMKV reste figé pendant le balayage des lecteurs.
#[derive(Debug)]
pub struct DriveScanStalled(pub String);

impl std::fmt::Display for DriveScanStalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MakeMKV reste bloqué au balayage des lecteurs : {}", self.0)
    }
}
impl std::error::Error for DriveScanStalled {}

/// Lance une commande makemkvcon et appelle `on` pour chaque ligne robot.
///
/// Les lignes `MSG` sont recopiées dans le journal ; les problèmes de clé sont
/// mémorisés et renvoyés sous forme de [`KeyProblem`].
///
/// Si MakeMKV ne liste pas les lecteurs (`DRV:`) dans le délai
/// [`sdf::SCAN_TIMEOUT`], il est considéré comme bloqué à l'étape SDF : le
/// processus est tué, `sdf_Stop` est écrit d'après le journal de débogage et la
/// commande est relancée une fois.
async fn run_robot(
    tool: &Tool,
    args: &[String],
    paths: &[&Path],
    tx: &Tx,
    cancel: Option<CancellationToken>,
    on: impl Fn(&RobotLine) + Send + Sync,
) -> Result<std::process::ExitStatus> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match run_robot_once(tool, args, paths, tx, cancel.clone(), &on).await {
            Err(e) if e.is::<DriveScanStalled>() && attempt == 1 => {
                log(tx, format!("⚠ {e}"));
                match sdf::apply_from_log() {
                    Ok(sdf::Fix::Applied(id)) => {
                        log(tx, format!("Ajout de sdf_Stop = \"{id}\" dans ~/.MakeMKV/settings.conf, nouvelle tentative…"));
                        emit(tx, AppEvent::Toast("Blocage MakeMKV contourné (sdf_Stop)".into()));
                    }
                    Ok(sdf::Fix::AlreadySet(id)) => {
                        bail!("{e} (sdf_Stop = \"{id}\" est déjà défini ; essayez MakeMKV 1.18)")
                    }
                    Ok(sdf::Fix::NoId) => bail!("{e} (identifiant SDF du lecteur introuvable dans {})", sdf::debug_log_path().display()),
                    Err(w) => bail!("{e} (écriture de sdf_Stop impossible : {w:#})"),
                }
            }
            other => return other,
        }
    }
}

async fn run_robot_once(
    tool: &Tool,
    args: &[String],
    paths: &[&Path],
    tx: &Tx,
    cancel: Option<CancellationToken>,
    on: &(impl Fn(&RobotLine) + Send + Sync),
) -> Result<std::process::ExitStatus> {
    // `--debug` écrit ~/MakeMKV_log.txt, seul endroit où figure l'identifiant
    // SDF du lecteur ; le Flatpak reçoit l'accès à ce seul fichier.
    let log_file = sdf::ensure_debug_log();
    let mut all_paths: Vec<&Path> = paths.to_vec();
    if let Some(p) = &log_file {
        all_paths.push(p);
    }
    let mut cmd = tool.command(&all_paths);
    if log_file.is_some() {
        cmd.arg("--debug");
    }
    cmd.args(args);
    let key_problem: Mutex<Option<String>> = Mutex::new(None);
    let saw_drive = AtomicBool::new(false);
    let stalled = AtomicBool::new(false);
    let run_cancel = cancel.as_ref().map(|c| c.child_token()).unwrap_or_default();

    let run = run_streaming(cmd, Some(run_cancel.clone()), |_, raw| {
        let line = parse_line(&raw);
        match &line {
            RobotLine::Msg { code, text } => {
                if is_key_problem(*code, text) {
                    key_problem.lock().unwrap().get_or_insert_with(|| text.clone());
                }
                if !text.starts_with("DEBUG:") {
                    log(tx, format!("[MakeMKV] {text}"));
                }
            }
            RobotLine::Drive(_) => saw_drive.store(true, Ordering::Relaxed),
            RobotLine::Unknown(s) => log(tx, format!("[sortie] {s}")),
            _ => {}
        }
        on(&line);
    });
    let watchdog = async {
        tokio::time::sleep(sdf::SCAN_TIMEOUT).await;
        if !saw_drive.load(Ordering::Relaxed) {
            stalled.store(true, Ordering::Relaxed);
            run_cancel.cancel();
        }
        std::future::pending::<()>().await
    };
    let res = tokio::select! {
        r = run => r,
        _ = watchdog => unreachable!(),
    };
    if stalled.load(Ordering::Relaxed) {
        return Err(DriveScanStalled(format!("aucun lecteur listé après {} s", sdf::SCAN_TIMEOUT.as_secs())).into());
    }
    let status = res?;

    if let Some(p) = key_problem.into_inner().unwrap() {
        return Err(KeyProblem(p).into());
    }
    Ok(status)
}

/// Sonde rapide : version, lecteurs et état de la clé (aucun disque n'est lu).
pub async fn probe(tool: &Tool, tx: &Tx) -> Result<ProbeResult> {
    let res = Mutex::new(ProbeResult::default());
    let version_re = regex::Regex::new(r"MakeMKV v(\d+\.\d+\.\d+)").unwrap();
    let args = ["-r", "--cache=1", "info", "disc:9999"].map(String::from);
    let outcome = run_robot(tool, &args, &[], tx, None, |l| {
        let mut r = res.lock().unwrap();
        match l {
            RobotLine::Msg { text, .. } => {
                if let Some(c) = version_re.captures(text) {
                    r.version = Some(c[1].to_string());
                }
            }
            RobotLine::Drive(d) if d.exists() => r.drives.push(d.clone()),
            _ => {}
        }
    })
    .await;
    let mut res = res.into_inner().unwrap();
    match outcome {
        Ok(_) => Ok(res),
        Err(e) => match e.downcast::<KeyProblem>() {
            Ok(kp) => {
                res.key_problem = Some(kp.0);
                Ok(res)
            }
            Err(e) => Err(e),
        },
    }
}

/// Analyse complète du disque présent dans `device` (ex. `/dev/sr0`).
pub async fn analyze(tool: &Tool, device: &str, min_length: u32, tx: &Tx, cancel: CancellationToken) -> Result<DiscInfo> {
    let lines = Mutex::new(Vec::<RobotLine>::new());
    let label = Mutex::new(String::from("Analyse du disque…"));
    let args = vec![
        "-r".into(),
        "--progress=-same".into(),
        format!("--minlength={min_length}"),
        "info".into(),
        format!("dev:{device}"),
    ];
    let status = run_robot(tool, &args, &[], tx, Some(cancel), |l| match l {
        RobotLine::ProgressTitle { name } | RobotLine::ProgressCurrent { name } => {
            *label.lock().unwrap() = name.clone();
            emit(tx, AppEvent::AnalysisProgress { label: name.clone(), fraction: None });
        }
        RobotLine::ProgressValue { total, max, .. } if *max > 0 => {
            emit(
                tx,
                AppEvent::AnalysisProgress {
                    label: label.lock().unwrap().clone(),
                    fraction: Some(*total as f64 / *max as f64),
                },
            );
        }
        RobotLine::DiscAttr { .. } | RobotLine::TitleAttr { .. } | RobotLine::StreamAttr { .. } => {
            lines.lock().unwrap().push(l.clone())
        }
        _ => {}
    })
    .await?;

    let lines = lines.into_inner().unwrap();
    if lines.is_empty() {
        bail!("MakeMKV n'a renvoyé aucune information (code {status}). Le disque est-il lisible ?");
    }
    let mut info = DiscInfo::from_lines(&lines);
    info.device = device.to_string();
    if info.titles.is_empty() {
        bail!("aucun titre de plus de {min_length} s trouvé sur le disque");
    }
    Ok(info)
}

/// Extrait le titre `title` dans `out_dir` ; renvoie le chemin du MKV produit.
///
/// `on_progress(fraction)` reçoit la progression globale (0.0 – 1.0) calculée
/// à partir de `PRGV:courant,total,max` (on utilise `total/max`).
pub async fn rip(
    tool: &Tool,
    device: &str,
    title: usize,
    min_length: u32,
    out_dir: &Path,
    tx: &Tx,
    cancel: CancellationToken,
    on_progress: impl Fn(String, f64) + Send + Sync,
) -> Result<PathBuf> {
    std::fs::create_dir_all(out_dir).context("création du dossier temporaire")?;
    let args = vec![
        "-r".into(),
        "--progress=-same".into(),
        format!("--minlength={min_length}"),
        "mkv".into(),
        format!("dev:{device}"),
        title.to_string(),
        out_dir.display().to_string(),
    ];
    let label = Mutex::new(String::from("Extraction"));
    let last = Mutex::new(-1.0f64);
    let mut attempt = 0;
    let title_failed = AtomicBool::new(false);
    let read_errors = AtomicU32::new(0);
    let status = loop {
        attempt += 1;
        let open_failed = AtomicBool::new(false);
        let status = run_robot(tool, &args, &[out_dir], tx, Some(cancel.clone()), |l| match l {
            RobotLine::Msg { code: MSG_OPEN_FAILED, .. } => open_failed.store(true, Ordering::Relaxed),
            RobotLine::Msg { code: MSG_TITLE_FAILED, .. } => title_failed.store(true, Ordering::Relaxed),
            RobotLine::Msg { code: MSG_READ_ERROR, .. } => {
                read_errors.fetch_add(1, Ordering::Relaxed);
            }
            RobotLine::ProgressTitle { name } => *label.lock().unwrap() = name.clone(),
            RobotLine::ProgressValue { total, max, .. } if *max > 0 => {
                let f = *total as f64 / *max as f64;
                // Limitation du débit d'événements : un toutes les 0,1 %.
                let mut last = last.lock().unwrap();
                if (f - *last).abs() >= 0.001 {
                    *last = f;
                    on_progress(label.lock().unwrap().clone(), f);
                }
            }
            _ => {}
        })
        .await?;
        if !open_failed.load(Ordering::Relaxed) {
            break status;
        }
        // Le disque ne s'ouvre pas : soit le MakeMKV précédent (sélection
        // modifiée) libère encore le lecteur, soit le lecteur ne répond plus.
        if attempt >= OPEN_ATTEMPTS {
            emit(tx, AppEvent::Toast("Le lecteur ne répond plus : éjectez puis réinsérez le disque".into()));
            bail!(
                "le lecteur ne répond plus (MakeMKV : « Échec de l'ouverture du disque »). \
                 Éjectez puis réinsérez le disque ; si cela persiste, débranchez et rebranchez le lecteur"
            );
        }
        log(tx, format!("Le disque ne s'ouvre pas (tentative {attempt}/{OPEN_ATTEMPTS}) : nouvel essai dans 4 s…"));
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(4)) => {}
            _ = cancel.cancelled() => bail!("opération annulée"),
        }
    };

    // MakeMKV a abandonné le titre (le plus souvent : zone du disque illisible).
    // Il peut laisser un fichier partiel et sortir avec le code 0 : ne pas s'y fier.
    if title_failed.load(Ordering::Relaxed) {
        let n = read_errors.load(Ordering::Relaxed);
        if n > 0 {
            emit(tx, AppEvent::Toast("Échec : le disque est illisible par endroits".into()));
            bail!("le disque est illisible par endroits ({n} erreurs de lecture) : MakeMKV a abandonné le film");
        }
        bail!("MakeMKV n'a pas pu écrire le film (« Échec à l'écriture du titre ») : voir le journal ci-dessus");
    }

    // On retient le plus gros .mkv produit.
    let produced = std::fs::read_dir(out_dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "mkv"))
        .max_by_key(|p| p.metadata().map(|m| m.len()).unwrap_or(0));
    match produced {
        Some(p) if status.success() => Ok(p),
        Some(p) => {
            log(tx, format!("⚠ makemkvcon s'est terminé avec le code {status}, fichier produit malgré tout."));
            Ok(p)
        }
        None => bail!("makemkvcon n'a produit aucun fichier (code {status})"),
    }
}
