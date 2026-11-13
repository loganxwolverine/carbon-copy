//! Carbon Copy — interface GTK4 / libadwaita automatisée pour MakeMKV.
//!
//! Architecture :
//! - `ui`        : fenêtre, widgets, contrôleur (thread GTK uniquement)
//! - `backend`   : orchestration des tâches de fond (runtime tokio)
//! - `makemkv`   : installation, réglages, parsing robot et pilotage de makemkvcon
//! - `beta_key`  : récupération / installation autonome de la clé bêta
//! - `selection` : règles de pré-sélection des pistes (sous-titres forcés…)
//! - `metadata`  : nettoyage du nom de disque, TMDB, nommage du fichier
//! - `remux`     : tri final des pistes avec mkvmerge
//! - `drive`     : détection d'insertion de disque (ioctl)

mod backend;
mod beta_key;
mod config;
mod drive;
mod events;
mod makemkv;
mod metadata;
mod process;
mod remux;
mod selection;
mod ui;

use adw::prelude::*;

pub const APP_ID: &str = "io.github.carboncopy.CarbonCopy";

fn main() -> gtk::glib::ExitCode {
    // Démarre le runtime tokio avant GTK (création paresseuse, threads dédiés).
    let _ = backend::runtime();
    config::migrate_legacy_dir();

    let app = adw::Application::builder().application_id(APP_ID).build();
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
