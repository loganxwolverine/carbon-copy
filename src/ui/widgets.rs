//! Construction des widgets de la fenêtre principale (aucune logique ici).

use adw::prelude::*;
use gtk::{gio, glib};

pub const APP_NAME: &str = "Carbon Copy";

/// Références vers tous les widgets manipulés par le contrôleur.
pub struct Ui {
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub window_title: adw::WindowTitle,
    pub refresh_btn: gtk::Button,
    pub stack: gtk::Stack,

    // Page « préparation »
    pub setup_page: adw::StatusPage,
    // Page « erreur »
    pub error_page: adw::StatusPage,
    pub retry_btn: gtk::Button,
    // Page « analyse »
    pub analysis_bar: gtk::ProgressBar,

    // Page « disque »
    pub disc_content: gtk::Box,
    pub type_row: adw::ActionRow,
    pub name_row: adw::ActionRow,
    pub duration_row: adw::ActionRow,
    pub title_combo: adw::ComboRow,
    pub title_model: gtk::StringList,
    pub audio_list: gtk::ListBox,
    pub subs_list: gtk::ListBox,
    pub movie_entry: adw::EntryRow,
    pub year_entry: adw::EntryRow,
    pub tmdb_btn: gtk::Button,
    pub filename_entry: adw::EntryRow,
    pub folder_row: adw::ActionRow,
    pub folder_btn: gtk::Button,
    pub progress_bar: gtk::ProgressBar,
    pub progress_label: gtk::Label,
    pub rip_btn: gtk::Button,

    // Journal
    pub log_view: gtk::TextView,
}

fn boxed_list() -> gtk::ListBox {
    gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build()
}

fn spinner() -> gtk::Widget {
    let s = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).halign(gtk::Align::Center).build();
    s.upcast()
}

pub fn build(app: &adw::Application) -> Ui {
    // --- Barre d'en-tête --------------------------------------------------
    let window_title = adw::WindowTitle::new(APP_NAME, "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&window_title));

    let refresh_btn = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Analyser à nouveau le disque")
        .sensitive(false)
        .build();
    header.pack_start(&refresh_btn);

    let menu = gio::Menu::new();
    menu.append(Some("Préférences"), Some("app.preferences"));
    menu.append(Some("À propos de Carbon Copy"), Some("app.about"));
    let menu_btn = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).primary(true).build();
    header.pack_end(&menu_btn);

    // --- Pages ------------------------------------------------------------
    let setup_page = adw::StatusPage::builder()
        .icon_name("media-optical-symbolic")
        .title("Préparation")
        .description("Vérification de MakeMKV…")
        .child(&spinner())
        .build();

    let retry_btn = gtk::Button::builder().label("Réessayer").halign(gtk::Align::Center).css_classes(["pill"]).build();
    let error_page = adw::StatusPage::builder()
        .icon_name("dialog-error-symbolic")
        .title("MakeMKV n'est pas disponible")
        .child(&retry_btn)
        .build();

    let nodisc_page = adw::StatusPage::builder()
        .icon_name("media-optical-symbolic")
        .title("Insérez un disque")
        .description("Un DVD, un Blu-ray ou un Blu-ray UHD sera analysé automatiquement.")
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

    // --- Page disque ------------------------------------------------------
    let type_row = adw::ActionRow::builder().title("Type de média").css_classes(["property"]).build();
    let name_row = adw::ActionRow::builder().title("Nom du disque").css_classes(["property"]).build();
    let duration_row = adw::ActionRow::builder().title("Durée du titre").css_classes(["property"]).build();
    let title_model = gtk::StringList::new(&[]);
    let title_combo = adw::ComboRow::builder().title("Titre à extraire").model(&title_model).build();

    let disc_group = adw::PreferencesGroup::builder().title("Disque").build();
    disc_group.add(&type_row);
    disc_group.add(&name_row);
    disc_group.add(&title_combo);
    disc_group.add(&duration_row);

    let audio_list = boxed_list();
    let audio_group = adw::PreferencesGroup::builder().title("Pistes audio").build();
    audio_group.add(&audio_list);

    let subs_list = boxed_list();
    let subs_group = adw::PreferencesGroup::builder()
        .title("Sous-titres")
        .description("Par défaut, seuls les sous-titres forcés de votre langue sont sélectionnés.")
        .build();
    subs_group.add(&subs_list);

    let movie_entry = adw::EntryRow::builder().title("Titre du film").build();
    let tmdb_btn = gtk::Button::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text("Rechercher sur TMDB")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    movie_entry.add_suffix(&tmdb_btn);
    let year_entry = adw::EntryRow::builder().title("Année").input_purpose(gtk::InputPurpose::Digits).build();
    let filename_entry = adw::EntryRow::builder().title("Nom du fichier").build();
    let folder_btn = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Choisir le dossier de destination")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let folder_row = adw::ActionRow::builder().title("Dossier de destination").subtitle_selectable(true).build();
    folder_row.add_suffix(&folder_btn);
    folder_row.set_activatable_widget(Some(&folder_btn));

    let out_group = adw::PreferencesGroup::builder().title("Fichier de sortie").build();
    out_group.add(&movie_entry);
    out_group.add(&year_entry);
    out_group.add(&filename_entry);
    out_group.add(&folder_row);

    let progress_bar = gtk::ProgressBar::builder().show_text(true).build();
    let progress_label = gtk::Label::builder().xalign(0.0).css_classes(["dim-label", "caption"]).ellipsize(gtk::pango::EllipsizeMode::End).build();
    let rip_btn = gtk::Button::builder()
        .label("Lancer le Rip")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .build();

    let disc_content = gtk::Box::new(gtk::Orientation::Vertical, 24);
    for g in [&disc_group, &audio_group, &subs_group, &out_group] {
        disc_content.append(g);
    }

    let disc_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    disc_box.append(&disc_content);
    let progress_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    progress_box.append(&progress_bar);
    progress_box.append(&progress_label);
    disc_box.append(&progress_box);
    disc_box.append(&rip_btn);

    let clamp = adw::Clamp::builder().maximum_size(760).child(&disc_box).build();
    let disc_page = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).build();

    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vexpand(true).build();
    stack.add_named(&setup_page, Some("setup"));
    stack.add_named(&error_page, Some("error"));
    stack.add_named(&nodisc_page, Some("nodisc"));
    stack.add_named(&analysis_page, Some("analysis"));
    stack.add_named(&disc_page, Some("disc"));

    // --- Journal ----------------------------------------------------------
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
    let log_scroll = gtk::ScrolledWindow::builder().min_content_height(160).child(&log_view).build();
    let log_frame = gtk::Frame::builder().child(&log_scroll).margin_start(12).margin_end(12).margin_bottom(12).build();
    let log_expander = gtk::Expander::builder().label("Journal").child(&log_frame).margin_start(12).margin_end(12).margin_bottom(6).build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&stack);
    root.append(&log_expander);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&root));

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(APP_NAME)
        .default_width(780)
        .default_height(900)
        .content(&toasts)
        .build();

    Ui {
        window,
        toasts,
        window_title,
        refresh_btn,
        stack,
        setup_page,
        error_page,
        retry_btn,
        analysis_bar,
        disc_content,
        type_row,
        name_row,
        duration_row,
        title_combo,
        title_model,
        audio_list,
        subs_list,
        movie_entry,
        year_entry,
        tmdb_btn,
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
