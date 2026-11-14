//! Contrôleur de l'interface : relie les widgets, l'état et les tâches de fond.

mod drive_panel;
mod launcher;
mod library;
mod movie_card;
mod movie_dialog;
mod prefs;
mod rip_view;
mod section;
mod theme;
mod timeline;
mod tmdb_setup;
mod widgets;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use tokio_util::sync::CancellationToken;

use crate::backend::{self, runtime, Extracted, Extraction, FinishJob, RIP_WEIGHT};
use crate::config::Config;
use crate::events::{AppEvent, Estimate, Phase, Tx};
use crate::history;
use crate::makemkv::disc::{DiscInfo, StreamKind, Title};
use crate::makemkv::rule;
use crate::makemkv::robot::format_duration;
use crate::metadata::{self, Movie};
use crate::process::Tool;
use crate::selection::{default_selection, Selection};

pub use widgets::APP_NAME;
use widgets::{append_log, Ui};

/// Extraction MakeMKV en cours ou terminée (préparée en arrière-plan).
struct ExtractionState {
    id: u64,
    title: usize,
    cancel: CancellationToken,
    tmp: PathBuf,
    /// Pistes audio / sous-titres que contiendra (ou contient) le fichier.
    contents: Selection,
    /// Pistes vides sur le disque (absentes du fichier) : inutile de relancer
    /// l'extraction pour elles.
    dropped: Selection,
    /// `Some` une fois terminée : fichier produit ou erreur.
    result: Option<Result<Extracted, String>>,
    fraction: f64,
    label: String,
}

impl ExtractionState {
    /// Le fichier en préparation contient-il toutes les pistes de `sel` ?
    fn covers(&self, title: usize, sel: &Selection) -> bool {
        let has = |i: &usize, c: &BTreeSet<usize>, d: &BTreeSet<usize>| c.contains(i) || d.contains(i);
        self.title == title
            && sel.audio.iter().all(|i| has(i, &self.contents.audio, &self.dropped.audio))
            && sel.subtitles.iter().all(|i| has(i, &self.contents.subtitles, &self.dropped.subtitles))
    }
}

#[derive(Default)]
struct State {
    extraction: Option<ExtractionState>,
    extract_seq: u64,
    /// Rip demandé : destination finale, en attente de la fin de l'extraction.
    pending_output: Option<PathBuf>,
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
    /// Film identifié sur TMDB (None : saisie manuelle / non reconnu).
    movie: Option<Movie>,
    /// Compteur de chargement d'affiche (ignore les réponses périmées).
    poster_gen: u64,
    /// Empêche les callbacks de réagir pendant un remplissage programmatique.
    updating: bool,
    /// Titre choisi à la main : la durée TMDB ne le remplace plus.
    title_user_chosen: bool,
    /// Analyse terminée, fiche TMDB attendue avant de lancer l'extraction
    /// (sa durée peut changer le titre principal).
    awaiting_metadata: bool,
    /// Compteur de modifications de la sélection (anti-rebond).
    selection_gen: u64,
    /// Début du rip (durée affichée à la fin).
    rip_started: Option<std::time::Instant>,
    /// Dernier fichier produit (boutons « Lire » et « Ouvrir le dossier »).
    last_output: Option<PathBuf>,
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

    if let Ok(robot_file) = std::env::var("CARBON_COPY_DEMO") {
        this.run_demo(robot_file);
    } else {
        // Surveillance du lecteur optique (dure toute la vie de l'application).
        runtime().spawn(crate::drive::watch(this.tx.clone(), CancellationToken::new()));
        this.start_setup();
    }
    if this.state.borrow().cfg.fullscreen {
        this.ui.window.fullscreen();
    } else {
        this.ui.window.maximize();
    }
    this.ui.window.present();
    this.refresh_drive_info();
    schedule_debug_screenshot(&this.ui.window);
}

/// Annule une extraction et supprime ses fichiers (en arrière-plan).
fn discard(e: ExtractionState) {
    e.cancel.cancel();
    // Extraction encore en cours : c'est elle qui supprime son dossier en
    // s'arrêtant. Sinon, on supprime le MKV brut devenu inutile.
    if e.result.is_some() {
        runtime().spawn(async move { backend::remove_tmp_dir(&e.tmp).await });
    }
}

/// Aide au développement : `CARBON_COPY_SCREENSHOT=/chemin.png` enregistre une
/// capture de la fenêtre toutes les `CARBON_COPY_SCREENSHOT_EVERY` secondes
/// (10 par défaut), pour vérifier la mise en page sans accès à l'écran.
fn schedule_debug_screenshot(window: &adw::ApplicationWindow) {
    let Ok(path) = std::env::var("CARBON_COPY_SCREENSHOT") else { return };
    let every = std::env::var("CARBON_COPY_SCREENSHOT_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let window = window.clone();
    let counter = std::cell::Cell::new(0u32);
    glib::timeout_add_seconds_local(every, move || {
        counter.set(counter.get() + 1);
        let path = path.replace("{n}", &format!("{:02}", counter.get()));
        let (w, h) = (window.width(), window.height());
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, w as f64, h as f64);
        let Some(node) = snapshot.to_node() else {
            eprintln!("capture : fenêtre vide ({w}×{h}, visible : {})", window.is_mapped());
            return glib::ControlFlow::Continue;
        };
        // Moteur de rendu dédié : indépendant de l'état de la fenêtre.
        let renderer = gtk::gsk::CairoRenderer::new();
        if let Err(e) = renderer.realize_for_display(&WidgetExt::display(&window)) {
            eprintln!("capture : {e}");
            return glib::ControlFlow::Continue;
        }
        let texture = renderer.render_texture(node, Some(&gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32)));
        renderer.unrealize();
        if let Err(e) = texture.save_to_png(&path) {
            eprintln!("capture : {e}");
        }
        glib::ControlFlow::Continue
    });
}

