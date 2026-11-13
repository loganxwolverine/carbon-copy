//! Contrôleur de l'interface : relie les widgets, l'état et les tâches de fond.

mod prefs;
mod widgets;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use tokio_util::sync::CancellationToken;

use crate::backend::{self, runtime, RipJob};
use crate::config::Config;
use crate::events::{AppEvent, Tx};
use crate::makemkv::disc::{DiscInfo, StreamKind, Title};
use crate::makemkv::robot::format_duration;
use crate::metadata;
use crate::process::Tool;
use crate::selection::{default_selection, Selection};

pub use widgets::APP_NAME;
use widgets::{append_log, Ui};

#[derive(Default)]
struct State {
    cfg: Config,
    makemkv: Option<Tool>,
    mkvmerge: Option<Tool>,
    /// Lecteurs contenant actuellement un disque.
    loaded_drives: BTreeSet<String>,
    device: Option<String>,
    disc: Option<DiscInfo>,
    title_index: usize,
    selection: Selection,
    analysis_cancel: Option<CancellationToken>,
    rip_cancel: Option<CancellationToken>,
    inhibit_cookie: Option<u32>,
    output_dir: PathBuf,
    /// Empêche les callbacks de réagir pendant un remplissage programmatique.
    updating: bool,
}

pub struct App {
    app: adw::Application,
    ui: Ui,
    state: RefCell<State>,
    tx: Tx,
}

/// Point d'entrée de l'interface (appelé sur `activate`).
pub fn build(app: &adw::Application) {
    if let Some(w) = app.active_window() {
        w.present();
        return;
    }
    let (tx, rx) = async_channel::unbounded::<AppEvent>();
    let cfg = Config::load();
    let state = State { output_dir: cfg.output_dir.clone(), cfg, ..Default::default() };
    let this = Rc::new(App { app: app.clone(), ui: widgets::build(app), state: RefCell::new(state), tx });

    this.connect_signals();
    this.install_actions();
    this.ui.folder_row.set_subtitle(&this.state.borrow().output_dir.display().to_string());

    // Boucle de réception des événements, exécutée sur le thread GTK :
    // c'est le SEUL endroit où les résultats des tâches tokio touchent l'interface.
    let me = this.clone();
    glib::spawn_future_local(async move {
        while let Ok(ev) = rx.recv().await {
            me.handle(ev);
        }
    });

    // Surveillance du lecteur optique (dure toute la vie de l'application).
    runtime().spawn(crate::drive::watch(this.tx.clone(), CancellationToken::new()));
    this.start_setup();
    this.ui.window.present();
}

impl App {
    // ------------------------------------------------------------------
    // Signaux
    // ------------------------------------------------------------------
    fn connect_signals(self: &Rc<Self>) {
        let me = self.clone();
        self.ui.retry_btn.connect_clicked(move |_| me.start_setup());

        let me = self.clone();
        self.ui.refresh_btn.connect_clicked(move |_| {
            let dev = {
                let s = me.state.borrow();
                s.device.clone().or_else(|| s.loaded_drives.iter().next().cloned())
            };
            match dev {
                Some(d) => me.start_analysis(d),
                None => me.toast("Aucun disque détecté"),
            }
        });

        let me = self.clone();
        self.ui.title_combo.connect_selected_notify(move |row| {
            if me.state.borrow().updating {
                return;
            }
            me.select_title(row.selected() as usize);
        });

        // Titre / année → nom de fichier régénéré automatiquement.
        for entry in [&self.ui.movie_entry, &self.ui.year_entry] {
            let me = self.clone();
            entry.connect_changed(move |_| me.regenerate_filename());
        }

        let me = self.clone();
        self.ui.tmdb_btn.connect_clicked(move |_| me.search_tmdb());

        let me = self.clone();
        self.ui.folder_btn.connect_clicked(move |_| me.choose_folder());

        let me = self.clone();
        self.ui.rip_btn.connect_clicked(move |_| {
            if me.state.borrow().rip_cancel.is_some() {
                me.cancel_rip();
            } else {
                me.clone().confirm_and_rip();
            }
        });

        let me = self.clone();
        self.ui.window.connect_close_request(move |_| {
            // Annule proprement un rip en cours (le garde restaure la config MakeMKV).
            let s = me.state.borrow();
            for c in [&s.rip_cancel, &s.analysis_cancel].into_iter().flatten() {
                c.cancel();
            }
            glib::Propagation::Proceed
        });
    }

