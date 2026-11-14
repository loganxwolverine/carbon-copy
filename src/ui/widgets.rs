//! Mise en page de la fenêtre principale (aucune logique ici).
//!
//! Page « disque », pensée pour tenir sur un écran sans défilement :
//!
//! ```text
//! ┌─────────────────────────────────────────────┬──────────────────────┐
//! │ Film (affiche, titre, résumé)               │ Lecteur              │
//! ├────────────┬─────────────┬──────────────────┼──────────────────────┤
//! │ Disque     │ Audio       │ Sous-titres      │ Fichier de sortie    │
//! ├────────────┴─────────────┴──────────────────┴──────────────────────┤
//! │ Progression ········································· [Lancer]     │
//! └────────────────────────────────────────────────────────────────────┘
//! ```
//! Toutes les cases sont des [`Section`] identiques (même carte, même en-tête,
//! couleur propre) ; les quatre colonnes ont la même largeur et la même
//! hauteur. Une colonne ne défile que si son contenu dépasse. Le journal est un
//! panneau latéral escamotable.

use adw::prelude::*;
use gtk::{gio, glib};

use super::drive_panel::DrivePanel;
use super::movie_card::MovieCard;
use super::section::Section;
use super::theme::Theme;

pub const APP_NAME: &str = "Carbon Copy";

/// Références vers tous les widgets manipulés par le contrôleur.
pub struct Ui {
    pub theme: Theme,
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub window_title: adw::WindowTitle,
    pub refresh_btn: gtk::Button,
    pub eject_btn: gtk::Button,
    pub stack: gtk::Stack,

    pub setup_page: adw::StatusPage,
    pub error_page: adw::StatusPage,
    pub retry_btn: gtk::Button,
    pub analysis_bar: gtk::ProgressBar,

    /// Panneau « lecteur » de la page d'attente et de la page disque.
    pub nodisc_drive: DrivePanel,
    pub disc_drive: DrivePanel,
    pub movie: MovieCard,

    pub disc_content: gtk::Box,
    pub type_row: adw::ActionRow,
    pub name_row: adw::ActionRow,
    pub duration_row: adw::ActionRow,
    pub title_combo: adw::ComboRow,
    pub title_model: gtk::StringList,
    pub audio_list: gtk::ListBox,
    pub subs_list: gtk::ListBox,
    pub audio_section: Section,
    pub subs_section: Section,
    pub movie_entry: adw::EntryRow,
    pub year_entry: adw::EntryRow,
    pub tmdb_btn: gtk::Button,
    pub tmdb_entry: adw::EntryRow,
    pub tags_row: adw::ActionRow,
    pub filename_entry: adw::EntryRow,
    pub folder_row: adw::ActionRow,
    pub folder_btn: gtk::Button,
    pub progress_bar: gtk::ProgressBar,
    pub progress_label: gtk::Label,
    pub rip_btn: gtk::Button,

    pub log_view: gtk::TextView,
}

fn boxed_list() -> gtk::ListBox {
    gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).valign(gtk::Align::Start).css_classes(["boxed-list"]).build()
}

fn spinner() -> gtk::Widget {
    let s = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).halign(gtk::Align::Center).build();
    s.upcast()
}

