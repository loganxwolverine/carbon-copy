//! Composant « Fiche du film » : affiche, titre, année, résumé, bouton de correction.

use adw::prelude::*;

use super::section::Section;

pub struct MovieCard {
    pub root: gtk::Box,
    pub poster_stack: gtk::Stack,
    pub poster: gtk::Picture,
    pub heading: gtk::Label,
    pub details: gtk::Label,
    pub overview: gtk::Label,
    pub change_btn: gtk::Button,
}

impl MovieCard {
    pub fn new() -> Self {
        let poster = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).build();
        let placeholder = gtk::Image::builder().icon_name("video-x-generic-symbolic").pixel_size(64).css_classes(["dim-label"]).build();
        let spinner = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        let poster_stack = gtk::Stack::builder()
            .width_request(180)
            .height_request(270)
            .valign(gtk::Align::Start)
            .overflow(gtk::Overflow::Hidden)
            .css_classes(["poster"])
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        poster_stack.add_named(&placeholder, Some("placeholder"));
        poster_stack.add_named(&spinner, Some("loading"));
        poster_stack.add_named(&poster, Some("poster"));

        let heading = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).css_classes(["movie-title"]).build();
        let details = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim-label"]).build();
        let overview = gtk::Label::builder()
            .xalign(0.0)
            .yalign(0.0)
            .wrap(true)
            .lines(8)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .vexpand(true)
            .build();
        let change_btn = gtk::Button::builder().label("Ce n'est pas le bon film ?").halign(gtk::Align::Start).css_classes(["pill"]).build();

        let text = gtk::Box::new(gtk::Orientation::Vertical, 8);
        text.set_hexpand(true);
        text.append(&heading);
        text.append(&details);
        text.append(&overview);
        text.append(&change_btn);

        // L'image garde sinon sa taille réelle (342×513) : on la borne en largeur
        // ET en hauteur pour que le bandeau reste compact.
        let poster_h = adw::Clamp::builder().orientation(gtk::Orientation::Vertical).maximum_size(270).tightening_threshold(270).child(&poster_stack).build();
        let poster_box = adw::Clamp::builder().maximum_size(180).tightening_threshold(180).valign(gtk::Align::Start).child(&poster_h).build();

        let section = Section::new("Film", "video-x-generic-symbolic", "sec-film");
        section.root.set_hexpand(true);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        section.content.append(&body);
        let root = section.root;
        body.append(&poster_box);
        body.append(&text);
        Self { root, poster_stack, poster, heading, details, overview, change_btn }
    }
}
