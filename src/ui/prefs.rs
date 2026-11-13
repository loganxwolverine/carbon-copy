//! Fenêtre de préférences (adw::PreferencesDialog).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::backend::{http, runtime};
use crate::beta_key::{self, KeyUpdate};
use crate::config::Config;
use crate::events::{emit, log, AppEvent, Tx};

/// Langues proposées : (code ISO 639-2, libellé).
const LANGUAGES: &[(&str, &str)] = &[
    ("fra", "Français"),
    ("eng", "Anglais"),
    ("deu", "Allemand"),
    ("spa", "Espagnol"),
    ("ita", "Italien"),
    ("por", "Portugais"),
    ("nld", "Néerlandais"),
    ("jpn", "Japonais"),
];

pub fn show(parent: &adw::ApplicationWindow, cfg: Config, tx: Tx, on_close: impl Fn(Config) + 'static) {
    let cfg = Rc::new(RefCell::new(cfg));
    let dialog = adw::PreferencesDialog::new();
    let page = adw::PreferencesPage::builder().title("Général").icon_name("preferences-system-symbolic").build();

    // --- Langue ------------------------------------------------------------
    let lang_group = adw::PreferencesGroup::builder()
        .title("Langue")
        .description("Langue préférée pour l'audio et les sous-titres forcés.")
        .build();
    let labels: Vec<&str> = LANGUAGES.iter().map(|(_, l)| *l).collect();
    let lang_row = adw::ComboRow::builder().title("Langue préférée").model(&gtk::StringList::new(&labels)).build();
    let current = crate::selection::normalize_lang(&cfg.borrow().language);
    if let Some(i) = LANGUAGES.iter().position(|(c, _)| *c == current) {
        lang_row.set_selected(i as u32);
    }
    let vo_row = adw::SwitchRow::builder()
        .title("Conserver la version originale")
        .subtitle("Sélectionner aussi la première piste audio dans une autre langue")
        .active(cfg.borrow().keep_original_audio)
        .build();
    lang_group.add(&lang_row);
    lang_group.add(&vo_row);

    // --- Métadonnées -------------------------------------------------------
    let meta_group = adw::PreferencesGroup::builder()
        .title("Métadonnées")
        .description("Clé API TMDB (v3) ou jeton de lecture (v4), gratuit sur themoviedb.org.")
        .build();
    let tmdb_row = adw::PasswordEntryRow::builder().title("Clé API TMDB").build();
    tmdb_row.set_text(&cfg.borrow().tmdb_api_key);
    meta_group.add(&tmdb_row);

    // --- Extraction --------------------------------------------------------
    let rip_group = adw::PreferencesGroup::builder().title("Extraction").build();
    let minlen_row = adw::SpinRow::with_range(0.0, 3600.0, 30.0);
    minlen_row.set_title("Durée minimale des titres (s)");
    minlen_row.set_value(cfg.borrow().min_length_secs as f64);
    let eject_row = adw::SwitchRow::builder().title("Éjecter le disque à la fin").active(cfg.borrow().eject_when_done).build();
    let folder_btn = gtk::Button::builder().icon_name("folder-open-symbolic").valign(gtk::Align::Center).css_classes(["flat"]).build();
    let folder_row = adw::ActionRow::builder()
        .title("Dossier par défaut")
        .subtitle(cfg.borrow().output_dir.display().to_string())
        .build();
    folder_row.add_suffix(&folder_btn);
    folder_row.set_activatable_widget(Some(&folder_btn));
    rip_group.add(&minlen_row);
    rip_group.add(&eject_row);
    rip_group.add(&folder_row);

    {
        let cfg = cfg.clone();
        let folder_row = folder_row.clone();
        let parent = parent.clone();
        folder_btn.connect_clicked(move |_| {
            let d = gtk::FileDialog::builder().title("Dossier par défaut").build();
            d.set_initial_folder(Some(&gio::File::for_path(&cfg.borrow().output_dir)));
            let cfg = cfg.clone();
            let folder_row = folder_row.clone();
            let parent = parent.clone();
            glib::spawn_future_local(async move {
                if let Ok(f) = d.select_folder_future(Some(&parent)).await {
                    if let Some(p) = f.path() {
                        folder_row.set_subtitle(&p.display().to_string());
                        cfg.borrow_mut().output_dir = p;
                    }
                }
            });
        });
    }

    // --- Clé bêta ----------------------------------------------------------
    let key_group = adw::PreferencesGroup::builder()
        .title("Clé bêta MakeMKV")
        .description("La clé publique est récupérée et installée automatiquement. Une clé achetée n'est jamais modifiée.")
        .build();
    let source_row = adw::EntryRow::builder().title("Source de la clé").build();
    source_row.set_text(&cfg.borrow().beta_key_sources.join(" "));
    let check_btn = gtk::Button::builder().label("Vérifier").valign(gtk::Align::Center).build();
    let current_key = crate::makemkv::settings::current_key();
    let key_row = adw::ActionRow::builder()
        .title("Clé installée")
        .subtitle(current_key.as_deref().map(beta_key::mask).unwrap_or_else(|| "aucune".into()))
        .build();
    key_row.add_suffix(&check_btn);
    key_group.add(&key_row);
    key_group.add(&source_row);

    {
        let source_row = source_row.clone();
        check_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            let sources: Vec<String> = source_row.text().split_whitespace().map(String::from).collect();
            let tx = tx.clone();
            let (rtx, rrx) = async_channel::bounded::<String>(1);
            runtime().spawn(async move {
                let msg = match beta_key::update_key(http(), &sources).await {
                    Ok(KeyUpdate::Updated(k)) => {
                        log(&tx, format!("Nouvelle clé bêta installée : {}", beta_key::mask(&k)));
                        beta_key::mask(&k)
                    }
                    Ok(KeyUpdate::UpToDate) => "à jour".to_string(),
                    Ok(KeyUpdate::PurchasedKey) => "clé achetée (inchangée)".to_string(),
                    Err(e) => {
                        emit(&tx, AppEvent::Toast(format!("Clé bêta : {e}")));
                        "échec de la vérification".to_string()
                    }
                };
                let _ = rtx.send(msg).await;
            });
            let btn = btn.clone();
            let key_row = key_row.clone();
            glib::spawn_future_local(async move {
                if let Ok(m) = rrx.recv().await {
                    key_row.set_subtitle(&m);
                }
                btn.set_sensitive(true);
            });
        });
    }

    page.add(&lang_group);
    page.add(&meta_group);
    page.add(&rip_group);
    page.add(&key_group);
    dialog.add(&page);

    // À la fermeture : on relit tous les champs et on renvoie la config.
    dialog.connect_closed(move |_| {
        let mut c = cfg.borrow().clone();
        if let Some((code, _)) = LANGUAGES.get(lang_row.selected() as usize) {
            c.language = code.to_string();
        }
        c.keep_original_audio = vo_row.is_active();
        c.tmdb_api_key = tmdb_row.text().trim().to_string();
        c.min_length_secs = minlen_row.value() as u32;
        c.eject_when_done = eject_row.is_active();
        let sources: Vec<String> = source_row.text().split_whitespace().map(String::from).collect();
        if !sources.is_empty() {
            c.beta_key_sources = sources;
        }
        on_close(c);
    });
    dialog.present(Some(parent));
}
