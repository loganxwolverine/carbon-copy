//! Composant « Lecteur » : modèle, firmware, connexion, capacités, disque inséré.

use adw::prelude::*;

use super::section::Section;
use crate::drive_info::DriveInfo;

pub struct DrivePanel {
    pub root: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    connection: gtk::Label,
    serial: gtk::Label,
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
        let (reads, writes) = (chips(), chips());
        let grid = gtk::Grid::builder().row_spacing(6).column_spacing(12).build();
        for (row, (k, w)) in [
            ("Connexion", connection.clone().upcast::<gtk::Widget>()),
            ("N° de série", serial.clone().upcast()),
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
        let p = Self { root, title, subtitle, connection, serial, reads, writes, media, region, verdict };
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
        self.serial.set_text(i.serial.as_deref().unwrap_or("—"));
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

/// « 2009-07-30 » → « 30/07/2009 ».
fn french_date(iso: &str) -> String {
    let p: Vec<&str> = iso.split('-').collect();
    match p.as_slice() {
        [y, m, d] => format!("{d}/{m}/{y}"),
        _ => iso.to_string(),
    }
}