    fn install_actions(self: &Rc<Self>) {
        let prefs = gio::SimpleAction::new("preferences", None);
        let me = self.clone();
        prefs.connect_activate(move |_, _| me.show_preferences());
        self.app.add_action(&prefs);
        self.app.set_accels_for_action("app.preferences", &["<Control>comma"]);

        let about = gio::SimpleAction::new("about", None);
        let me = self.clone();
        about.connect_activate(move |_, _| {
            let d = adw::AboutDialog::builder()
                .application_name(APP_NAME)
                .application_icon(crate::APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .comments("Interface GNOME automatisée pour MakeMKV")
                .license_type(gtk::License::Gpl30)
                .build();
            d.present(Some(&me.ui.window));
        });
        self.app.add_action(&about);

        let quit = gio::SimpleAction::new("quit", None);
        let me = self.clone();
        quit.connect_activate(move |_, _| me.ui.window.close());
        self.app.add_action(&quit);
        self.app.set_accels_for_action("app.quit", &["<Control>q"]);
    }

    // ------------------------------------------------------------------
    // Événements venant des tâches de fond
    // ------------------------------------------------------------------
    fn handle(self: &Rc<Self>, ev: AppEvent) {
        match ev {
            AppEvent::Status(s) => {
                self.ui.setup_page.set_description(Some(&s));
                self.ui.window_title.set_subtitle(&s);
            }
            AppEvent::Log(l) => {
                eprintln!("{l}");
                append_log(&self.ui.log_view, &l)
            }
            AppEvent::Toast(t) => self.toast(&t),
            AppEvent::Ready { makemkv, mkvmerge } => {
                self.ui.window_title.set_subtitle(&format!("MakeMKV ({})", makemkv.short_label()));
                append_log(&self.ui.log_view, &format!("Prêt — MakeMKV : {}", makemkv.describe()));
                {
                    let mut s = self.state.borrow_mut();
                    s.makemkv = Some(makemkv);
                    s.mkvmerge = Some(mkvmerge);
                }
                self.ui.refresh_btn.set_sensitive(true);
                let first = self.state.borrow().loaded_drives.iter().next().cloned();
                match first {
                    Some(d) => self.start_analysis(d),
                    None => self.ui.stack.set_visible_child_name("nodisc"),
                }
            }
            AppEvent::SetupFailed(e) => {
                append_log(&self.ui.log_view, &format!("✖ {e}"));
                self.ui.error_page.set_description(Some(&e));
                self.ui.stack.set_visible_child_name("error");
                self.ui.window_title.set_subtitle("");
            }
            AppEvent::DriveState { device, has_disc } => self.on_drive_state(device, has_disc),
            AppEvent::AnalysisProgress { label, fraction } => {
                self.ui.analysis_bar.set_text(Some(&label));
                match fraction {
                    Some(f) => self.ui.analysis_bar.set_fraction(f),
                    None => self.ui.analysis_bar.pulse(),
                }
            }
            AppEvent::DiscAnalyzed(res) => {
                self.state.borrow_mut().analysis_cancel = None;
                match res {
                    Ok(info) => self.show_disc(info),
                    Err(e) => {
                        append_log(&self.ui.log_view, &format!("✖ Analyse impossible : {e}"));
                        if !e.contains("annulée") {
                            self.ui.error_page.set_title("Analyse du disque impossible");
                            self.ui.error_page.set_description(Some(&e));
                            self.ui.stack.set_visible_child_name("error");
                        }
                    }
                }
            }
            AppEvent::Metadata { title, year, source } => {
                self.ui.movie_entry.set_text(&title);
                self.ui.year_entry.set_text(&year.map(|y| y.to_string()).unwrap_or_default());
                self.regenerate_filename();
                if source == "TMDB" {
                    self.toast(&format!("Film identifié : {title}"));
                }
            }
            AppEvent::RipProgress { label, fraction } => {
                self.ui.progress_bar.set_fraction(fraction.clamp(0.0, 1.0));
                self.ui.progress_bar.set_text(Some(&format!("{:.1} %", fraction * 100.0)));
                self.ui.progress_label.set_text(&label);
            }
            AppEvent::RipFinished(res) => self.on_rip_finished(res),
        }
    }

    fn toast(&self, msg: &str) {
        self.ui.toasts.add_toast(adw::Toast::new(msg));
    }

    // ------------------------------------------------------------------
    // Initialisation / détection
    // ------------------------------------------------------------------
    fn start_setup(self: &Rc<Self>) {
        self.ui.stack.set_visible_child_name("setup");
        self.ui.error_page.set_title("MakeMKV n'est pas disponible");
        let cfg = self.state.borrow().cfg.clone();
        runtime().spawn(backend::startup(self.tx.clone(), cfg));
    }

    fn on_drive_state(self: &Rc<Self>, device: String, has_disc: bool) {
        append_log(
            &self.ui.log_view,
            &format!("{device} : {}", if has_disc { "disque inséré" } else { "aucun disque" }),
        );
        let (ready, ripping, current) = {
            let mut s = self.state.borrow_mut();
            if has_disc {
                s.loaded_drives.insert(device.clone());
            } else {
                s.loaded_drives.remove(&device);
            }
            (s.makemkv.is_some(), s.rip_cancel.is_some(), s.device.clone())
        };
        if !ready || ripping {
            return; // Pendant un rip, on ignore tout changement d'état.
        }
        if has_disc {
            self.start_analysis(device);
        } else if current.as_deref() == Some(device.as_str()) {
            if let Some(c) = self.state.borrow_mut().analysis_cancel.take() {
                c.cancel();
            }
            let mut s = self.state.borrow_mut();
            s.disc = None;
            s.device = None;
            drop(s);
            self.ui.stack.set_visible_child_name("nodisc");
        }
    }

    fn start_analysis(self: &Rc<Self>, device: String) {
        let (tool, cfg) = {
            let mut s = self.state.borrow_mut();
            let Some(tool) = s.makemkv.clone() else { return };
            if s.rip_cancel.is_some() {
                return;
            }
            if let Some(c) = s.analysis_cancel.take() {
                c.cancel();
            }
            s.device = Some(device.clone());
            (tool, s.cfg.clone())
        };
        let cancel = CancellationToken::new();
        self.state.borrow_mut().analysis_cancel = Some(cancel.clone());
        self.ui.analysis_bar.set_fraction(0.0);
        self.ui.analysis_bar.set_text(Some("Ouverture du disque…"));
        self.ui.stack.set_visible_child_name("analysis");
        runtime().spawn(backend::analyze(self.tx.clone(), cfg, tool, device, cancel));
    }

    // ------------------------------------------------------------------
    // Affichage du disque et des pistes
    // ------------------------------------------------------------------
    fn show_disc(self: &Rc<Self>, info: DiscInfo) {
        let main = info.main_title().unwrap_or(0);
        self.ui.type_row.set_subtitle(info.media_type().label());
        self.ui.name_row.set_subtitle(&info.name());

        // Liste des titres (le titre principal est présélectionné).
        let labels: Vec<String> = info
            .titles
            .iter()
            .map(|t| {
                let mut l = format!(
                    "Titre {} — {} · {} chap. · {}",
                    t.index,
                    format_duration(t.duration_secs()),
                    t.chapters(),
                    t.size_label()
                );
                if t.index == main {
                    l.push_str("  ★ principal");
                }
                l
            })
            .collect();
        {
            let mut s = self.state.borrow_mut();
            s.updating = true;
            s.disc = Some(info);
        }
        let n = self.ui.title_model.n_items();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.ui.title_model.splice(0, n, &refs);
        self.ui.title_combo.set_selected(main as u32);
        self.state.borrow_mut().updating = false;

        self.select_title(main);
        self.ui.progress_bar.set_fraction(0.0);
        self.ui.progress_bar.set_text(Some("Prêt"));
        self.ui.progress_label.set_text("");
        self.ui.stack.set_visible_child_name("disc");
    }

    /// Change de titre : recalcule la sélection par défaut et reconstruit les listes.
    fn select_title(self: &Rc<Self>, index: usize) {
        let title = {
            let mut s = self.state.borrow_mut();
            let Some(t) = s.disc.as_ref().and_then(|d| d.titles.get(index)).cloned() else { return };
            s.title_index = index;
            s.selection = default_selection(&t, &s.cfg);
            t
        };
        self.ui.duration_row.set_subtitle(&format!(
            "{} ({})",
            format_duration(title.duration_secs()),
            if title.source().is_empty() { "—" } else { title.source() }
        ));
        self.rebuild_tracks(&title);
    }

    fn rebuild_tracks(self: &Rc<Self>, title: &Title) {
        let sel = self.state.borrow().selection.clone();
        self.ui.audio_list.remove_all();
        self.ui.subs_list.remove_all();

        for s in title.streams_of(StreamKind::Audio) {
            let row = self.track_row(&s.lang_name(), &s.summary(), sel.audio.contains(&s.index), s.index, StreamKind::Audio);
            self.ui.audio_list.append(&row);
        }
        for s in title.streams_of(StreamKind::Subtitle) {
            let (name, sub) = if s.is_forced() {
                (format!("{} — forcés", s.lang_name()), format!("{} · passages en langue étrangère uniquement", s.codec()))
            } else {
                (s.lang_name(), format!("{} · sous-titres complets", s.codec()))
            };
            let row = self.track_row(&name, &sub, sel.subtitles.contains(&s.index), s.index, StreamKind::Subtitle);
            if s.is_forced() {
                let tag = gtk::Label::builder().label("forcé").valign(gtk::Align::Center).css_classes(["accent", "caption-heading"]).build();
                row.add_suffix(&tag);
            }
            self.ui.subs_list.append(&row);
        }
        if title.streams_of(StreamKind::Audio).next().is_none() {
            self.ui.audio_list.append(&adw::ActionRow::builder().title("Aucune piste audio").build());
        }
        if title.streams_of(StreamKind::Subtitle).next().is_none() {
            self.ui.subs_list.append(&adw::ActionRow::builder().title("Aucun sous-titre").build());
        }
    }

    /// Ligne « case à cocher » pour une piste ; met à jour la sélection au clic.
    fn track_row(self: &Rc<Self>, title: &str, subtitle: &str, active: bool, index: usize, kind: StreamKind) -> adw::ActionRow {
        let check = gtk::CheckButton::builder().active(active).valign(gtk::Align::Center).build();
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(title).as_str()).subtitle(glib::markup_escape_text(subtitle).as_str()).build();
        row.add_prefix(&check);
        row.set_activatable_widget(Some(&check));
        let me = self.clone();
        check.connect_toggled(move |c| {
            let mut s = me.state.borrow_mut();
            let set = if kind == StreamKind::Audio { &mut s.selection.audio } else { &mut s.selection.subtitles };
            if c.is_active() {
                set.insert(index);
            } else {
                set.remove(&index);
            }
        });
        row
    }

