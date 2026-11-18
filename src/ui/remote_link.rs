//! Lien entre le contrôleur et la télécommande web : état exposé à la page
//! et exécution des actions reçues (sur le thread GTK, comme les boutons).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use tokio_util::sync::CancellationToken;

use super::{audio_extras, codec_text, App};
use crate::events::{Estimate, Phase};
use crate::makemkv::disc::StreamKind;
use crate::makemkv::robot::format_duration;
use crate::remote::{self, Bar, Command, DiscView, MovieView, Notice, ProgressView, RipResult, Snapshot, TitleView, TrackView};

/// Lignes du journal envoyées à la page.
const LOG_LINES: i32 = 40;

#[derive(Default)]
pub(super) struct RemoteLink {
    /// Serveur actif : port et jeton d'arrêt.
    server: RefCell<Option<(u16, CancellationToken)>>,
    /// Dernière progression du rip (étape, vitesse, temps restant).
    progress: RefCell<Option<ProgressView>>,
    /// Issue du dernier rip.
    result: RefCell<Option<RipResult>>,
    notice: RefCell<Option<Notice>>,
    notice_seq: Cell<u64>,
}

fn phase_id(p: Phase) -> &'static str {
    match p {
        Phase::Copy => "copy",
        Phase::Verify => "verify",
        Phase::Finalize => "finalize",
        Phase::Eject => "eject",
    }
}

impl App {
    /// Démarre, arrête ou redémarre le serveur selon les préférences.
    pub(super) fn apply_remote_config(&self) {
        let (enabled, port) = {
            let s = self.state.borrow();
            (s.cfg.remote_enabled, s.cfg.remote_port)
        };
        let mut server = self.remote.server.borrow_mut();
        if enabled && server.as_ref().is_some_and(|(p, _)| *p == port) {
            return;
        }
        if let Some((_, cancel)) = server.take() {
            cancel.cancel();
        }
        if enabled {
            *server = Some((port, remote::start(port, self.tx.clone())));
        }
    }

    pub(super) fn stop_remote(&self) {
        if let Some((_, cancel)) = self.remote.server.borrow_mut().take() {
            cancel.cancel();
        }
    }

    pub(super) fn remote_notice(&self, text: &str) {
        let id = self.remote.notice_seq.get() + 1;
        self.remote.notice_seq.set(id);
        *self.remote.notice.borrow_mut() = Some(Notice { id, text: text.to_string() });
    }

    pub(super) fn remote_progress(&self, phase: Phase, label: &str, fraction: f64, estimate: Option<&Estimate>) {
        *self.remote.progress.borrow_mut() = Some(ProgressView {
            phase: phase_id(phase).into(),
            label: label.to_string(),
            fraction,
            speed: estimate.and_then(|e| e.bytes_per_sec).map(|b| format!("{} Mo/s", format!("{:.1}", b / 1e6).replace('.', ","))),
            remaining: estimate.map(|e| crate::progress::short_duration(e.remaining)),
        });
    }

    pub(super) fn remote_rip_started(&self) {
        *self.remote.progress.borrow_mut() = None;
        *self.remote.result.borrow_mut() = None;
    }

    pub(super) fn remote_rip_finished(&self, result: Option<RipResult>) {
        *self.remote.progress.borrow_mut() = None;
        *self.remote.result.borrow_mut() = result;
    }

