//! Écran « Copie en cours » puis « Copie terminée ».
//!
//! Pendant le rip : grande affiche, anneau de progression aux couleurs du
//! film, temps restant et vitesse en gros, frise des étapes. À la fin :
//! récapitulatif (taille, durée du film, durée de la copie) et actions.

use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::gdk;

use crate::events::{Estimate, Phase};
use crate::progress;

const STEPS: [(Phase, &str); 4] = [
    (Phase::Copy, "Lecture du disque"),
    (Phase::Verify, "Vérification"),
    (Phase::Finalize, "Finalisation"),
    (Phase::Eject, "Éjection"),
];

pub struct RipView {
    pub root: gtk::Stack,
    poster: gtk::Picture,
    title: gtk::Label,
    file: gtk::Label,
    ring: gtk::DrawingArea,
    fraction: Rc<Cell<f64>>,
    color: Rc<Cell<(u8, u8, u8)>>,
    percent: gtk::Label,
    eta: gtk::Label,
    speed: gtk::Label,
    phase_label: gtk::Label,
    steps: Vec<(Phase, gtk::Label, gtk::Label)>,
    done_poster: gtk::Picture,
    done_title: gtk::Label,
    done_file: gtk::Label,
    fact_size: gtk::Label,
    fact_duration: gtk::Label,
    fact_time: gtk::Label,
    pub play_btn: gtk::Button,
    pub folder_btn: gtk::Button,
    pub next_btn: gtk::Button,
}

fn big_poster() -> (gtk::Picture, adw::Clamp) {
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .width_request(280)
        .height_request(420)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["rip-poster"])
        .build();
    let v = adw::Clamp::builder().orientation(gtk::Orientation::Vertical).maximum_size(420).tightening_threshold(420).child(&picture).build();
    let h = adw::Clamp::builder().maximum_size(280).tightening_threshold(280).valign(gtk::Align::Center).child(&v).build();
    (picture, h)
}

fn fact(caption: &str) -> (gtk::Box, gtk::Label) {
    let value = gtk::Label::builder().css_classes(["value"]).build();
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).css_classes(["done-fact"]).build();
    b.append(&gtk::Label::builder().label(caption).css_classes(["dim-label", "caption"]).build());
    b.append(&value);
    (b, value)
}

