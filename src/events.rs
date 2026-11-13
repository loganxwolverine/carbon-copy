//! Événements envoyés des tâches tokio (threads de travail) vers le thread GTK.
//!
//! Le canal est non borné : `emit` ne bloque jamais, on peut donc l'appeler
//! depuis n'importe quel callback, y compris pendant la lecture d'un flux.

use std::path::PathBuf;

use crate::makemkv::disc::DiscInfo;
use crate::process::Tool;

#[derive(Debug)]
pub enum AppEvent {
    /// Message d'état affiché pendant l'initialisation.
    Status(String),
    /// Ligne ajoutée au journal.
    Log(String),
    /// Notification discrète (toast libadwaita).
    Toast(String),
    /// Initialisation terminée : outils prêts.
    Ready { makemkv: Tool, mkvmerge: Tool },
    /// Échec bloquant de l'initialisation.
    SetupFailed(String),
    /// Changement d'état du lecteur optique (détection par ioctl).
    DriveState { device: String, has_disc: bool },
    /// Progression de l'analyse du disque.
    AnalysisProgress { label: String, fraction: Option<f64> },
    /// Analyse terminée.
    DiscAnalyzed(Result<DiscInfo, String>),
    /// Métadonnées trouvées (TMDB ou déduites du disque).
    Metadata { title: String, year: Option<u32>, source: String },
    /// Progression du rip (0.0 – 1.0) et libellé de phase.
    RipProgress { label: String, fraction: f64 },
    /// Fin du rip.
    RipFinished(Result<PathBuf, String>),
}

pub type Tx = async_channel::Sender<AppEvent>;

pub fn emit(tx: &Tx, ev: AppEvent) {
    let _ = tx.try_send(ev);
}

pub fn log(tx: &Tx, msg: impl Into<String>) {
    emit(tx, AppEvent::Log(msg.into()));
}

pub fn status(tx: &Tx, msg: impl Into<String>) {
    let msg = msg.into();
    emit(tx, AppEvent::Log(msg.clone()));
    emit(tx, AppEvent::Status(msg));
}

