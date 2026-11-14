//! Fenêtre « Identifier le film » : recherche TMDB manuelle.
//!
//! L'utilisateur saisit un titre (et éventuellement une année), un identifiant
//! TMDB ou un lien themoviedb.org ; les résultats s'affichent avec leur
//! affiche, un clic choisit le film.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::backend::{http, runtime};
use crate::config::Config;
use crate::metadata::{self, Movie};

/// Convertit des octets d'image en texture GTK.
pub fn texture(bytes: Vec<u8>) -> Option<gdk::Texture> {
    gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).ok()
}

/// Charge une affiche TMDB dans `picture` (silencieux en cas d'échec).
pub fn load_poster_into(picture: &gtk::Picture, poster_path: Option<String>, size: &'static str) {
    let Some(path) = poster_path else { return };
    let picture = picture.clone();
    glib::spawn_future_local(async move {
        let res = runtime().spawn(async move { metadata::fetch_poster(http(), &path, size).await }).await;
        if let Ok(Ok(bytes)) = res {
            if let Some(t) = texture(bytes) {
                picture.set_paintable(Some(&t));
            }
        }
    });
}

pub fn show(parent: &impl IsA<gtk::Widget>, cfg: Config, query: String, year: Option<u32>, on_pick: impl Fn(Movie) + 'static) {
    let on_pick = Rc::new(on_pick);
    let dialog = adw::Dialog::builder().title("Identifier le film").content_width(600).content_height(680).build();

    let search = gtk::SearchEntry::builder()
        .hexpand(true)
        .placeholder_text("Titre, identifiant TMDB ou lien themoviedb.org")
        .text(query.as_str())
        .build();
    let year_entry = gtk::Entry::builder()
        .placeholder_text("Année")
        .width_chars(6)
        .max_length(4)
        .input_purpose(gtk::InputPurpose::Digits)
        .text(year.map(|y| y.to_string()).unwrap_or_default().as_str())
        .build();
    let go = gtk::Button::builder().label("Rechercher").css_classes(["suggested-action"]).build();
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bar.append(&search);
    bar.append(&year_entry);
    bar.append(&go);

    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).valign(gtk::Align::Start).build();
    let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build();
    let empty = adw::StatusPage::builder()
        .icon_name("system-search-symbolic")
        .title("Rechercher un film")
        .description("Saisissez le titre (en français ou en version originale) puis Entrée.")
        .build();
    empty.add_css_class("compact");
    let loading = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
    let results = gtk::Stack::builder().vexpand(true).build();
    results.add_named(&empty, Some("empty"));
    results.add_named(&loading, Some("loading"));
    results.add_named(&scroll, Some("list"));

    let hint = gtk::Label::builder()
        .label("Film absent de TMDB ? Fermez cette fenêtre et saisissez simplement le titre et l'année dans « Fichier de sortie ».")
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&bar);
    content.append(&results);
    content.append(&hint);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    let run = {
        let (search, year_entry, list, results, empty, dialog) =
            (search.clone(), year_entry.clone(), list.clone(), results.clone(), empty.clone(), dialog.clone());
        let on_pick = on_pick.clone();
        Rc::new(move || {
            let key = cfg.tmdb_api_key.trim().to_string();
            if key.is_empty() {
                empty.set_title("Clé TMDB manquante");
                empty.set_description(Some("Ajoutez une clé API TMDB dans les Préférences pour rechercher les films."));
                results.set_visible_child_name("empty");
                return;
            }
            let q = search.text().trim().to_string();
            if q.is_empty() {
                return;
            }
            let y: Option<u32> = year_entry.text().trim().parse().ok();
            let lang = cfg.language.clone();
            results.set_visible_child_name("loading");
            let (list, results, empty, dialog, on_pick) = (list.clone(), results.clone(), empty.clone(), dialog.clone(), on_pick.clone());
            glib::spawn_future_local(async move {
                let res = runtime()
                    .spawn(async move {
                        match metadata::parse_tmdb_id(&q) {
                            Some(id) => metadata::tmdb_movie(http(), &key, id, &lang).await.map(|m| vec![m]),
                            None => metadata::tmdb_search(http(), &key, &q, y, &lang).await,
                        }
                    })
                    .await;
                let movies = match res {
                    Ok(Ok(m)) => m,
                    Ok(Err(e)) => {
                        empty.set_title("Recherche impossible");
                        empty.set_description(Some(&format!("{e:#}")));
                        results.set_visible_child_name("empty");
                        return;
                    }
                    Err(_) => return,
                };
                list.remove_all();
                if movies.is_empty() {
                    empty.set_title("Aucun résultat");
                    empty.set_description(Some("Essayez le titre original, sans année, ou collez le lien themoviedb.org du film."));
                    results.set_visible_child_name("empty");
                    return;
                }
                for m in movies {
                    list.append(&result_row(&m, &dialog, on_pick.clone()));
                }
                results.set_visible_child_name("list");
            });
        })
    };

    let r = run.clone();
    search.connect_activate(move |_| r());
    let r = run.clone();
    year_entry.connect_activate(move |_| r());
    let r = run.clone();
    go.connect_clicked(move |_| r());

    dialog.present(Some(parent));
    search.grab_focus();
    if !query.trim().is_empty() {
        run();
    }
}

fn result_row(m: &Movie, dialog: &adw::Dialog, on_pick: Rc<dyn Fn(Movie)>) -> adw::ActionRow {
    let title = match m.year() {
        Some(y) => format!("{} ({y})", m.title),
        None => m.title.clone(),
    };
    let mut sub = Vec::new();
    if !m.original_title.is_empty() && m.original_title != m.title {
        sub.push(m.original_title.clone());
    }
    sub.push(format!("TMDB {}", m.id));
    if !m.overview.is_empty() {
        sub.push(m.overview.clone());
    }
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&title).as_str())
        .subtitle(glib::markup_escape_text(&sub.join(" · ")).as_str())
        .subtitle_lines(2)
        .activatable(true)
        .build();

    let pic = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).build();
    let frame = gtk::Stack::builder()
        .width_request(46)
        .height_request(69)
        .valign(gtk::Align::Center)
        .overflow(gtk::Overflow::Hidden)
        .margin_top(6)
        .margin_bottom(6)
        .css_classes(["card"])
        .build();
    frame.add_child(&gtk::Image::builder().icon_name("video-x-generic-symbolic").css_classes(["dim-label"]).build());
    frame.add_named(&pic, Some("pic"));
    if m.poster_path.is_some() {
        frame.set_visible_child_name("pic");
        load_poster_into(&pic, m.poster_path.clone(), "w92");
    }
    row.add_prefix(&frame);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    let movie = m.clone();
    let dialog = dialog.clone();
    row.connect_activated(move |_| {
        on_pick(movie.clone());
        dialog.close();
    });
    row
}