impl App {
    /// Mode démo (développement, `CARBON_COPY_DEMO=<sortie robot de makemkvcon info>`) :
    /// accueil, puis disque tiré du fichier (fiche TMDB réelle), puis rip simulé.
    /// Ni MakeMKV ni le lecteur ne sont utilisés, rien n'est écrit.
    fn run_demo(self: &Rc<Self>, robot_file: String) {
        let lines: Vec<_> = std::fs::read_to_string(&robot_file)
            .unwrap_or_default()
            .lines()
            .map(crate::makemkv::robot::parse_line)
            .collect();
        let mut info = DiscInfo::from_lines(&lines);
        info.device = "/dev/sr0".into();
        self.show_nodisc();
        let me = self.clone();
        glib::timeout_add_seconds_local_once(8, move || {
            me.show_disc(info.clone());
            let cfg = me.state.borrow().cfg.clone();
            runtime().spawn(backend::lookup_metadata(me.tx.clone(), cfg, info));
            let me = me.clone();
            glib::timeout_add_seconds_local_once(12, move || {
                let output = me.output_path().unwrap_or_else(|| "/tmp/demo.mkv".into());
                let name = output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let poster = me.ui.movie.poster.paintable();
                me.ui.rip_view.start(&me.display_title(), &name, poster, me.ui.theme.color(), true);
                me.ui.disc_stack.set_visible_child_name("rip");
                me.set_ripping_ui(true);
                let est = Estimate { bytes_per_sec: Some(5.8e6), remaining: std::time::Duration::from_secs(9 * 60) };
                me.show_rip_progress(Phase::Copy, "Enregistrement des titres sur le disque dur", 0.58, Some(est));
                let me = me.clone();
                glib::timeout_add_seconds_local_once(10, move || {
                    me.set_ripping_ui(false);
                    me.ui.launcher.set_progress(None);
                    let secs = {
                        let s = me.state.borrow();
                        s.disc.as_ref().and_then(|d| d.titles.get(s.title_index)).map(|t| t.duration_secs()).unwrap_or(0)
                    };
                    me.ui.rip_view.finish(&me.display_title(), &name, 4_052_305_028, secs, std::time::Duration::from_secs(1105));
                    me.ui.action_zone.set_visible(false);
                });
            });
        });
    }

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
            me.state.borrow_mut().title_user_chosen = true;
            me.select_title(row.selected() as usize);
        });

        // Titre / année → nom de fichier régénéré automatiquement.
        for entry in [&self.ui.movie_entry, &self.ui.year_entry] {
            let me = self.clone();
            entry.connect_changed(move |_| me.regenerate_filename());
        }

        let me = self.clone();
        self.ui.tmdb_btn.connect_clicked(move |_| me.open_movie_dialog());
        let me = self.clone();
        self.ui.movie.change_btn.connect_clicked(move |_| me.open_movie_dialog());

        // Identifiant TMDB : modifie le nom ; Entrée charge la fiche du film.
        let me = self.clone();
        self.ui.tmdb_entry.connect_changed(move |_| me.regenerate_filename());
        let me = self.clone();
        self.ui.tmdb_entry.connect_apply(move |e| match metadata::parse_tmdb_id(&e.text()) {
            Some(id) => me.load_movie_by_id(id),
            None => me.toast("Identifiant TMDB invalide"),
        });
        // Titre saisi à la main : la fiche suit tant qu'aucun film n'est identifié.
        let me = self.clone();
        self.ui.movie_entry.connect_changed(move |_| {
            if me.state.borrow().movie.is_none() {
                me.update_movie_card();
            }
        });

        let me = self.clone();
        self.ui.eject_btn.connect_clicked(move |_| me.eject());

        // Frise des titres : un clic choisit le titre (comme la liste).
        let me = self.clone();
        self.ui.timeline.connect_pick(move |i| me.ui.title_combo.set_selected(i as u32));

        // Nom de fichier découpé en blocs colorés, quelle que soit la saisie.
        let me = self.clone();
        self.ui.filename_entry.connect_changed(move |e| fill_name_parts(&me.ui.name_parts, &e.text()));

        // Écran de fin de rip.
        let me = self.clone();
        self.ui.rip_view.play_btn.connect_clicked(move |_| {
            if let Some(p) = me.state.borrow().last_output.clone() {
                gtk::FileLauncher::new(Some(&gio::File::for_path(p))).launch(Some(&me.ui.window), gio::Cancellable::NONE, |_| {});
            }
        });
        let me = self.clone();
        self.ui.rip_view.folder_btn.connect_clicked(move |_| {
            if let Some(p) = me.state.borrow().last_output.clone() {
                gtk::FileLauncher::new(Some(&gio::File::for_path(p))).open_containing_folder(Some(&me.ui.window), gio::Cancellable::NONE, |_| {});
            }
        });
        let me = self.clone();
        self.ui.rip_view.next_btn.connect_clicked(move |_| me.next_disc());

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
            let mut s = me.state.borrow_mut();
            for c in [&s.rip_cancel, &s.analysis_cancel].into_iter().flatten() {
                c.cancel();
            }
            if let Some(e) = s.extraction.take() {
                discard(e);
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

        let fs = gio::SimpleAction::new("fullscreen", None);
        let me = self.clone();
        fs.connect_activate(move |_, _| {
            let w = &me.ui.window;
            let full = !w.is_fullscreen();
            if full {
                w.fullscreen();
            } else {
                w.unfullscreen();
                w.maximize();
            }
            // On retient le choix pour le prochain lancement.
            let cfg = {
                let mut s = me.state.borrow_mut();
                s.cfg.fullscreen = full;
                s.cfg.clone()
            };
            let _ = cfg.save();
        });
        self.app.add_action(&fs);
        self.app.set_accels_for_action("app.fullscreen", &["F11"]);

        // Lancer le rip au clavier (Ctrl+R), comme le bouton.
        let rip = gio::SimpleAction::new("rip", None);
        let me = self.clone();
        rip.connect_activate(move |_, _| {
            let ready = me.state.borrow().disc.is_some() && me.state.borrow().rip_cancel.is_none();
            if ready && me.ui.rip_btn.is_sensitive() {
                me.clone().confirm_and_rip();
            }
        });
        self.app.add_action(&rip);
        self.app.set_accels_for_action("app.rip", &["<Control>r"]);

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
                // Ménage des copies temporaires de sessions interrompues.
                let (dir, tx) = (self.state.borrow().output_dir.clone(), self.tx.clone());
                runtime().spawn_blocking(move || {
                    for p in backend::sweep_stale_tmp(&dir) {
                        crate::events::log(&tx, format!("Copie temporaire abandonnée supprimée : {}", p.display()));
                    }
                });
                self.ui.refresh_btn.set_sensitive(true);
                self.ui.eject_btn.set_sensitive(!crate::drive::optical_drives().is_empty());
                // La sonde MakeMKV vient d'écrire l'identifiant SDF (date du firmware).
                self.refresh_drive_info();
                // Premier lancement sans clé TMDB : l'assistant est proposé une fois.
                let prompt = {
                    let mut s = self.state.borrow_mut();
                    let p = s.cfg.tmdb_api_key.trim().is_empty() && !s.cfg.tmdb_prompted;
                    if p {
                        s.cfg.tmdb_prompted = true;
                        let _ = s.cfg.save();
                    }
                    p
                };
                if prompt {
                    self.open_tmdb_setup();
                }
                let first = self.state.borrow().loaded_drives.iter().next().cloned();
                match first {
                    Some(d) => self.start_analysis(d),
                    None => self.show_nodisc(),
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
            AppEvent::Metadata { movie, title, year } => {
                match movie {
                    Some(m) => {
                        self.toast(&format!("Film identifié : {}", m.title));
                        self.apply_movie(Some(m));
                    }
                    None => {
                        self.apply_movie(None);
                        self.ui.movie_entry.set_text(&title);
                        self.ui.year_entry.set_text(&year.map(|y| y.to_string()).unwrap_or_default());
                        self.update_movie_card();
                    }
                }
                // Le titre principal est maintenant fixé : l'extraction peut partir.
                let was_waiting = std::mem::take(&mut self.state.borrow_mut().awaiting_metadata);
                if was_waiting {
                    let idx = self.state.borrow().title_index;
                    self.prepare_extraction(idx);
                }
            }
            AppEvent::ExtractProgress { id, phase, label, fraction, estimate } => self.on_extract_progress(id, phase, label, fraction, estimate),
            AppEvent::Extracted { id, result } => self.on_extracted(id, result),
            AppEvent::RipProgress { phase, label, fraction, estimate } => self.show_rip_progress(phase, &label, fraction, estimate),
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

    /// Relit les informations du lecteur courant et met à jour les deux panneaux.
    fn refresh_drive_info(self: &Rc<Self>) {
        let dev = {
            let s = self.state.borrow();
            s.device.clone().or_else(|| s.loaded_drives.iter().next().cloned())
        }
        .or_else(|| crate::drive::optical_drives().into_iter().next());
        let me = self.clone();
        glib::spawn_future_local(async move {
            let info = match dev {
                Some(d) => runtime().spawn_blocking(move || crate::drive_info::gather(&d)).await.ok().flatten(),
                None => None,
            };
            me.ui.nodisc_drive.update(info.as_ref());
            me.ui.disc_drive.update(info.as_ref());
        });
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
        // udev met à jour le type de média inséré un peu après l'ioctl.
        let me = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || me.refresh_drive_info());
        if !ready || ripping {
            return; // Pendant un rip, on ignore tout changement d'état.
        }
        if has_disc {
            self.start_analysis(device);
        } else if current.as_deref() == Some(device.as_str()) {
            if let Some(c) = self.state.borrow_mut().analysis_cancel.take() {
                c.cancel();
            }
            self.discard_extraction();
            let mut s = self.state.borrow_mut();
            s.disc = None;
            s.device = None;
            drop(s);
            // Écran « Copie terminée » : il reste affiché après l'éjection.
            let done_screen = self.ui.stack.visible_child_name().as_deref() == Some("disc")
                && self.ui.disc_stack.visible_child_name().as_deref() == Some("rip");
            if !done_screen {
                self.show_nodisc();
            }
        }
    }

    /// Accueil : invitation à insérer un disque et vidéothèque à jour.
    fn show_nodisc(self: &Rc<Self>) {
        self.ui.stack.set_visible_child_name("nodisc");
        self.ui.launcher.set_progress(None);
        let (dir, key) = {
            let s = self.state.borrow();
            (s.output_dir.clone(), s.cfg.tmdb_api_key.clone())
        };
        let me = self.clone();
        glib::spawn_future_local(async move {
            let entries = runtime().spawn_blocking(move || history::library(&dir)).await.unwrap_or_default();
            let me2 = me.clone();
            let open: Rc<dyn Fn(PathBuf)> = Rc::new(move |p: PathBuf| {
                gtk::FileLauncher::new(Some(&gio::File::for_path(p))).launch(Some(&me2.ui.window), gio::Cancellable::NONE, |_| {});
            });
            me.ui.library.fill(&entries, &key, open);
        });
    }

    /// « Disque suivant » sur l'écran de fin.
    fn next_disc(self: &Rc<Self>) {
        self.ui.disc_stack.set_visible_child_name("details");
        self.ui.action_zone.set_visible(true);
        let (disc, loaded) = {
            let s = self.state.borrow();
            (s.disc.is_some(), s.device.as_ref().is_some_and(|d| s.loaded_drives.contains(d)))
        };
        if !(disc && loaded) {
            self.show_nodisc();
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
            drop(s);
            self.discard_extraction();
            (tool, self.state.borrow().cfg.clone())
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
        {
            let mut s = self.state.borrow_mut();
            s.disc = Some(info);
            s.title_user_chosen = false;
            // Extraction différée jusqu'à la fiche TMDB (choix du titre).
            s.awaiting_metadata = true;
        }
        self.fill_title_list(main);
        self.ui.disc_stack.set_visible_child_name("details");
        self.ui.action_zone.set_visible(true);

        // Fiche du film en attente du résultat TMDB.
        self.apply_movie(None);
        self.ui.movie_entry.set_text("");
        self.ui.year_entry.set_text("");
        self.ui.movie.heading.set_text("Recherche du film…");
        self.ui.movie.details.set_text("");
        self.ui.movie.overview.set_text("");
        self.ui.movie.poster_stack.set_visible_child_name("loading");

        self.select_title(main);
        self.ui.progress_bar.set_fraction(0.0);
        self.ui.progress_bar.set_text(Some("Prêt"));
        self.ui.progress_label.set_text("");
        self.ui.stack.set_visible_child_name("disc");
    }

    /// Liste des titres, `main` marqué « principal » et sélectionné.
    fn fill_title_list(&self, main: usize) {
        let labels: Vec<String> = {
            let s = self.state.borrow();
            let Some(info) = s.disc.as_ref() else { return };
            info.titles
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
                .collect()
        };
        self.state.borrow_mut().updating = true;
        let n = self.ui.title_model.n_items();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.ui.title_model.splice(0, n, &refs);
        self.ui.title_combo.set_selected(main as u32);
        self.state.borrow_mut().updating = false;
        let bars = self
            .state
            .borrow()
            .disc
            .as_ref()
            .map(|d| d.titles.iter().map(|t| timeline::Bar { index: t.index, secs: t.duration_secs(), main: t.index == main }).collect())
            .unwrap_or_default();
        self.ui.timeline.set_titles(bars);
        self.ui.timeline.set_selected(main);
    }

    /// Durée du film connue : choisit le titre qui lui correspond le mieux
    /// (sauf si l'utilisateur a choisi lui-même).
    fn refine_main_title(self: &Rc<Self>) {
        let (best, current, minutes) = {
            let s = self.state.borrow();
            let Some(minutes) = s.movie.as_ref().and_then(|m| m.runtime).filter(|m| *m > 0) else { return };
            if s.title_user_chosen || s.rip_cancel.is_some() {
                return;
            }
            let Some(best) = s.disc.as_ref().and_then(|d| d.main_title_for(Some(minutes as u64 * 60))) else { return };
            (best, s.title_index, minutes)
        };
        let label = crate::progress::short_duration(std::time::Duration::from_secs(minutes as u64 * 60));
        append_log(&self.ui.log_view, &format!("Durée TMDB : {label} → titre principal : {best}"));
        if best != current {
            self.fill_title_list(best);
            self.select_title(best);
            self.toast(&format!("Titre principal ajusté d'après la durée du film ({label})"));
        } else {
            self.fill_title_list(best);
        }
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
        self.ui.timeline.set_selected(index);
        self.ui.duration_row.set_subtitle(&format!(
            "{} ({})",
            format_duration(title.duration_secs()),
            if title.source().is_empty() { "—" } else { title.source() }
        ));
        self.rebuild_tracks(&title);
        self.regenerate_filename();
        self.prepare_extraction(index);
    }

    fn rebuild_tracks(self: &Rc<Self>, title: &Title) {
        let sel = self.state.borrow().selection.clone();
        self.ui.audio_list.remove_all();
        self.ui.subs_list.remove_all();

        for s in title.streams_of(StreamKind::Audio) {
            let name = format!("{}  {}", crate::selection::lang_flag(&s.lang()), s.lang_name());
            let row = self.track_row(&name, &audio_extras(s), sel.audio.contains(&s.index), s.index, StreamKind::Audio);
            for c in codec_chips(s) {
                row.add_suffix(&c);
            }
            self.ui.audio_list.append(&row);
        }
        for s in title.streams_of(StreamKind::Subtitle) {
            let flag = crate::selection::lang_flag(&s.lang());
            let (name, sub) = if s.is_forced() {
                (format!("{flag}  {} — forcés", s.lang_name()), "passages en langue étrangère uniquement".to_string())
            } else {
                (format!("{flag}  {}", s.lang_name()), "sous-titres complets".to_string())
            };
            let row = self.track_row(&name, &sub, sel.subtitles.contains(&s.index), s.index, StreamKind::Subtitle);
            for c in codec_chips(s) {
                row.add_suffix(&c);
            }
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
            drop(s);
            // Les pistes retenues changent les caractéristiques (MULTi, codec…).
            me.regenerate_filename();
            me.on_selection_changed();
        });
        row
    }

    /// Pistes cochées / décochées : si l'extraction en préparation ne contient
    /// plus tout ce qu'il faut (ou vient à peine de commencer), on la relance
    /// avec la nouvelle sélection, après un court délai (clics successifs).
    fn on_selection_changed(self: &Rc<Self>) {
        let gen = {
            let mut s = self.state.borrow_mut();
            s.selection_gen += 1;
            s.selection_gen
        };
        // Sur Blu-ray, chaque relance refait le traitement AACS / BD+ (jusqu'à
        // 1 min 30) : on laisse plus de temps pour finir de cocher.
        let bluray = {
            let s = self.state.borrow();
            s.disc
                .as_ref()
                .and_then(|d| d.titles.get(s.title_index))
                .is_some_and(|t| t.streams_of(StreamKind::Video).any(|v| !v.codec_id().starts_with("V_MPEG2")))
        };
        let delay = if bluray { 5000 } else { 1500 };
        let me = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
            let restart = {
                let s = me.state.borrow();
                if s.selection_gen != gen || s.rip_cancel.is_some() {
                    return;
                }
                let Some(title) = s.disc.as_ref().and_then(|d| d.titles.get(s.title_index)) else { return };
                match s.extraction.as_ref() {
                    Some(e) => {
                        // Copie à peine commencée : on repart si le nouveau plan
                        // évite un remux (fichier plus petit, fin instantanée).
                        let early = e.result.is_none() && e.fraction < 0.25 && rule::plan(title, &s.selection).contents != e.contents;
                        !e.covers(s.title_index, &s.selection) || early
                    }
                    None => false,
                }
            };
            if restart {
                append_log(&me.ui.log_view, "Sélection des pistes modifiée : la préparation repart avec les nouvelles pistes.");
                let idx = me.state.borrow().title_index;
                me.start_extraction(idx);
            }
        });
    }

    // ------------------------------------------------------------------
    // Sortie : titre, nom de fichier, dossier
    // ------------------------------------------------------------------
    fn year(&self) -> Option<u32> {
        self.ui.year_entry.text().trim().parse().ok().filter(|y| (1880..=2100).contains(y))
    }

    fn regenerate_filename(&self) {
        let tags = {
            let s = self.state.borrow();
            match s.disc.as_ref().and_then(|d| d.titles.get(s.title_index).map(|t| (d, t))) {
                Some((d, t)) => metadata::media_tags(d.media_type(), t, &s.selection, &s.cfg.language),
                None => String::new(),
            }
        };
        self.ui.tags_row.set_subtitle(if tags.is_empty() { "—" } else { &tags });
        self.update_counts();
        let tmdb_id = metadata::parse_tmdb_id(&self.ui.tmdb_entry.text());
        let name = metadata::output_filename(self.ui.movie_entry.text().trim(), self.year(), tmdb_id, &tags);
        self.ui.filename_entry.set_text(&name);
    }

    /// Compteurs « cochées / disponibles » dans l'en-tête des sections.
    fn update_counts(&self) {
        let s = self.state.borrow();
        let Some(t) = s.disc.as_ref().and_then(|d| d.titles.get(s.title_index)) else { return };
        let (na, ns) = (t.streams_of(StreamKind::Audio).count(), t.streams_of(StreamKind::Subtitle).count());
        self.ui.audio_section.set_count(&format!("{} / {na}", s.selection.audio.len()));
        self.ui.subs_section.set_count(&format!("{} / {ns}", s.selection.subtitles.len()));
    }

    // ------------------------------------------------------------------
    // Film : fiche, affiche, identification manuelle
    // ------------------------------------------------------------------

    /// Applique un film TMDB (titre, année, identifiant, affiche) ou l'efface.
    fn apply_movie(self: &Rc<Self>, movie: Option<Movie>) {
        self.state.borrow_mut().movie = movie.clone();
        if let Some(m) = &movie {
            self.ui.movie_entry.set_text(&m.title);
            self.ui.year_entry.set_text(&m.year().map(|y| y.to_string()).unwrap_or_default());
            self.ui.tmdb_entry.set_text(&m.id.to_string());
        } else {
            self.ui.tmdb_entry.set_text("");
        }
        self.update_movie_card();
        self.ui.movie.set_chips(movie.as_ref());
        self.show_already_ripped(movie.as_ref().map(|m| m.id));
        let runtime_known = movie.as_ref().is_some_and(|m| m.runtime.is_some());
        let backdrop = movie.as_ref().and_then(|m| m.backdrop_path.clone().map(|b| (m.id, b)));
        self.load_poster(movie.and_then(|m| m.poster_path));
        self.load_backdrop(backdrop);
        self.regenerate_filename();
        if runtime_known {
            self.refine_main_title();
        }
    }

    fn update_movie_card(&self) {
        let s = self.state.borrow();
        match &s.movie {
            Some(m) => {
                self.ui.movie.heading.set_text(&m.title);
                let mut d = Vec::new();
                if let Some(y) = m.year() {
                    d.push(y.to_string());
                }
                if !m.original_title.is_empty() && m.original_title != m.title {
                    d.push(format!("titre original : {}", m.original_title));
                }
                d.push(format!("TMDB {}", m.id));
                self.ui.movie.details.set_text(&d.join(" · "));
                self.ui.movie.overview.set_text(if m.overview.is_empty() { "Aucun résumé disponible." } else { &m.overview });
                self.ui.movie.change_btn.set_label("Ce n'est pas le bon film ?");
            }
            None => {
                let typed = self.ui.movie_entry.text();
                self.ui.movie.heading.set_text(if typed.trim().is_empty() { "Film non reconnu" } else { typed.trim() });
                self.ui.movie.details.set_text("Non identifié sur TMDB");
                self.ui.movie.overview.set_text(
                    "Identifiez le film pour obtenir l'affiche, le titre officiel et l'identifiant TMDB, \
                     ou saisissez le titre et l'année dans « Fichier de sortie ».",
                );
                self.ui.movie.change_btn.set_label(if s.cfg.tmdb_api_key.trim().is_empty() {
                    "Activer la reconnaissance des films…"
                } else {
                    "Identifier le film…"
                });
                self.ui.movie.poster_stack.set_visible_child_name("placeholder");
            }
        }
    }

    /// Bandeau « déjà copié » si ce film figure dans l'historique.
    fn show_already_ripped(self: &Rc<Self>, tmdb_id: Option<u64>) {
        self.ui.movie.already.set_visible(false);
        let Some(id) = tmdb_id else { return };
        let me = self.clone();
        glib::spawn_future_local(async move {
            let found = runtime().spawn_blocking(move || history::find_by_tmdb(id)).await.ok().flatten();
            // Un autre film a pu être choisi entre-temps.
            if me.state.borrow().movie.as_ref().map(|m| m.id) != Some(id) {
                return;
            }
            if let Some(e) = found {
                let name = e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                me.ui.movie.already.set_text(&format!("⚠ Déjà copié le {} : {name} ({})", e.date_label(), e.size_label()));
                me.ui.movie.already.set_visible(true);
            }
        });
    }

    /// Image de fond de la fiche : petite image TMDB étirée (floue), en cache.
    fn load_backdrop(self: &Rc<Self>, backdrop: Option<(u64, String)>) {
        let gen = self.state.borrow().poster_gen;
        let Some((id, path)) = backdrop else {
            self.ui.theme.set_backdrop(None);
            return;
        };
        let file = gtk::glib::user_cache_dir().join(crate::config::APP_DIR).join("backdrops").join(format!("{id}.jpg"));
        let me = self.clone();
        glib::spawn_future_local(async move {
            let f = file.clone();
            let ok = runtime()
                .spawn(async move {
                    if f.exists() {
                        return true;
                    }
                    let Ok(bytes) = metadata::fetch_poster(backend::http(), &path, "w300").await else { return false };
                    if let Some(d) = f.parent() {
                        let _ = std::fs::create_dir_all(d);
                    }
                    std::fs::write(&f, bytes).is_ok()
                })
                .await
                .unwrap_or(false);
            if me.state.borrow().poster_gen == gen {
                me.ui.theme.set_backdrop(ok.then_some(file));
            }
        });
    }

    fn load_poster(self: &Rc<Self>, poster_path: Option<String>) {
        let gen = {
            let mut s = self.state.borrow_mut();
            s.poster_gen += 1;
            s.poster_gen
        };
        let Some(path) = poster_path else {
            self.ui.theme.reset();
            self.ui.movie.poster_stack.set_visible_child_name("placeholder");
            return;
        };
        self.ui.movie.poster_stack.set_visible_child_name("loading");
        let me = self.clone();
        glib::spawn_future_local(async move {
            let res = runtime().spawn(async move { metadata::fetch_poster(backend::http(), &path, "w342").await }).await;
            if me.state.borrow().poster_gen != gen {
                return; // Un autre film a été choisi entre-temps.
            }
            match res.ok().and_then(|r| r.ok()).and_then(movie_dialog::texture) {
                Some(t) => {
                    // L'affiche donne sa couleur à toute la fenêtre.
                    me.ui.theme.set_color(theme::dominant_color(&t));
                    me.ui.movie.poster.set_paintable(Some(&t));
                    me.ui.movie.poster_stack.set_visible_child_name("poster");
                }
                None => {
                    me.ui.theme.reset();
                    me.ui.movie.poster_stack.set_visible_child_name("placeholder");
                }
            }
        });
    }

    /// Assistant de clé TMDB ; une fois la clé enregistrée, le disque en
    /// cours est identifié aussitôt.
    fn open_tmdb_setup(self: &Rc<Self>) {
        let current = self.state.borrow().cfg.tmdb_api_key.clone();
        let me = self.clone();
        tmdb_setup::show(&self.ui.window, &current, move |key| {
            let (cfg, disc) = {
                let mut s = me.state.borrow_mut();
                s.cfg.tmdb_api_key = key.clone();
                (s.cfg.clone(), if s.movie.is_none() && s.rip_cancel.is_none() { s.disc.clone() } else { None })
            };
            if let Err(e) = cfg.save() {
                me.toast(&format!("Clé non enregistrée : {e}"));
                return;
            }
            me.toast("Clé TMDB enregistrée");
            me.update_movie_card();
            if let Some(d) = disc {
                me.ui.movie.heading.set_text("Recherche du film…");
                runtime().spawn(backend::lookup_metadata(me.tx.clone(), cfg, d));
            }
        });
    }

    fn open_movie_dialog(self: &Rc<Self>) {
        if self.state.borrow().cfg.tmdb_api_key.trim().is_empty() {
            self.open_tmdb_setup();
            return;
        }
        let cfg = self.state.borrow().cfg.clone();
        let query = self.ui.movie_entry.text().trim().to_string();
        let me = self.clone();
        movie_dialog::show(&self.ui.window, cfg, query, self.year(), move |m| {
            me.toast(&format!("Film choisi : {}", m.title));
            let id = m.id;
            me.apply_movie(Some(m));
            // La recherche ne donne pas la durée : fiche détaillée.
            me.load_movie_by_id(id);
        });
    }

    fn load_movie_by_id(self: &Rc<Self>, id: u64) {
        let cfg = self.state.borrow().cfg.clone();
        if cfg.tmdb_api_key.trim().is_empty() {
            self.open_tmdb_setup();
            return;
        }
        let me = self.clone();
        glib::spawn_future_local(async move {
            let res = runtime()
                .spawn(async move { metadata::tmdb_movie(backend::http(), cfg.tmdb_api_key.trim(), id, &cfg.language).await })
                .await;
            match res {
                Ok(Ok(m)) => me.apply_movie(Some(m)),
                Ok(Err(e)) => me.toast(&format!("TMDB : {e:#}")),
                Err(_) => {}
            }
        });
    }

    // ------------------------------------------------------------------
    // Éjection
    // ------------------------------------------------------------------
    fn eject(self: &Rc<Self>) {
        let device = {
            let mut s = self.state.borrow_mut();
            if s.rip_cancel.is_some() {
                return;
            }
            // MakeMKV tient le lecteur pendant l'analyse : on l'arrête d'abord.
            if let Some(c) = s.analysis_cancel.take() {
                c.cancel();
            }
            if let Some(e) = s.extraction.take() {
                discard(e);
            }
            s.device
                .clone()
                .or_else(|| s.loaded_drives.iter().next().cloned())
                .or_else(|| crate::drive::optical_drives().into_iter().next())
        };
        let Some(device) = device else {
            self.toast("Aucun lecteur optique");
            return;
        };
        self.ui.eject_btn.set_sensitive(false);
        let tx = self.tx.clone();
        let me = self.clone();
        glib::spawn_future_local(async move {
            let ok = runtime().spawn(async move { backend::eject(&tx, device).await }).await.unwrap_or(false);
            me.ui.eject_btn.set_sensitive(me.state.borrow().rip_cancel.is_none());
            if ok {
                me.toast("Disque éjecté");
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
                    // Le dernier dossier choisi devient la destination par défaut.
                    let cfg = {
                        let mut s = me.state.borrow_mut();
                        s.output_dir = path.clone();
                        s.cfg.output_dir = path;
                        s.cfg.clone()
                    };
                    if let Err(e) = cfg.save() {
                        me.toast(&format!("Dossier non mémorisé : {e}"));
                    }
                    // Nouvelle destination : on y prépare l'extraction si rien n'est en cours.
                    let idx = me.state.borrow().title_index;
                    me.prepare_extraction(idx);
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
        let cancel = CancellationToken::new();
        // Empêche la mise en veille pendant le rip (qui peut durer une heure).
        let cookie = self.app.inhibit(Some(&self.ui.window), gtk::ApplicationInhibitFlags::SUSPEND | gtk::ApplicationInhibitFlags::LOGOUT, Some("Rip en cours"));
        let title = {
            let mut s = self.state.borrow_mut();
            s.rip_cancel = Some(cancel);
            s.inhibit_cookie = Some(cookie);
            s.pending_output = Some(output.clone());
            s.title_index
        };
        self.set_ripping_ui(true);
        append_log(&self.ui.log_view, &format!("▶ Rip vers {}", output.display()));
        {
            let mut s = self.state.borrow_mut();
            s.rip_started = Some(std::time::Instant::now());
            s.last_output = None;
        }
        let poster = (self.ui.movie.poster_stack.visible_child_name().as_deref() == Some("poster")).then(|| self.ui.movie.poster.paintable()).flatten();
        let eject = self.state.borrow().cfg.eject_when_done;
        self.ui.rip_view.start(
            &self.display_title(),
            &output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            poster,
            self.ui.theme.color(),
            eject,
        );
        self.ui.disc_stack.set_visible_child_name("rip");
        self.ui.launcher.set_progress(Some(0.0));

        // Extraction déjà prête, en cours, ou à lancer ?
        let state = {
            let s = self.state.borrow();
            s.extraction.as_ref().filter(|e| e.covers(title, &s.selection)).map(|e| {
                if e.contents != s.selection {
                    append_log(&self.ui.log_view, "Pistes décochées depuis la préparation : un remux retirera les pistes en trop.");
                }
                (e.result.clone(), e.fraction, e.label.clone())
            })
        };
        match state {
            Some((Some(Ok(_)), _, _)) => {
                append_log(&self.ui.log_view, "Extraction déjà prête : passage direct à la finalisation.");
                self.launch_finish();
            }
            Some((None, fraction, label)) => {
                append_log(&self.ui.log_view, "Extraction déjà en cours : le rip la reprend.");
                self.show_rip_progress(Phase::Copy, &label, fraction * RIP_WEIGHT, None);
            }
            _ => {
                self.show_rip_progress(Phase::Copy, "Démarrage de MakeMKV…", 0.0, None);
                self.start_extraction(title);
            }
        }
    }

    fn show_rip_progress(&self, phase: Phase, label: &str, fraction: f64, estimate: Option<Estimate>) {
        self.ui.progress_bar.set_fraction(fraction.clamp(0.0, 1.0));
        self.ui.progress_bar.set_text(Some(&format!("{:.1} %", fraction * 100.0).replace('.', ",")));
        self.ui.progress_label.set_text(&match &estimate {
            Some(e) => format!("{label} · {}", crate::progress::describe(e)),
            None => label.to_string(),
        });
        self.ui.rip_view.update(phase, label, fraction, estimate);
        self.ui.launcher.set_progress(Some(fraction));
    }

    /// « Titre (Année) » tel qu'affiché et inscrit dans le fichier.
    fn display_title(&self) -> String {
        let movie = self.ui.movie_entry.text().trim().to_string();
        match self.year() {
            Some(y) => format!("{movie} ({y})"),
            None => movie,
        }
    }

    // ------------------------------------------------------------------
    // Extraction (éventuellement préparée en arrière-plan)
    // ------------------------------------------------------------------

    /// Après l'analyse ou un changement de titre : lance l'extraction du titre
    /// en arrière-plan si l'option est active.
    fn prepare_extraction(self: &Rc<Self>, title: usize) {
        let (enabled, busy, same) = {
            let s = self.state.borrow();
            (
                s.cfg.prepare_in_background && s.makemkv.is_some() && s.disc.is_some(),
                s.rip_cancel.is_some() || s.awaiting_metadata,
                s.extraction.as_ref().is_some_and(|e| e.covers(title, &s.selection)),
            )
        };
        if enabled && !busy && !same {
            self.start_extraction(title);
        }
    }

    fn start_extraction(self: &Rc<Self>, title: usize) {
        self.discard_extraction();
        // Jamais de copie sans la place nécessaire (un DVD fait 4 à 9 Go).
        let (dir, need, ripping) = {
            let s = self.state.borrow();
            let need = s.disc.as_ref().and_then(|d| d.titles.get(title)).map(|t| t.size_bytes()).unwrap_or(0);
            (s.output_dir.clone(), need, s.pending_output.is_some())
        };
        if let Err(e) = backend::check_space(&dir, need) {
            append_log(&self.ui.log_view, &format!("⚠ Extraction non lancée : {e}"));
            if ripping {
                self.on_rip_finished(Err(format!("Rip impossible : {e}. Choisissez un autre dossier de destination.")));
            } else {
                self.ui.progress_bar.set_fraction(0.0);
                self.ui.progress_bar.set_text(Some("Espace insuffisant"));
                self.ui.progress_label.set_text(&format!("Préparation impossible : {e}. Choisissez un autre dossier de destination."));
            }
            return;
        }
        let job = {
            let mut s = self.state.borrow_mut();
            let (Some(makemkv), Some(mkvmerge), Some(device)) = (s.makemkv.clone(), s.mkvmerge.clone(), s.device.clone()) else { return };
            let Some(title_info) = s.disc.as_ref().and_then(|d| d.titles.get(title)).cloned() else { return };
            let selection = s.selection.clone();
            let plan = rule::plan(&title_info, &selection);
            s.extract_seq += 1;
            let id = s.extract_seq;
            // Sur le même disque que la destination : pas de copie inter-disques.
            let tmp = s.output_dir.join(format!(".carbon-copy-{}-{id}", std::process::id()));
            let cancel = CancellationToken::new();
            s.extraction = Some(ExtractionState {
                id,
                title,
                cancel: cancel.clone(),
                tmp: tmp.clone(),
                contents: plan.contents.clone(),
                dropped: Selection::default(),
                result: None,
                fraction: 0.0,
                label: "Démarrage de MakeMKV…".into(),
            });
            let exact = plan.is_exact(&selection);
            let job = Extraction { id, makemkv, mkvmerge, device, title: title_info, selection, plan, tmp, cfg: s.cfg.clone() };
            (job, cancel, exact)
        };
        append_log(
            &self.ui.log_view,
            if job.2 {
                "MakeMKV n'extrait que les pistes cochées : pas de remux à la fin."
            } else {
                "Certaines pistes cochées ne se distinguent pas par une règle MakeMKV : un remux retirera les pistes en trop."
            },
        );
        if self.state.borrow().rip_cancel.is_none() {
            self.ui.progress_bar.set_fraction(0.0);
            self.ui.progress_bar.set_text(Some("Préparation…"));
            self.ui.progress_label.set_text(&format!("Préparation en arrière-plan du titre {title} : vous pouvez vérifier la fiche et les pistes."));
        }
        runtime().spawn(backend::extract(self.tx.clone(), job.0, job.1));
    }

    /// Abandonne l'extraction courante (annulation, suppression des fichiers).
    fn discard_extraction(&self) {
        if let Some(e) = self.state.borrow_mut().extraction.take() {
            discard(e);
        }
    }

    fn on_extract_progress(self: &Rc<Self>, id: u64, phase: Phase, label: String, fraction: f64, estimate: Option<Estimate>) {
        let ripping = {
            let mut s = self.state.borrow_mut();
            let Some(e) = s.extraction.as_mut().filter(|e| e.id == id) else { return };
            e.fraction = fraction;
            e.label = label.clone();
            s.pending_output.is_some()
        };
        if ripping {
            self.show_rip_progress(phase, &label, fraction * RIP_WEIGHT, estimate);
        } else {
            self.ui.progress_bar.set_fraction(fraction);
            self.ui.progress_bar.set_text(Some(&format!("Préparation {:.0} %", fraction * 100.0)));
            let eta = estimate.map(|e| format!(" · {}", crate::progress::describe(&e))).unwrap_or_default();
            self.ui.progress_label.set_text(&format!("Préparation en arrière-plan — {label}{eta}"));
        }
    }

    fn on_extracted(self: &Rc<Self>, id: u64, result: Result<Extracted, String>) {
        let ripping = {
            let mut s = self.state.borrow_mut();
            let Some(e) = s.extraction.as_mut().filter(|e| e.id == id) else { return };
            if let Ok(x) = &result {
                e.contents = x.contents.clone();
                e.dropped = x.dropped.clone();
            }
            e.result = Some(result.clone());
            s.pending_output.is_some()
        };
        match (result, ripping) {
            (Ok(_), true) => self.launch_finish(),
            (Ok(_), false) => {
                self.ui.progress_bar.set_fraction(1.0);
                self.ui.progress_bar.set_text(Some("Prêt"));
                self.ui.progress_label.set_text("Extraction prête : le rip ne prendra que quelques secondes.");
                self.toast("Disque copié : le rip sera quasi instantané");
            }
            (Err(e), true) => {
                self.state.borrow_mut().extraction = None;
                self.on_rip_finished(Err(e));
            }
            (Err(e), false) => {
                self.state.borrow_mut().extraction = None;
                self.ui.progress_bar.set_fraction(0.0);
                self.ui.progress_bar.set_text(Some("Prêt"));
                if !e.contains("annulée") {
                    append_log(&self.ui.log_view, &format!("⚠ Préparation interrompue : {e}"));
                    self.ui.progress_label.set_text("Préparation interrompue : le rip relancera l'extraction.");
                }
            }
        }
    }

    /// Extraction terminée et rip demandé : remux vers la destination.
    fn launch_finish(self: &Rc<Self>) {
        let job = {
            let mut s = self.state.borrow_mut();
            let Some(output) = s.pending_output.take() else { return };
            let Some(e) = s.extraction.take() else { return };
            let Some(Ok(extracted)) = e.result else { return };
            let (Some(mkvmerge), Some(device), Some(disc)) = (s.mkvmerge.clone(), s.device.clone(), s.disc.as_ref()) else { return };
            let Some(title) = disc.titles.get(e.title).cloned() else { return };
            let movie = self.ui.movie_entry.text().trim().to_string();
            FinishJob {
                mkvmerge,
                device,
                title,
                selection: s.selection.clone(),
                extracted,
                tmp: e.tmp,
                output,
                movie_title: match self.year() {
                    Some(y) => format!("{movie} ({y})"),
                    None => movie,
                },
                cfg: s.cfg.clone(),
            }
        };
        let cancel = self.state.borrow().rip_cancel.clone().unwrap_or_default();
        runtime().spawn(backend::finish(self.tx.clone(), job, cancel));
    }

    fn cancel_rip(self: &Rc<Self>) {
        let me = self.clone();
        glib::spawn_future_local(async move {
            if me.ask("Annuler le rip ?", "Le fichier partiel sera supprimé.", "Annuler le rip").await {
                let s = me.state.borrow();
                if let Some(c) = s.rip_cancel.as_ref() {
                    c.cancel();
                }
                // Extraction encore en cours : son annulation terminera le rip.
                if let Some(e) = s.extraction.as_ref().filter(|e| e.result.is_none() && s.pending_output.is_some()) {
                    e.cancel.cancel();
                }
            }
        });
    }

    fn set_ripping_ui(&self, ripping: bool) {
        self.ui.disc_content.set_sensitive(!ripping);
        self.ui.refresh_btn.set_sensitive(!ripping);
        self.ui.eject_btn.set_sensitive(!ripping);
        if ripping {
            self.ui.rip_btn.set_label("Annuler");
            self.ui.rip_btn.add_css_class("cancel");
        } else {
            self.ui.rip_btn.set_label("Lancer le Rip");
            self.ui.rip_btn.remove_css_class("cancel");
        }
    }

    fn on_rip_finished(self: &Rc<Self>, res: Result<PathBuf, String>) {
        {
            let mut s = self.state.borrow_mut();
            s.rip_cancel = None;
            s.pending_output = None;
            if let Some(c) = s.inhibit_cookie.take() {
                self.app.uninhibit(c);
            }
        }
        self.set_ripping_ui(false);
        self.ui.launcher.set_progress(None);
        let succeeded = res.is_ok();
        match res {
            Ok(path) => {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                append_log(&self.ui.log_view, &format!("✔ {}", path.display()));
                self.toast(&format!("Rip terminé : {file_name}"));
                // Écran de fin et historique (vidéothèque, alerte « déjà copié »).
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let (elapsed, film_secs, movie) = {
                    let mut s = self.state.borrow_mut();
                    s.last_output = Some(path.clone());
                    let secs = s.disc.as_ref().and_then(|d| d.titles.get(s.title_index)).map(|t| t.duration_secs()).unwrap_or(0);
                    (s.rip_started.take().map(|t| t.elapsed()).unwrap_or_default(), secs, s.movie.clone())
                };
                self.ui.rip_view.finish(&self.display_title(), &file_name, size, film_secs, elapsed);
                self.ui.action_zone.set_visible(false);
                let entry = history::Entry {
                    title: self.ui.movie_entry.text().trim().to_string(),
                    year: self.year(),
                    tmdb_id: metadata::parse_tmdb_id(&self.ui.tmdb_entry.text()),
                    poster_path: movie.and_then(|m| m.poster_path),
                    path: path.clone(),
                    size,
                    date: history::today(),
                };
                if let Err(e) = history::add(entry) {
                    append_log(&self.ui.log_view, &format!("⚠ Historique non enregistré : {e}"));
                }
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
        if succeeded {
            return; // L'écran « Copie terminée » reste affiché.
        }
        self.ui.disc_stack.set_visible_child_name("details");
        // Un disque a pu être éjecté / changé pendant le rip.
        let (loaded, current) = {
            let s = self.state.borrow();
            (s.loaded_drives.clone(), s.device.clone())
        };
        if let Some(dev) = current {
            if !loaded.contains(&dev) {
                self.show_nodisc();
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

/// Particularités d'une piste audio (sous-titre de la ligne).
fn audio_extras(s: &crate::makemkv::disc::Stream) -> String {
    let mut parts = Vec::new();
    if s.is_commentary() {
        parts.push("commentaire");
    }
    if s.is_visually_impaired() {
        parts.push("audiodescription");
    }
    if s.is_derived_core() {
        parts.push("cœur dérivé");
    }
    parts.join(" · ")
}

/// Pastilles colorées : format (DTS, AC3, TrueHD…) et canaux (5.1, 2.0…).
fn codec_chips(s: &crate::makemkv::disc::Stream) -> Vec<gtk::Label> {
    let id = s.codec_id();
    let codec = s.codec();
    let class = if id.contains("TRUEHD") {
        "truehd"
    } else if id.contains("DTS") {
        if codec.contains("MA") { "lossless" } else { "dts" }
    } else if id.contains("AC3") {
        "ac3"
    } else if id.contains("FLAC") || id.contains("PCM") {
        "lossless"
    } else {
        "other"
    };
    let label = match id {
        "S_VOBSUB" => "VobSub",
        "S_HDMV/PGS" => "PGS",
        "S_TEXT/UTF8" => "SRT",
        "S_TEXT/ASS" | "S_TEXT/SSA" => "ASS",
        _ if codec.is_empty() => id,
        _ => codec,
    };
    let mut chips = vec![gtk::Label::builder().label(label).valign(gtk::Align::Center).css_classes(["codec", class]).build()];
    let layout = match s.channels() {
        0 => String::new(),
        1 => "1.0".into(),
        2 => "2.0".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => format!("{n} canaux"),
    };
    if !layout.is_empty() {
        chips.push(gtk::Label::builder().label(&layout).valign(gtk::Align::Center).css_classes(["codec", "layout"]).build());
    }
    chips
}

/// Découpe le nom de fichier en blocs colorés : titre, année, identifiant
/// TMDB, caractéristiques techniques, langue.
fn fill_name_parts(flow: &gtk::FlowBox, name: &str) {
    flow.remove_all();
    let add = |text: &str, class: &str| {
        flow.append(&gtk::Label::builder().label(text).css_classes(["name-part", class]).build());
    };
    let stem = name.trim().trim_end_matches(".mkv");
    let (head, tech) = match stem.split_once(" - ") {
        Some((h, t)) => (h, Some(t)),
        None => (stem, None),
    };
    let year_re = regex::Regex::new(r"^(.*?)\s*\((\d{4})\)(.*)$").unwrap();
    let (title, year, rest) = match year_re.captures(head) {
        Some(c) => (c[1].to_string(), Some(c[2].to_string()), c[3].to_string()),
        None => (head.to_string(), None, String::new()),
    };
    if !title.is_empty() {
        add(&title, "title");
    }
    if let Some(y) = year {
        add(&y, "year");
    }
    for part in rest.split(']').map(|p| p.trim().trim_start_matches('[')).filter(|p| !p.is_empty()) {
        add(part, if part.starts_with("tmdbid-") { "tmdb" } else { "tech" });
    }
    if let Some(t) = tech {
        let words: Vec<&str> = t.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            let last = i + 1 == words.len();
            add(w, if last && w.chars().any(|c| c.is_alphabetic()) && !w.contains('.') { "lang" } else { "tech" });
        }
    }
    flow.append(&gtk::Label::builder().label(".mkv").css_classes(["name-part", "tech"]).build());
}