    // ------------------------------------------------------------------
    // Sortie : titre, nom de fichier, dossier
    // ------------------------------------------------------------------
    fn year(&self) -> Option<u32> {
        self.ui.year_entry.text().trim().parse().ok().filter(|y| (1880..=2100).contains(y))
    }

    fn regenerate_filename(&self) {
        let name = metadata::output_filename(self.ui.movie_entry.text().trim(), self.year());
        self.ui.filename_entry.set_text(&name);
    }

    fn search_tmdb(self: &Rc<Self>) {
        let cfg = self.state.borrow().cfg.clone();
        if cfg.tmdb_api_key.trim().is_empty() {
            self.toast("Ajoutez une clé API TMDB dans les préférences");
            return;
        }
        let query = self.ui.movie_entry.text().to_string();
        let year = self.year();
        let tx = self.tx.clone();
        runtime().spawn(async move {
            match metadata::tmdb_search(backend::http(), cfg.tmdb_api_key.trim(), &query, year, &cfg.language).await {
                Ok(Some((title, year))) => {
                    crate::events::emit(&tx, AppEvent::Metadata { title, year, source: "TMDB".into() })
                }
                Ok(None) => crate::events::emit(&tx, AppEvent::Toast(format!("TMDB : aucun résultat pour « {query} »"))),
                Err(e) => crate::events::emit(&tx, AppEvent::Toast(format!("TMDB : {e}"))),
            }
        });
    }

