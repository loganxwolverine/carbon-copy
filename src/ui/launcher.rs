//! Progression sur l'icône de la barre des tâches (KDE Plasma, Dock d'Ubuntu,
//! Dash to Dock…) via l'API « Unity LauncherEntry » : un signal D-Bus
//! `com.canonical.Unity.LauncherEntry.Update` porte la fraction et sa
//! visibilité pour l'application `application://<id>.desktop`.

use std::cell::Cell;
use std::collections::HashMap;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

pub struct Launcher {
    last: Cell<Option<i32>>,
}

impl Launcher {
    pub fn new() -> Self {
        Self { last: Cell::new(None) }
    }

    /// `Some(fraction)` affiche la barre, `None` la masque.
    pub fn set_progress(&self, fraction: Option<f64>) {
        // Un signal par demi-pourcent au plus.
        let key = fraction.map(|f| (f.clamp(0.0, 1.0) * 200.0) as i32);
        if self.last.replace(key) == key {
            return;
        }
        let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return };
        let mut props: HashMap<String, glib::Variant> = HashMap::new();
        props.insert("progress".into(), fraction.unwrap_or(0.0).clamp(0.0, 1.0).to_variant());
        props.insert("progress-visible".into(), fraction.is_some().to_variant());
        let uri = format!("application://{}.desktop", crate::APP_ID);
        let params = (uri, props).to_variant();
        let _ = conn.emit_signal(None, "/io/github/loganxwolverine/IntactBitstream/launcher", "com.canonical.Unity.LauncherEntry", "Update", Some(&params));
    }
}
