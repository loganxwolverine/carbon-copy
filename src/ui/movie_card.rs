//! Composant « Fiche du film » : image de fond, affiche, titre, pastilles
//! (note, genres, durée, réalisateur, acteurs), résumé, bouton de correction.

use adw::prelude::*;

use super::section::Section;
use crate::metadata::Movie;

pub const POSTER_W: i32 = 200;
pub const POSTER_H: i32 = 300;

pub struct MovieCard {
    pub root: gtk::Box,
    pub poster_stack: gtk::Stack,
    pub poster: gtk::Picture,
    pub heading: gtk::Label,
    pub details: gtk::Label,
    pub chips: gtk::Box,
    pub overview: gtk::Label,
    /// Bandeau « déjà copié le … ».
    pub already: gtk::Label,
    pub change_btn: gtk::Button,
}

fn chip(text: &str, class: &str) -> gtk::Label {
    gtk::Label::builder().label(text).css_classes(["movie-chip", class]).build()
}

impl MovieCard {
    pub fn new() -> Self {
        let poster = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).build();
        let placeholder = gtk::Image::builder().icon_name("camera-video-symbolic").pixel_size(64).css_classes(["dim-label"]).build();
        let spinner = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        let poster_stack = gtk::Stack::builder()
            .width_request(POSTER_W)
            .height_request(POSTER_H)
            .valign(gtk::Align::Start)
            .overflow(gtk::Overflow::Hidden)
            .css_classes(["poster"])
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(450)
            .build();
        poster_stack.add_named(&placeholder, Some("placeholder"));
        poster_stack.add_named(&spinner, Some("loading"));
        poster_stack.add_named(&poster, Some("poster"));

        let heading = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).css_classes(["movie-title"]).build();
        let details = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim-label"]).build();
        // Une seule ligne (nombre de pastilles borné) : une FlowBox mesure mal
        // sa hauteur à côté d'un texte à la ligne et chevaucherait le résumé.
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let overview = gtk::Label::builder()
            .xalign(0.0)
            .yalign(0.0)
            .wrap(true)
            .lines(6)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .vexpand(true)
            .build();
        let already = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).css_classes(["already-ripped"]).build();
        let change_btn = gtk::Button::builder().label("Ce n'est pas le bon film ?").halign(gtk::Align::Start).css_classes(["pill"]).build();

        let text = gtk::Box::new(gtk::Orientation::Vertical, 10);
        text.set_hexpand(true);
        for w in [heading.upcast_ref::<gtk::Widget>(), details.upcast_ref(), chips.upcast_ref(), overview.upcast_ref(), already.upcast_ref(), change_btn.upcast_ref()] {
            text.append(w);
        }

        // L'image garde sinon sa taille réelle (342×513) : on la borne en largeur
        // ET en hauteur pour que le bandeau reste compact.
        let poster_h = adw::Clamp::builder().orientation(gtk::Orientation::Vertical).maximum_size(POSTER_H).tightening_threshold(POSTER_H).child(&poster_stack).build();
        let poster_box = adw::Clamp::builder().maximum_size(POSTER_W).tightening_threshold(POSTER_W).valign(gtk::Align::Start).child(&poster_h).build();

        let section = Section::new("Film", "camera-video-symbolic", "sec-film");
        section.root.set_hexpand(true);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 22);
        section.content.append(&body);
        let root = section.root;
        body.append(&poster_box);
        body.append(&text);
        Self { root, poster_stack, poster, heading, details, chips, overview, already, change_btn }
    }

    /// Pastilles : note, durée, genres, réalisateur, premiers rôles.
    pub fn set_chips(&self, movie: Option<&Movie>) {
        while let Some(c) = self.chips.first_child() {
            self.chips.remove(&c);
        }
        let Some(m) = movie else {
            self.chips.set_visible(false);
            return;
        };
        if let Some(r) = m.rating_label() {
            self.chips.append(&chip(&format!("★ {r}"), "rating"));
        }
        if let Some(min) = m.runtime.filter(|r| *r > 0) {
            self.chips.append(&chip(&format!("{} h {:02}", min / 60, min % 60), "runtime"));
        }
        for g in m.genres.iter().take(2) {
            self.chips.append(&chip(&g.name, "genre"));
        }
        let directors = m.directors();
        if !directors.is_empty() {
            self.chips.append(&chip(&format!("🎬 {}", directors.join(", ")), "person"));
        }
        for a in m.top_cast(2) {
            self.chips.append(&chip(&a, "person"));
        }
        self.chips.set_visible(self.chips.first_child().is_some());
    }
}