    fn choose_folder(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title("Dossier de destination").modal(true).build();
        dialog.set_initial_folder(Some(&gio::File::for_path(&self.state.borrow().output_dir)));
        let me = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(folder) = dialog.select_folder_future(Some(&me.ui.window)).await {
                if let Some(path) = folder.path() {
                    me.ui.folder_row.set_subtitle(&path.display().to_string());
                    me.state.borrow_mut().output_dir = path;
                }
            }
        });
    }

    // ------------------------------------------------------------------
    // Rip
    // ------------------------------------------------------------------
    fn output_path(&self) -> Option<PathBuf> {
        let mut name = self.ui.filename_entry.text().trim().replace('/', "_");
        if name.is_empty() {
            return None;
        }
        if !name.to_lowercase().ends_with(".mkv") {
            name.push_str(".mkv");
        }
        Some(self.state.borrow().output_dir.join(name))
    }

    async fn ask(&self, heading: &str, body: &str, ok: &str) -> bool {
        let d = adw::AlertDialog::new(Some(heading), Some(body));
        d.add_responses(&[("cancel", "Annuler"), ("ok", ok)]);
        d.set_response_appearance("ok", adw::ResponseAppearance::Destructive);
        d.set_default_response(Some("cancel"));
        d.set_close_response("cancel");
        d.choose_future(Some(&self.ui.window)).await == "ok"
    }

    fn confirm_and_rip(self: Rc<Self>) {
        glib::spawn_future_local(async move {
            let Some(output) = self.output_path() else {
                self.toast("Indiquez un nom de fichier");
                return;
            };
            if self.state.borrow().selection.audio.is_empty()
                && !self.ask("Aucune piste audio", "Le fichier final n'aura pas de son. Continuer ?", "Continuer").await
            {
                return;
            }
            if output.exists()
                && !self
                    .ask("Le fichier existe déjà", &format!("Remplacer « {} » ?", output.display()), "Remplacer")
                    .await
            {
                return;
            }
            self.start_rip(output);
        });
    }

    fn start_rip(self: &Rc<Self>, output: PathBuf) {
        let job = {
            let s = self.state.borrow();
            let (Some(makemkv), Some(mkvmerge), Some(device), Some(disc)) = (&s.makemkv, &s.mkvmerge, &s.device, &s.disc) else {
                return;
            };
            let Some(title) = disc.titles.get(s.title_index).cloned() else { return };
            let movie = self.ui.movie_entry.text().trim().to_string();
            RipJob {
                makemkv: makemkv.clone(),
                mkvmerge: mkvmerge.clone(),
                device: device.clone(),
                title,
                selection: s.selection.clone(),
                output,
                movie_title: match self.year() {
                    Some(y) => format!("{movie} ({y})"),
                    None => movie,
                },
                cfg: s.cfg.clone(),
            }
        };
        let cancel = CancellationToken::new();
        // Empêche la mise en veille pendant le rip (qui peut durer une heure).
        let cookie = self.app.inhibit(Some(&self.ui.window), gtk::ApplicationInhibitFlags::SUSPEND | gtk::ApplicationInhibitFlags::LOGOUT, Some("Rip en cours"));
        {
            let mut s = self.state.borrow_mut();
            s.rip_cancel = Some(cancel.clone());
            s.inhibit_cookie = Some(cookie);
        }
        self.set_ripping_ui(true);
        append_log(&self.ui.log_view, &format!("▶ Rip vers {}", job.output.display()));
        runtime().spawn(backend::rip(self.tx.clone(), job, cancel));
    }

    fn cancel_rip(self: &Rc<Self>) {
        let me = self.clone();
        glib::spawn_future_local(async move {
            if me.ask("Annuler le rip ?", "Le fichier partiel sera supprimé.", "Annuler le rip").await {
                if let Some(c) = me.state.borrow().rip_cancel.as_ref() {
                    c.cancel();
                }
            }
        });
    }

    fn set_ripping_ui(&self, ripping: bool) {
        self.ui.disc_content.set_sensitive(!ripping);
        self.ui.refresh_btn.set_sensitive(!ripping);
        if ripping {
            self.ui.rip_btn.set_label("Annuler");
            self.ui.rip_btn.remove_css_class("suggested-action");
            self.ui.rip_btn.add_css_class("destructive-action");
        } else {
            self.ui.rip_btn.set_label("Lancer le Rip");
            self.ui.rip_btn.remove_css_class("destructive-action");
            self.ui.rip_btn.add_css_class("suggested-action");
        }
    }

    fn on_rip_finished(self: &Rc<Self>, res: Result<PathBuf, String>) {
        {
            let mut s = self.state.borrow_mut();
            s.rip_cancel = None;
            if let Some(c) = s.inhibit_cookie.take() {
                self.app.uninhibit(c);
            }
        }
        self.set_ripping_ui(false);
        match res {
            Ok(path) => {
                let msg = format!("Rip terminé : {}", path.file_name().unwrap_or_default().to_string_lossy());
                append_log(&self.ui.log_view, &format!("✔ {}", path.display()));
                self.toast(&msg);
                // Notification système si la fenêtre n'a pas le focus.
                if !self.ui.window.is_active() {
                    let n = gio::Notification::new("Rip terminé");
                    n.set_body(Some(&path.display().to_string()));
                    self.app.send_notification(Some("rip-finished"), &n);
                }
            }
            Err(e) if e.contains("annulée") => {
                append_log(&self.ui.log_view, "Rip annulé.");
                self.ui.progress_bar.set_fraction(0.0);
                self.ui.progress_bar.set_text(Some("Annulé"));
                self.toast("Rip annulé");
            }
            Err(e) => {
                append_log(&self.ui.log_view, &format!("✖ {e}"));
                self.ui.progress_bar.set_text(Some("Échec"));
                let d = adw::AlertDialog::new(Some("Le rip a échoué"), Some(&e));
                d.add_response("close", "Fermer");
                d.present(Some(&self.ui.window));
            }
        }
        // Un disque a pu être éjecté / changé pendant le rip.
        let (loaded, current) = {
            let s = self.state.borrow();
            (s.loaded_drives.clone(), s.device.clone())
        };
        if let Some(dev) = current {
            if !loaded.contains(&dev) {
                self.ui.stack.set_visible_child_name("nodisc");
            }
        }
    }

    // ------------------------------------------------------------------
    // Préférences
    // ------------------------------------------------------------------
    fn show_preferences(self: &Rc<Self>) {
        let cfg = self.state.borrow().cfg.clone();
        let me = self.clone();
        prefs::show(&self.ui.window, cfg, self.tx.clone(), move |new_cfg| {
            if let Err(e) = new_cfg.save() {
                me.toast(&format!("Préférences non enregistrées : {e}"));
            }
            let lang_changed = {
                let mut s = me.state.borrow_mut();
                let changed = s.cfg.language != new_cfg.language || s.cfg.keep_original_audio != new_cfg.keep_original_audio;
                if s.cfg.output_dir != new_cfg.output_dir {
                    s.output_dir = new_cfg.output_dir.clone();
                }
                s.cfg = new_cfg;
                changed && s.rip_cancel.is_none()
            };
            me.ui.folder_row.set_subtitle(&me.state.borrow().output_dir.display().to_string());
            // Nouvelle langue → nouvelle sélection par défaut pour le disque courant.
            if lang_changed {
                let idx = me.state.borrow().title_index;
                me.select_title(idx);
            }
        });
    }
}
