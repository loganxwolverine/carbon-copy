//! Intact Bitstream — interface GTK4 / libadwaita automatisée pour MakeMKV.
//!
//! Architecture :
//! - `ui`        : fenêtre, widgets, contrôleur (thread GTK uniquement)
//! - `backend`   : orchestration des tâches de fond (runtime tokio)
//! - `makemkv`   : installation, réglages, parsing robot et pilotage de makemkvcon
//! - `beta_key`  : récupération / installation autonome de la clé bêta
//! - `selection` : règles de pré-sélection des pistes (sous-titres forcés…)
//! - `metadata`  : nettoyage du nom de disque, TMDB, nommage du fichier
//! - `remux`     : vérification, drapeaux (mkvpropedit) ou tri des pistes (mkvmerge)
//! - `progress`  : vitesse et temps restant
//! - `drive`     : détection d'insertion de disque (ioctl)
//! - `remote`    : télécommande web (serveur HTTP local)
//! - `autostart` : lancement à l'ouverture de session

mod autostart;
mod backend;
mod beta_key;
mod config;
mod drive;
mod drive_info;
mod events;
mod history;
mod makemkv;
mod metadata;
mod process;
mod progress;
mod remote;
mod remux;
mod selection;
mod ui;

use adw::prelude::*;

pub const APP_ID: &str = "io.github.loganxwolverine.IntactBitstream";

fn main() -> gtk::glib::ExitCode {
    // Démarre le runtime tokio avant GTK (création paresseuse, threads dédiés).
    let _ = backend::runtime();
    config::migrate_legacy_dir();

    // Mode démo (développement) : instance indépendante de celle de l'utilisateur.
    let flags = if std::env::var_os("INTACT_BITSTREAM_DEMO").is_some() {
        gtk::gio::ApplicationFlags::NON_UNIQUE
    } else {
        gtk::gio::ApplicationFlags::empty()
    };
    let app = adw::Application::builder().application_id(APP_ID).flags(flags).build();
    app.connect_startup(|_| {
        // Icône : thème installé (~/.local/share/icons) ou, à défaut, celle des
        // sources (lancement depuis `target/`).
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::IconTheme::for_display(&display).add_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons"));
        }
        gtk::Window::set_default_icon_name(APP_ID);
    });
    app.connect_activate(ui::build);
    app.run()
}
