//! Vidéothèque de l'écran d'accueil : affiches des derniers rips.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use super::section::Section;
use crate::backend::{http, runtime};
use crate::history::{self, Entry};
use crate::metadata;

const MAX_CARDS: usize = 60;

pub struct Library {
    pub section: Section,
    /// Carte « Collection » : nombre de films, taille totale, rips du mois.
    pub stats: Section,
    flow: gtk::FlowBox,
    stack: gtk::Stack,
    stat_count: gtk::Label,
    stat_size: gtk::Label,
    stat_month: gtk::Label,
}

fn stat(caption: &str) -> (gtk::Box, gtk::Label) {
    let value = gtk::Label::builder().label("—").css_classes(["stat-value"]).build();
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
    b.append(&value);
    b.append(&gtk::Label::builder().label(caption).css_classes(["dim-label", "caption"]).build());
    (b, value)
}

impl Library {
    pub fn new() -> Self {
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(2)
            .max_children_per_line(12)
            .row_spacing(12)
            .column_spacing(12)
            .valign(gtk::Align::Start)
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("folder-videos-symbolic")
            .title("Aucun film pour l'instant")
            .description("Vos rips apparaîtront ici, avec leur affiche.")
            .build();
        empty.add_css_class("compact");
        let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&flow).build();
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vexpand(true).build();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scroll, Some("list"));
        let section = Section::new("Vidéothèque", "folder-videos-symbolic", "sec-disc");
        section.content.append(&stack);
        section.root.set_hexpand(true);
        section.root.set_vexpand(true);

        let stats = Section::new("Collection", "emblem-favorite-symbolic", "sec-output");
        let (a, stat_count) = stat("films");
        let (b, stat_size) = stat("au total");
        let (c, stat_month) = stat("ce mois-ci");
        let row = gtk::Box::builder().homogeneous(true).spacing(12).build();
        row.append(&a);
        row.append(&b);
        row.append(&c);
        stats.content.append(&row);
        stats.root.set_valign(gtk::Align::Start);
        Self { section, stats, flow, stack, stat_count, stat_size, stat_month }
    }

    /// Affiche `entries` ; un clic sur une affiche appelle `on_open`.
    pub fn fill(&self, entries: &[Entry], tmdb_key: &str, on_open: Rc<dyn Fn(PathBuf)>) {
        self.flow.remove_all();
        self.stack.set_visible_child_name(if entries.is_empty() { "empty" } else { "list" });
        self.section.set_count(&if entries.is_empty() { String::new() } else { entries.len().to_string() });
        let total: u64 = entries.iter().map(|e| e.size).sum();
        let month = history::today().get(..7).unwrap_or("").to_string();
        self.stat_count.set_text(&entries.len().to_string());
        self.stat_size.set_text(&format!("{:.0} Go", total as f64 / 1e9));
        self.stat_month.set_text(&entries.iter().filter(|e| !month.is_empty() && e.date.starts_with(&month)).count().to_string());
        for e in entries.iter().take(MAX_CARDS) {
            self.flow.append(&card(e, tmdb_key, on_open.clone()));
        }
    }
}

fn card(e: &Entry, tmdb_key: &str, on_open: Rc<dyn Fn(PathBuf)>) -> gtk::Button {
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::ScaleDown)
        .can_shrink(true)
        .width_request(130)
        .height_request(195)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["library-poster"])
        .build();
    let v = adw::Clamp::builder().orientation(gtk::Orientation::Vertical).maximum_size(195).tightening_threshold(195).child(&picture).build();
    let h = adw::Clamp::builder().maximum_size(130).tightening_threshold(130).child(&v).build();
    let title = match e.year {
        Some(y) => format!("{} ({y})", e.title),
        None => e.title.clone(),
    };
    let name = gtk::Label::builder().label(&title).wrap(true).lines(2).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(16).justify(gtk::Justification::Center).css_classes(["heading"]).build();
    let meta = gtk::Label::builder().label(format!("{} · {}", e.date_label(), e.size_label())).css_classes(["dim-label", "caption"]).build();
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.append(&h);
    b.append(&name);
    b.append(&meta);
    let btn = gtk::Button::builder().child(&b).css_classes(["flat", "library-card"]).tooltip_text(e.path.display().to_string()).build();
    let path = e.path.clone();
    btn.connect_clicked(move |_| on_open(path.clone()));
    // Sans affiche : icône de film en attendant (ou à défaut).
    if let Some(d) = gdk::Display::default() {
        let icon = gtk::IconTheme::for_display(&d).lookup_icon("camera-video-symbolic", &[], 64, 1, gtk::TextDirection::None, gtk::IconLookupFlags::empty());
        picture.set_paintable(Some(&icon));
    }
    load_poster(&picture, e, tmdb_key);
    btn
}

fn set_poster(picture: &gtk::Picture, t: &gdk::Texture) {
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_paintable(Some(t));
}

/// Affiche depuis le cache, sinon depuis TMDB (puis mise en cache).
fn load_poster(picture: &gtk::Picture, e: &Entry, key: &str) {
    let Some(id) = e.tmdb_id else { return };
    let cache = history::poster_cache(id);
    if let Ok(t) = gdk::Texture::from_filename(&cache) {
        set_poster(picture, &t);
        return;
    }
    let (poster_path, key) = (e.poster_path.clone(), key.trim().to_string());
    if poster_path.is_none() && key.is_empty() {
        return;
    }
    let picture = picture.clone();
    glib::spawn_future_local(async move {
        let res = runtime()
            .spawn(async move {
                let path = match poster_path {
                    Some(p) => Some(p),
                    None => metadata::tmdb_movie(http(), &key, id, "fra").await.ok().and_then(|m| m.poster_path),
                };
                let bytes = metadata::fetch_poster(http(), &path?, "w185").await.ok()?;
                if let Some(d) = cache.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let _ = std::fs::write(&cache, &bytes);
                Some(bytes)
            })
            .await;
        if let Ok(Some(bytes)) = res {
            if let Ok(t) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)) {
                set_poster(&picture, &t);
            }
        }
    });
}