    /// Action demandée par la télécommande. Pas de boîte de dialogue : un cas
    /// qui en demanderait une (fichier existant, aucune piste audio) est refusé
    /// avec une notification, visible sur la page.
    pub(super) fn on_remote(self: &Rc<Self>, cmd: Command) {
        self.log(&format!("Télécommande web : {cmd:?}"));
        let screen = self.screen();
        let ripping = self.state.borrow().rip_cancel.is_some();
        match cmd {
            Command::Rip => {
                if screen != "disc" || ripping || self.state.borrow().disc.is_none() || !self.ui.rip_btn.is_sensitive() {
                    self.toast("Aucun disque prêt à ripper");
                    return;
                }
                let Some(output) = self.output_path() else {
                    self.toast("Nom de fichier vide : rip impossible");
                    return;
                };
                if self.state.borrow().selection.audio.is_empty() {
                    self.toast("Aucune piste audio sélectionnée : rip refusé");
                    return;
                }
                if output.exists() {
                    self.toast("Le fichier existe déjà : rip refusé (à remplacer depuis l'application)");
                    return;
                }
                self.start_rip(output);
            }
            Command::Cancel => {
                if ripping {
                    self.abort_rip();
                }
            }
            Command::Eject => {
                if ripping {
                    self.toast("Éjection impossible pendant un rip");
                } else {
                    self.eject();
                }
            }
            Command::Analyze => {
                if !ripping {
                    self.reanalyze();
                }
            }
            Command::NextDisc => {
                if screen == "done" {
                    self.next_disc();
                }
            }
            Command::SelectTitle(i) => {
                let count = self.state.borrow().disc.as_ref().map(|d| d.titles.len()).unwrap_or(0);
                if screen == "disc" && !ripping && i < count {
                    self.ui.title_combo.set_selected(i as u32);
                }
            }
            // Fiche, pistes et dossier : modifiables tant que le rip n'est pas
            // lancé, comme dans la fenêtre.
            _ if screen != "disc" || ripping => self.toast("Modification impossible pendant un rip"),
            Command::Track { subtitle, index, on } => self.remote_track(subtitle, index, on),
            Command::Movie(id) => {
                if self.state.borrow().cfg.tmdb_api_key.trim().is_empty() {
                    self.toast("Clé TMDB absente : à ajouter dans les préférences de l'application");
                } else {
                    self.load_movie_by_id(id);
                }
            }
            Command::ManualMovie { title, year } => {
                self.apply_movie(None);
                self.ui.movie_entry.set_text(&title);
                self.ui.year_entry.set_text(&year.map(|y| y.to_string()).unwrap_or_default());
                self.update_movie_card();
                self.toast(&format!("Film saisi : {title}"));
            }
            Command::Folder(path) => {
                if path.is_dir() {
                    self.toast(&format!("Destination : {}", path.display()));
                    self.set_output_dir(path);
                } else {
                    self.toast("Dossier introuvable");
                }
            }
        }
    }

    /// Coche ou décoche une piste, puis met à jour la fenêtre (cases, nom du
    /// fichier) et la préparation, comme un clic sur la case.
    fn remote_track(self: &Rc<Self>, subtitle: bool, index: usize, on: bool) {
        let kind = if subtitle { StreamKind::Subtitle } else { StreamKind::Audio };
        let title = {
            let mut s = self.state.borrow_mut();
            let Some(t) = s.disc.as_ref().and_then(|d| d.titles.get(s.title_index)).cloned() else { return };
            if !t.streams_of(kind).any(|st| st.index == index) {
                return;
            }
            let set = if subtitle { &mut s.selection.subtitles } else { &mut s.selection.audio };
            let changed = if on { set.insert(index) } else { set.remove(&index) };
            if !changed {
                return;
            }
            t
        };
        self.rebuild_tracks(&title);
        self.regenerate_filename();
        self.on_selection_changed();
    }

    fn log(&self, line: &str) {
        super::append_log(&self.ui.log_view, line);
    }

