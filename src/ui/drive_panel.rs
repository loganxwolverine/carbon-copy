//! Composant « Lecteur » : modèle, firmware, connexion, capacités, disque inséré.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::section::Section;
use crate::drive_info::DriveInfo;

pub struct DrivePanel {
    pub root: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    connection: gtk::Label,
    serial: gtk::Label,
    /// Numéro de série réel ; masqué à l'écran tant qu'on ne clique pas sur l'œil.
    serial_value: Rc<RefCell<String>>,
    serial_btn: gtk::ToggleButton,
    reads: gtk::FlowBox,
    writes: gtk::FlowBox,
    media: gtk::Label,
    region: gtk::Label,
    verdict: gtk::Label,
}

fn key(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).yalign(0.0).css_classes(["dim-label", "caption-heading"]).build()
}

fn value() -> gtk::Label {
    gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).hexpand(true).build()
}

fn chips() -> gtk::FlowBox {
    gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(3)
        .max_children_per_line(12)
        .row_spacing(4)
        .column_spacing(4)
        .hexpand(true)
        .build()
}

fn fill_chips(b: &gtk::FlowBox, items: &[&str], accent: &[&str]) {
    b.remove_all();
    if items.is_empty() {
        b.append(&gtk::Label::builder().label("—").css_classes(["dim-label"]).build());
    }
    for it in items {
        let mut classes = vec!["chip"];
        if accent.contains(it) {
            classes.push("chip-accent");
        }
        b.append(&gtk::Label::builder().label(*it).css_classes(classes).build());
    }
}

impl DrivePanel {
    pub fn new() -> Self {
        let title = gtk::Label::builder().label("Lecteur optique").xalign(0.0).wrap(true).css_classes(["title-4"]).build();
        let subtitle = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim-label"]).build();
        let head_text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        head_text.append(&title);
        head_text.append(&subtitle);
        let head = head_text;

        let (connection, serial, media, region) = (value(), value(), value(), value());
        serial.set_selectable(false);
        serial.set_hexpand(false);
        let serial_value: Rc<RefCell<String>> = Rc::default();
        let serial_btn = gtk::ToggleButton::builder()
            .icon_name("view-reveal-symbolic")
            .tooltip_text("Afficher le numéro de série")
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build();
        {
            let (serial, value) = (serial.clone(), serial_value.clone());
            serial_btn.connect_toggled(move |b| {
                show_serial(&serial, &value.borrow(), b.is_active());
                b.set_icon_name(if b.is_active() { "view-conceal-symbolic" } else { "view-reveal-symbolic" });
            });
        }
        let serial_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        serial_box.append(&serial);
        serial_box.append(&serial_btn);
        let (reads, writes) = (chips(), chips());
        let grid = gtk::Grid::builder().row_spacing(6).column_spacing(12).build();
        for (row, (k, w)) in [
            ("Connexion", connection.clone().upcast::<gtk::Widget>()),
            ("N° de série", serial_box.clone().upcast()),
            ("Lecture", reads.clone().upcast()),
            ("Gravure", writes.clone().upcast()),
            ("Zone DVD", region.clone().upcast()),
            ("Disque", media.clone().upcast()),
        ]
        .into_iter()
        .enumerate()
        {
            grid.attach(&key(k), 0, row as i32, 1, 1);
            grid.attach(&w, 1, row as i32, 1, 1);
        }
        let verdict = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["caption"]).build();

        let section = Section::new("Lecteur", "drive-optical-symbolic", "sec-drive");
        section.content.append(&head);
        section.content.append(&grid);
        section.content.append(&verdict);
        let root = section.root;
        let p = Self { root, title, subtitle, connection, serial, serial_value, serial_btn, reads, writes, media, region, verdict };
        p.update(None);
        p
    }

    pub fn update(&self, info: Option<&DriveInfo>) {
        let Some(i) = info else {
            self.title.set_text("Aucun lecteur optique");
            self.subtitle.set_text("Branchez un lecteur DVD ou Blu-ray.");
            for l in [&self.connection, &self.serial, &self.media, &self.region] {
                l.set_text("—");
            }
            self.serial_value.replace(String::new());
            self.serial_btn.set_visible(false);
            fill_chips(&self.reads, &[], &[]);
            fill_chips(&self.writes, &[], &[]);
            self.verdict.set_text("");
            return;
        };
        self.title.set_text(&i.name());
        let fw = match &i.firmware_date {
            Some(d) => format!("Firmware {} du {}", i.firmware, french_date(d)),
            None => format!("Firmware {}", i.firmware),
        };
        self.subtitle.set_text(&fw);

        let mut conn = Vec::new();
        conn.extend(i.bus.clone());
        conn.push(i.device.clone());
        if let Some(s) = i.max_speed {
            conn.push(format!("{s}× max"));
        }
        self.connection.set_text(&conn.join(" · "));
        self.serial_value.replace(i.serial.clone().unwrap_or_default());
        self.serial_btn.set_visible(i.serial.is_some());
        show_serial(&self.serial, &self.serial_value.borrow(), self.serial_btn.is_active());
        fill_chips(&self.reads, &i.reads, &["DVD", "Blu-ray"]);
        fill_chips(&self.writes, &i.writes, &[]);
        self.media.set_text(i.media.as_deref().unwrap_or("Aucun"));
        self.region.remove_css_class("warning");
        match i.region {
            Some(r) if r.zones().is_empty() => {
                self.region.set_text("Aucune zone définie : décryptage des DVD très lent");
                self.region.add_css_class("warning");
            }
            Some(r) => {
                let z: Vec<String> = r.zones().iter().map(|z| z.to_string()).collect();
                self.region.set_text(&format!("{} · {} changement(s) restant(s)", z.join(", "), r.changes_left));
            }
            None => self.region.set_text("Sans verrou de zone (RPC-1)"),
        }

        self.verdict.remove_css_class("warning");
        self.verdict.remove_css_class("success");
        if i.reads_bluray() {
            self.verdict.set_text("✔ Lit les DVD et les Blu-ray. Les Blu-ray UHD exigent en plus un firmware compatible LibreDrive.");
            self.verdict.add_css_class("success");
        } else if i.reads.contains(&"DVD") {
            self.verdict.set_text("✔ Compatible DVD. Ce lecteur ne lit pas les Blu-ray.");
            self.verdict.add_css_class("warning");
        } else {
            self.verdict.set_text("Ce lecteur ne lit ni les DVD ni les Blu-ray.");
            self.verdict.add_css_class("warning");
        }
    }
}

/// Numéro de série en clair ou masqué (captures d'écran partagées…).
fn show_serial(label: &gtk::Label, value: &str, reveal: bool) {
    if value.is_empty() {
        label.set_text("—");
    } else if reveal {
        label.set_text(value);
    } else {
        label.set_text(&"•".repeat(value.chars().count().min(12)));
    }
}

/// « 2009-07-30 » → « 30/07/2009 ».
fn french_date(iso: &str) -> String {
    let p: Vec<&str> = iso.split('-').collect();
    match p.as_slice() {
        [y, m, d] => format!("{d}/{m}/{y}"),
        _ => iso.to_string(),
    }
}