impl RipView {
    pub fn new() -> Self {
        // --- En cours -------------------------------------------------------
        let (poster, poster_box) = big_poster();
        let title = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["rip-title"]).build();
        let file = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).css_classes(["dim-label", "monospace"]).build();

        let fraction = Rc::new(Cell::new(0.0f64));
        let color = Rc::new(Cell::new((0x35u8, 0x84u8, 0xe4u8)));
        let ring = gtk::DrawingArea::builder().content_width(230).content_height(230).halign(gtk::Align::Center).build();
        {
            let (fraction, color) = (fraction.clone(), color.clone());
            ring.set_draw_func(move |_, cr, w, h| {
                let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
                let r = cx.min(cy) - 12.0;
                cr.set_line_width(16.0);
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                cr.set_source_rgba(0.5, 0.5, 0.5, 0.18);
                cr.arc(cx, cy, r, 0.0, 2.0 * PI);
                let _ = cr.stroke();
                let f = fraction.get().clamp(0.0, 1.0);
                if f > 0.0 {
                    let (r8, g8, b8) = color.get();
                    let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, w as f64, h as f64);
                    grad.add_color_stop_rgb(0.0, r8 as f64 / 255.0, g8 as f64 / 255.0, b8 as f64 / 255.0);
                    grad.add_color_stop_rgb(1.0, 1.0, 0.47, 0.0);
                    let _ = cr.set_source(&grad);
                    cr.arc(cx, cy, r, -PI / 2.0, -PI / 2.0 + f * 2.0 * PI);
                    let _ = cr.stroke();
                }
            });
        }
        let percent = gtk::Label::builder().css_classes(["rip-percent"]).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        let ring_overlay = gtk::Overlay::builder().child(&ring).halign(gtk::Align::Start).build();
        ring_overlay.add_overlay(&percent);

        let eta = gtk::Label::builder().xalign(0.0).css_classes(["rip-eta"]).build();
        let speed = gtk::Label::builder().xalign(0.0).css_classes(["rip-speed", "dim-label"]).build();
        let phase_label = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim-label"]).build();
        let figures = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).valign(gtk::Align::Center).build();
        figures.append(&eta);
        figures.append(&speed);
        figures.append(&phase_label);
        let meter = gtk::Box::new(gtk::Orientation::Horizontal, 32);
        meter.append(&ring_overlay);
        meter.append(&figures);

        let steps_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let mut steps = Vec::new();
        for (i, (phase, text)) in STEPS.iter().enumerate() {
            let arrow = gtk::Label::builder().label("→").css_classes(["step-arrow"]).visible(i > 0).build();
            let l = gtk::Label::builder().label(*text).css_classes(["step"]).build();
            steps_box.append(&arrow);
            steps_box.append(&l);
            steps.push((*phase, l, arrow));
        }

        let info = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(22).valign(gtk::Align::Center).hexpand(true).build();
        for w in [title.upcast_ref::<gtk::Widget>(), file.upcast_ref(), meter.upcast_ref(), steps_box.upcast_ref()] {
            info.append(w);
        }
        let running = gtk::Box::builder().spacing(48).halign(gtk::Align::Center).valign(gtk::Align::Center).margin_start(48).margin_end(48).build();
        running.append(&poster_box);
        running.append(&info);

        // --- Terminé ---------------------------------------------------------
        let (done_poster, done_poster_box) = big_poster();
        let done_head = gtk::Label::builder().label("✔ Copie terminée").xalign(0.0).css_classes(["done-title"]).build();
        let done_title = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["rip-title"]).build();
        let done_file = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).css_classes(["dim-label", "monospace"]).build();
        let (size_box, fact_size) = fact("Taille du fichier");
        let (dur_box, fact_duration) = fact("Durée du film");
        let (time_box, fact_time) = fact("Durée de la copie");
        let facts = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        facts.append(&size_box);
        facts.append(&dur_box);
        facts.append(&time_box);
        let play_btn = gtk::Button::builder().label("▶  Lire le film").css_classes(["pill", "suggested-action"]).build();
        let folder_btn = gtk::Button::builder().label("Ouvrir le dossier").css_classes(["pill"]).build();
        let next_btn = gtk::Button::builder().label("Disque suivant").css_classes(["pill"]).build();
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        actions.append(&play_btn);
        actions.append(&folder_btn);
        actions.append(&next_btn);
        let done_info = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).valign(gtk::Align::Center).hexpand(true).build();
        for w in [done_head.upcast_ref::<gtk::Widget>(), done_title.upcast_ref(), done_file.upcast_ref(), facts.upcast_ref(), actions.upcast_ref()] {
            done_info.append(w);
        }
        let done = gtk::Box::builder().spacing(48).halign(gtk::Align::Center).valign(gtk::Align::Center).margin_start(48).margin_end(48).build();
        done.append(&done_poster_box);
        done.append(&done_info);

        let root = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(400)
            .vexpand(true)
            .css_classes(["rip-screen"])
            .build();
        root.add_named(&running, Some("running"));
        root.add_named(&done, Some("done"));

        Self {
            root,
            poster,
            title,
            file,
            ring,
            fraction,
            color,
            percent,
            eta,
            speed,
            phase_label,
            steps,
            done_poster,
            done_title,
            done_file,
            fact_size,
            fact_duration,
            fact_time,
            play_btn,
            folder_btn,
            next_btn,
        }
    }

    /// Prépare l'écran pour un nouveau rip.
    pub fn start(&self, title: &str, file_name: &str, poster: Option<gdk::Paintable>, color: (u8, u8, u8), eject: bool) {
        self.title.set_text(title);
        self.file.set_text(file_name);
        self.poster.set_paintable(poster.as_ref());
        self.done_poster.set_paintable(poster.as_ref());
        self.color.set(color);
        for (phase, _, arrow) in &self.steps {
            if *phase == Phase::Eject {
                arrow.set_visible(eject);
            }
        }
        if let Some((_, l, _)) = self.steps.iter().find(|(p, _, _)| *p == Phase::Eject) {
            l.set_visible(eject);
        }
        self.eta.set_text("Calcul du temps restant…");
        self.speed.set_text("");
        self.phase_label.set_text("");
        self.update(Phase::Copy, "Démarrage de MakeMKV…", 0.0, None);
        self.root.set_visible_child_name("running");
    }

    pub fn update(&self, phase: Phase, label: &str, fraction: f64, estimate: Option<Estimate>) {
        self.fraction.set(fraction);
        self.ring.queue_draw();
        self.percent.set_text(&format!("{:.0} %", fraction * 100.0));
        self.phase_label.set_text(label);
        match estimate {
            Some(e) => {
                self.eta.set_text(&format!("Reste {}", progress::short_duration(e.remaining)));
                self.speed.set_text(&e.bytes_per_sec.map(|b| format!("{} Mo/s", format!("{:.1}", b / 1e6).replace('.', ","))).unwrap_or_default());
            }
            None if phase > Phase::Copy => {
                self.eta.set_text("Presque terminé…");
                self.speed.set_text("");
            }
            None => {}
        }
        for (p, l, _) in &self.steps {
            l.remove_css_class("active");
            l.remove_css_class("done");
            let text = STEPS.iter().find(|(sp, _)| sp == p).map(|(_, t)| *t).unwrap_or("");
            if *p < phase {
                l.add_css_class("done");
                l.set_text(&format!("✓ {text}"));
            } else {
                if *p == phase {
                    l.add_css_class("active");
                }
                l.set_text(text);
            }
        }
    }

    /// Écran final.
    pub fn finish(&self, title: &str, file_name: &str, size: u64, film_secs: u64, elapsed: Duration) {
        self.done_title.set_text(title);
        self.done_file.set_text(file_name);
        self.fact_size.set_text(&format!("{:.1} Go", size as f64 / 1e9).replace('.', ","));
        self.fact_duration.set_text(&if film_secs > 0 { progress::short_duration(Duration::from_secs(film_secs)) } else { "—".into() });
        self.fact_time.set_text(&progress::short_duration(elapsed));
        self.root.set_visible_child_name("done");
    }
}