    /// Écran affiché : pages principales, plus `rip` / `done` pour l'écran de rip.
    fn screen(&self) -> String {
        let page = self.ui.stack.visible_child_name().map(|s| s.to_string()).unwrap_or_default();
        if page == "disc" && self.ui.disc_stack.visible_child_name().as_deref() == Some("rip") {
            let done = self.ui.rip_view.root.visible_child_name().as_deref() == Some("done");
            return if done { "done" } else { "rip" }.into();
        }
        page
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        let ui = &self.ui;
        let screen = self.screen();
        let s = self.state.borrow();
        let ripping = s.rip_cancel.is_some();

        let error = (screen == "error").then(|| {
            let title = ui.error_page.title().to_string();
            let desc = ui.error_page.description().map(|d| d.to_string()).unwrap_or_default();
            if desc.is_empty() { title } else { format!("{title} : {desc}") }
        });
        let analysis = (screen == "analysis").then(|| Bar {
            label: ui.analysis_bar.text().map(|t| t.to_string()).unwrap_or_default(),
            fraction: Some(ui.analysis_bar.fraction()),
        });

        let runtime = s.movie.as_ref().and_then(|m| m.runtime).filter(|m| *m > 0).map(|m| m as u64 * 60);
        let disc = s.disc.as_ref().map(|d| {
            let main = d.main_title_for(runtime).or_else(|| d.main_title());
            let titles = d
                .titles
                .iter()
                .map(|t| TitleView {
                    index: t.index,
                    duration: format_duration(t.duration_secs()),
                    chapters: t.chapters(),
                    size: t.size_label().to_string(),
                    main: Some(t.index) == main,
                })
                .collect();
            let (mut audio, mut subtitles) = (Vec::new(), Vec::new());
            if let Some(t) = d.titles.get(s.title_index) {
                for st in t.streams_of(StreamKind::Audio).filter(|st| s.selection.audio.contains(&st.index)) {
                    let (codec, _, layout) = codec_text(st);
                    audio.push(format!("{} · {codec} {layout}", st.lang_name()).trim_end().to_string());
                }
                for st in t.streams_of(StreamKind::Subtitle).filter(|st| s.selection.subtitles.contains(&st.index)) {
                    subtitles.push(if st.is_forced() { format!("{} (forcés)", st.lang_name()) } else { st.lang_name() });
                }
            }
            let tracks = d
                .titles
                .get(s.title_index)
                .map(|t| {
                    let audio = t.streams_of(StreamKind::Audio).map(|st| (false, st));
                    let subs = t.streams_of(StreamKind::Subtitle).map(|st| (true, st));
                    audio
                        .chain(subs)
                        .map(|(subtitle, st)| {
                            let (codec, _, layout) = codec_text(st);
                            let set = if subtitle { &s.selection.subtitles } else { &s.selection.audio };
                            TrackView {
                                subtitle,
                                index: st.index,
                                lang: st.lang_name(),
                                codec,
                                layout,
                                extras: if subtitle { String::new() } else { audio_extras(st) },
                                forced: subtitle && st.is_forced(),
                                selected: set.contains(&st.index),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            DiscView {
                tracks,
                kind: d.media_type().label().to_string(),
                name: d.name(),
                title_index: s.title_index,
                titles,
                audio,
                subtitles,
            }
        });

        let movie = s.disc.as_ref().map(|_| MovieView {
            title: ui.movie_entry.text().trim().to_string(),
            year: self.year(),
            overview: s.movie.as_ref().map(|m| m.overview.clone()).unwrap_or_default(),
            poster: s.movie.as_ref().and_then(|m| m.poster_path.as_deref()).map(remote::poster_url),
            tmdb_id: crate::metadata::parse_tmdb_id(&ui.tmdb_entry.text()),
            identified: s.movie.is_some(),
            original_title: s.movie.as_ref().map(|m| m.original_title.clone()).filter(|o| *o != ui.movie_entry.text().trim()).unwrap_or_default(),
        });

        // Hors rip : préparation de l'extraction en arrière-plan.
        let progress = if ripping {
            self.remote.progress.borrow().clone()
        } else if screen == "disc" && s.extraction.is_some() {
            Some(ProgressView {
                phase: "prepare".into(),
                label: ui.progress_label.text().to_string(),
                fraction: ui.progress_bar.fraction(),
                speed: None,
                remaining: None,
            })
        } else {
            None
        };

        Snapshot {
            can_rip: screen == "disc" && !ripping && s.disc.is_some() && ui.rip_btn.is_sensitive(),
            screen,
            status: ui.window_title.subtitle().to_string(),
            error,
            analysis,
            disc,
            movie,
            filename: ui.filename_entry.text().trim().to_string(),
            output_dir: s.output_dir.display().to_string(),
            ripping,
            progress,
            result: self.remote.result.borrow().clone(),
            notice: self.remote.notice.borrow().clone(),
            log: last_lines(&ui.log_view, LOG_LINES),
            version: env!("CARGO_PKG_VERSION").into(),
            tmdb_ready: !s.cfg.tmdb_api_key.trim().is_empty(),
        }
    }
}

fn last_lines(view: &gtk::TextView, n: i32) -> Vec<String> {
    let buf = view.buffer();
    let start_line = (buf.line_count() - n - 1).max(0);
    let Some(start) = buf.iter_at_line(start_line) else { return Vec::new() };
    buf.text(&start, &buf.end_iter(), false).lines().filter(|l| !l.is_empty()).map(String::from).collect()
}