pub fn build(app: &adw::Application) -> Ui {
    let theme = Theme::install();

    // --- Barre d'en-tête --------------------------------------------------
    let window_title = adw::WindowTitle::new(APP_NAME, "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&window_title));

    let refresh_btn = gtk::Button::builder().icon_name("view-refresh-symbolic").tooltip_text("Analyser à nouveau le disque").sensitive(false).build();
    let eject_btn = gtk::Button::builder().icon_name("media-eject-symbolic").tooltip_text("Éjecter le disque").sensitive(false).build();
    header.pack_start(&refresh_btn);
    header.pack_start(&eject_btn);

    let menu = gio::Menu::new();
    menu.append(Some("Préférences"), Some("app.preferences"));
    menu.append(Some("À propos de Carbon Copy"), Some("app.about"));
    let menu_btn = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).primary(true).build();
    let fullscreen_btn = gtk::Button::builder().icon_name("view-fullscreen-symbolic").tooltip_text("Plein écran (F11)").action_name("app.fullscreen").build();
    let log_btn = gtk::ToggleButton::builder().icon_name("utilities-terminal-symbolic").tooltip_text("Journal").build();
    header.pack_end(&menu_btn);
    header.pack_end(&fullscreen_btn);
    header.pack_end(&log_btn);

    // --- Pages simples ----------------------------------------------------
    let setup_page = adw::StatusPage::builder().icon_name("media-optical-symbolic").title("Préparation").description("Vérification de MakeMKV…").child(&spinner()).build();

    let retry_btn = gtk::Button::builder().label("Réessayer").halign(gtk::Align::Center).css_classes(["pill"]).build();
    let error_page = adw::StatusPage::builder().icon_name("dialog-error-symbolic").title("MakeMKV n'est pas disponible").child(&retry_btn).build();

    let nodisc_drive = DrivePanel::new();
    let nodisc_page = adw::StatusPage::builder()
        .icon_name("media-optical-symbolic")
        .title("Insérez un disque")
        .description("Un DVD, un Blu-ray ou un Blu-ray UHD sera analysé automatiquement.")
        .child(&adw::Clamp::builder().maximum_size(560).child(&nodisc_drive.root).build())
        .build();

    let analysis_bar = gtk::ProgressBar::builder().show_text(true).width_request(320).halign(gtk::Align::Center).build();
    let analysis_box = gtk::Box::new(gtk::Orientation::Vertical, 18);
    analysis_box.append(&spinner());
    analysis_box.append(&analysis_bar);
    let analysis_page = adw::StatusPage::builder()
        .icon_name("media-optical-bd-symbolic")
        .title("Analyse du disque")
        .description("MakeMKV lit la structure du disque, cela peut prendre une minute…")
        .child(&analysis_box)
        .build();

    // --- Bandeau : film + lecteur -----------------------------------------
    let movie = MovieCard::new();
    let disc_drive = DrivePanel::new();

    // --- Colonnes ------------------------------------------------------------
    let type_row = adw::ActionRow::builder().title("Type de média").css_classes(["property"]).build();
    let name_row = adw::ActionRow::builder().title("Nom du disque").css_classes(["property"]).build();
    let duration_row = adw::ActionRow::builder().title("Durée du titre").css_classes(["property"]).build();
    let title_model = gtk::StringList::new(&[]);
    let title_combo = adw::ComboRow::builder().title("Titre à extraire").model(&title_model).build();
    let disc_list = boxed_list();
    for r in [type_row.upcast_ref::<gtk::Widget>(), name_row.upcast_ref(), title_combo.upcast_ref(), duration_row.upcast_ref()] {
        disc_list.append(r);
    }
    let disc_section = Section::scrolled("Disque", "media-optical-symbolic", "sec-disc", &disc_list);

    let audio_list = boxed_list();
    let audio_section = Section::scrolled("Pistes audio", "audio-speakers-symbolic", "sec-audio", &audio_list);

    let subs_list = boxed_list();
    let subs_hint = gtk::Label::builder()
        .label("Par défaut : uniquement les sous-titres forcés de votre langue.")
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .build();
    let subs_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    subs_box.append(&subs_hint);
    subs_box.append(&subs_list);
    let subs_section = Section::scrolled("Sous-titres", "media-view-subtitles-symbolic", "sec-subs", &subs_box);

    let movie_entry = adw::EntryRow::builder().title("Titre du film").build();
    let tmdb_btn = gtk::Button::builder().icon_name("system-search-symbolic").tooltip_text("Identifier le film sur TMDB…").valign(gtk::Align::Center).css_classes(["flat"]).build();
    movie_entry.add_suffix(&tmdb_btn);
    let year_entry = adw::EntryRow::builder().title("Année").input_purpose(gtk::InputPurpose::Digits).build();
    let tmdb_entry = adw::EntryRow::builder().title("Identifiant TMDB (Entrée pour charger)").show_apply_button(true).build();
    let tags_row = adw::ActionRow::builder().title("Caractéristiques du média").css_classes(["property"]).subtitle_selectable(true).build();
    let filename_entry = adw::EntryRow::builder().title("Nom du fichier").build();
    let folder_btn = gtk::Button::builder().icon_name("folder-open-symbolic").tooltip_text("Choisir le dossier de destination").valign(gtk::Align::Center).css_classes(["flat"]).build();
    let folder_row = adw::ActionRow::builder().title("Dossier de destination").subtitle_selectable(true).build();
    folder_row.add_suffix(&folder_btn);
    folder_row.set_activatable_widget(Some(&folder_btn));
    let out_list = boxed_list();
    for r in [
        movie_entry.upcast_ref::<gtk::Widget>(),
        year_entry.upcast_ref(),
        tmdb_entry.upcast_ref(),
        tags_row.upcast_ref(),
        filename_entry.upcast_ref(),
        folder_row.upcast_ref(),
    ] {
        out_list.append(r);
    }
    let out_section = Section::scrolled("Fichier de sortie", "document-save-symbolic", "sec-output", &out_list);

    // Quatre colonnes de même largeur et de même hauteur. Le bandeau du haut
    // suit la même grille : Film = 3 colonnes, Lecteur = la 4e.
    let grid = gtk::Grid::builder().column_spacing(18).row_spacing(18).column_homogeneous(true).vexpand(true).build();
    grid.attach(&movie.root, 0, 0, 3, 1);
    grid.attach(&disc_drive.root, 3, 0, 1, 1);
    for (i, sec) in [&disc_section, &audio_section, &subs_section, &out_section].into_iter().enumerate() {
        sec.root.set_vexpand(true);
        grid.attach(&sec.root, i as i32, 1, 1, 1);
    }

    let disc_content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(18)
        .margin_bottom(12)
        .margin_start(18)
        .margin_end(18)
        .vexpand(true)
        .build();
    disc_content.append(&grid);

    // --- Zone d'action (toujours visible) -----------------------------------
    let progress_bar = gtk::ProgressBar::builder().show_text(true).valign(gtk::Align::Center).css_classes(["rip"]).build();
    let progress_label = gtk::Label::builder().xalign(0.0).css_classes(["dim-label", "caption"]).ellipsize(gtk::pango::EllipsizeMode::End).build();
    let progress_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).hexpand(true).valign(gtk::Align::Center).build();
    progress_box.append(&progress_bar);
    progress_box.append(&progress_label);
    let rip_btn = gtk::Button::builder().label("Lancer le Rip").valign(gtk::Align::Center).css_classes(["rip-button"]).build();
    let action_zone = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(18).css_classes(["action-zone"]).build();
    action_zone.append(&progress_box);
    action_zone.append(&rip_btn);

    let disc_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    disc_page.append(&disc_content);
    disc_page.append(&action_zone);

    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vexpand(true).hexpand(true).css_classes(["ambient"]).build();
    stack.add_named(&setup_page, Some("setup"));
    stack.add_named(&error_page, Some("error"));
    stack.add_named(&nodisc_page, Some("nodisc"));
    stack.add_named(&analysis_page, Some("analysis"));
    stack.add_named(&disc_page, Some("disc"));

    // --- Journal : panneau latéral escamotable ------------------------------
    let log_view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(6)
        .right_margin(6)
        .build();
    let log_scroll = gtk::ScrolledWindow::builder().vexpand(true).child(&log_view).build();
    let log_toolbar = adw::ToolbarView::new();
    log_toolbar.add_top_bar(&adw::HeaderBar::builder().show_end_title_buttons(false).title_widget(&adw::WindowTitle::new("Journal", "")).build());
    log_toolbar.set_content(Some(&log_scroll));
    let split = adw::OverlaySplitView::builder()
        .sidebar_position(gtk::PackType::End)
        .collapsed(true)
        .show_sidebar(false)
        .min_sidebar_width(420.0)
        .max_sidebar_width(640.0)
        .sidebar_width_fraction(0.38)
        .content(&stack)
        .sidebar(&log_toolbar)
        .build();
    log_btn.bind_property("active", &split, "show-sidebar").bidirectional().sync_create().build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&split));

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(APP_NAME)
        .default_width(1400)
        .default_height(900)
        .width_request(1100)
        .height_request(700)
        .content(&toasts)
        .build();

    // Icône du bouton selon l'état plein écran.
    let btn = fullscreen_btn.clone();
    window.connect_fullscreened_notify(move |w| {
        let (icon, tip) = if w.is_fullscreen() { ("view-restore-symbolic", "Quitter le plein écran (F11)") } else { ("view-fullscreen-symbolic", "Plein écran (F11)") };
        btn.set_icon_name(icon);
        btn.set_tooltip_text(Some(tip));
    });

    Ui {
        theme,
        window,
        toasts,
        window_title,
        refresh_btn,
        eject_btn,
        stack,
        setup_page,
        error_page,
        retry_btn,
        analysis_bar,
        nodisc_drive,
        disc_drive,
        movie,
        disc_content,
        type_row,
        name_row,
        duration_row,
        title_combo,
        title_model,
        audio_list,
        subs_list,
        audio_section,
        subs_section,
        movie_entry,
        year_entry,
        tmdb_btn,
        tmdb_entry,
        tags_row,
        filename_entry,
        folder_row,
        folder_btn,
        progress_bar,
        progress_label,
        rip_btn,
        log_view,
    }
}

/// Ajoute une ligne au journal (horodatée) et fait défiler vers le bas.
pub fn append_log(view: &gtk::TextView, line: &str) {
    const MAX_LINES: i32 = 5000;
    let buf = view.buffer();
    let time = glib::DateTime::now_local().ok().and_then(|d| d.format("%H:%M:%S").ok()).unwrap_or_default();
    let mut end = buf.end_iter();
    buf.insert(&mut end, &format!("{time}  {line}\n"));
    // On borne la taille du journal pour ne pas ralentir l'interface.
    if buf.line_count() > MAX_LINES {
        let mut start = buf.start_iter();
        if let Some(mut cut) = buf.iter_at_line(buf.line_count() - MAX_LINES) {
            buf.delete(&mut start, &mut cut);
        }
    }
    let mark = buf.create_mark(None, &buf.end_iter(), false);
    view.scroll_mark_onscreen(&mark);
    buf.delete_mark(&mark);
}
