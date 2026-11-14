//! Assistant d'obtention de la clé TMDB (reconnaissance des films).
//!
//! Trois étapes guidées avec des boutons qui ouvrent les bonnes pages. La clé
//! est détectée automatiquement dans le presse-papiers quand l'utilisateur
//! revient dans l'application, vérifiée auprès de TMDB, puis enregistrée.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::backend::{http, runtime};
use crate::metadata;

const SIGNUP_URL: &str = "https://www.themoviedb.org/signup";
const API_URL: &str = "https://www.themoviedb.org/settings/api";

fn open_url(parent: &impl IsA<gtk::Window>, url: &str) {
    gtk::UriLauncher::new(url).launch(Some(parent), None::<&gio::Cancellable>, |_| {});
}

fn step(n: u32, title: &str, subtitle: &str, button: Option<(&str, gtk::Button)>) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).subtitle(subtitle).subtitle_lines(4).build();
    let badge = gtk::Label::builder().label(n.to_string()).valign(gtk::Align::Center).css_classes(["step-badge"]).build();
    row.add_prefix(&badge);
    if let Some((label, btn)) = button {
        btn.set_label(label);
        btn.set_valign(gtk::Align::Center);
        row.add_suffix(&btn);
    }
    row
}

/// Affiche l'assistant ; `on_saved` reçoit la clé vérifiée.
pub fn show(parent: &adw::ApplicationWindow, current: &str, on_saved: impl Fn(String) + 'static) {
    let dialog = adw::Dialog::builder().title("Reconnaissance des films").content_width(620).build();
    let intro = gtk::Label::builder()
        .label(
            "Carbon Copy utilise TMDB, la base de films collaborative et gratuite, pour reconnaître \
             le film, afficher son affiche et nommer le fichier. Il suffit d'une clé personnelle \
             gratuite : trois étapes, environ deux minutes.",
        )
        .wrap(true)
        .xalign(0.0)
        .build();

    let signup_btn = gtk::Button::new();
    let api_btn = gtk::Button::new();
    api_btn.add_css_class("suggested-action");
    let steps = adw::PreferencesGroup::new();
    steps.add(&step(1, "Créer un compte gratuit", "Puis cliquez sur le lien de confirmation reçu par e-mail.", Some(("Créer le compte", signup_btn.clone()))));
    steps.add(&step(
        2,
        "Demander une clé d'API",
        "Choisissez « Developer », acceptez les conditions et remplissez le formulaire : \
         usage personnel, nom de l'application « Carbon Copy ».",
        Some(("Ouvrir la page", api_btn.clone())),
    ));
    steps.add(&step(
        3,
        "Copier la clé",
        "Copiez la « Clé d'API » (ou le « Jeton d'accès en lecture ») puis revenez ici : \
         elle est détectée automatiquement.",
        None,
    ));

    let entry = adw::PasswordEntryRow::builder().title("Clé API TMDB").build();
    entry.set_text(current);
    let key_group = adw::PreferencesGroup::new();
    key_group.add(&entry);
    let status = gtk::Label::builder().wrap(true).xalign(0.0).css_classes(["caption"]).build();

    let later = gtk::Button::builder().label("Plus tard").build();
    let save = gtk::Button::builder().label("Enregistrer").sensitive(false).css_classes(["suggested-action"]).build();
    let buttons = gtk::Box::builder().spacing(12).halign(gtk::Align::End).build();
    buttons.append(&later);
    buttons.append(&save);

    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).margin_top(12).margin_bottom(24).margin_start(24).margin_end(24).build();
    for w in [intro.upcast_ref::<gtk::Widget>(), steps.upcast_ref(), key_group.upcast_ref(), status.upcast_ref(), buttons.upcast_ref()] {
        body.append(w);
    }
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&body));
    dialog.set_child(Some(&view));

    {
        let p = parent.clone();
        signup_btn.connect_clicked(move |_| open_url(&p, SIGNUP_URL));
        let p = parent.clone();
        api_btn.connect_clicked(move |_| open_url(&p, API_URL));
        let d = dialog.clone();
        later.connect_clicked(move |_| {
            d.close();
        });
    }

    // Vérification (anti-rebond : la dernière saisie seulement).
    let valid: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let gen = Rc::new(Cell::new(0u64));
    let on_saved = Rc::new(on_saved);
    let saved = Rc::new(Cell::new(false));
    let do_save = {
        let (dialog, valid, on_saved, saved) = (dialog.clone(), valid.clone(), on_saved.clone(), saved.clone());
        move || {
            if let Some(k) = valid.borrow().clone() {
                if !saved.replace(true) {
                    on_saved(k);
                }
                dialog.close();
            }
        }
    };
    let do_save = Rc::new(do_save);
    {
        let do_save = do_save.clone();
        save.connect_clicked(move |_| do_save());
    }
    {
        let (status, save, valid, gen, do_save) = (status.clone(), save.clone(), valid.clone(), gen.clone(), do_save.clone());
        entry.connect_changed(move |e| {
            let key = e.text().trim().to_string();
            let g = gen.get() + 1;
            gen.set(g);
            valid.replace(None);
            save.set_sensitive(false);
            status.remove_css_class("success");
            status.remove_css_class("error");
            if key.is_empty() {
                status.set_text("");
                return;
            }
            status.set_text("Vérification auprès de TMDB…");
            let (status, save, valid, gen, do_save) = (status.clone(), save.clone(), valid.clone(), gen.clone(), do_save.clone());
            glib::spawn_future_local(async move {
                glib::timeout_future(std::time::Duration::from_millis(500)).await;
                if gen.get() != g {
                    return;
                }
                let k = key.clone();
                let res = runtime().spawn(async move { metadata::tmdb_check(http(), &k).await }).await;
                if gen.get() != g {
                    return;
                }
                match res {
                    Ok(Ok(())) => {
                        status.set_text("✔ Clé valide : reconnaissance des films activée.");
                        status.add_css_class("success");
                        valid.replace(Some(key));
                        save.set_sensitive(true);
                        // Clé trouvée et vérifiée : on enregistre sans autre clic.
                        glib::timeout_future(std::time::Duration::from_millis(1200)).await;
                        if gen.get() == g {
                            do_save();
                        }
                    }
                    Ok(Err(e)) => {
                        status.set_text(&format!("✖ {e:#}. Vérifiez la clé copiée."));
                        status.add_css_class("error");
                    }
                    Err(_) => {}
                }
            });
        });
    }

    // Retour dans l'application après la copie : lecture du presse-papiers.
    let check_clipboard = {
        let entry = entry.clone();
        move || {
            let entry = entry.clone();
            let clipboard = WidgetExt::display(&entry).clipboard();
            glib::spawn_future_local(async move {
                if let Ok(Some(text)) = clipboard.read_text_future().await {
                    let text = text.trim().to_string();
                    if metadata::looks_like_tmdb_key(&text) && entry.text().trim() != text {
                        entry.set_text(&text);
                    }
                }
            });
        }
    };
    let handler = {
        let check = check_clipboard.clone();
        parent.connect_is_active_notify(move |w| {
            if w.is_active() {
                check();
            }
        })
    };
    let handler = RefCell::new(Some(handler));
    {
        let parent = parent.clone();
        dialog.connect_closed(move |_| {
            if let Some(h) = handler.take() {
                parent.disconnect(h);
            }
        });
    }
    check_clipboard();
    dialog.present(Some(parent));
}
