//! Section homogène : carte arrondie, liseré coloré, en-tête avec pastille
//! d'icône, titre et compteur. Toutes les cases de la fenêtre en sont faites.

use adw::prelude::*;

pub struct Section {
    pub root: gtk::Box,
    pub content: gtk::Box,
    /// Petit compteur à droite de l'en-tête (« 2 / 3 »…), masqué si vide.
    pub count: gtk::Label,
}

impl Section {
    /// `class` : classe de couleur (`sec-disc`, `sec-audio`…).
    pub fn new(title: &str, icon: &str, class: &str) -> Self {
        // Pastille : l'icône elle-même porte le fond coloré (taille fixe en CSS).
        let badge = gtk::Image::builder().icon_name(icon).pixel_size(18).valign(gtk::Align::Center).css_classes(["section-badge"]).build();
        let label = gtk::Label::builder().label(title).xalign(0.0).hexpand(true).css_classes(["section-title"]).build();
        let count = gtk::Label::builder().css_classes(["section-count"]).valign(gtk::Align::Center).visible(false).build();
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.append(&badge);
        header.append(&label);
        header.append(&count);

        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).vexpand(true).build();
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).css_classes(["section", class]).build();
        root.append(&header);
        root.append(&content);
        Self { root, content, count }
    }

    /// Section dont le contenu défile si nécessaire (listes longues).
    pub fn scrolled(title: &str, icon: &str, class: &str, child: &impl IsA<gtk::Widget>) -> Self {
        let s = Self::new(title, icon, class);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .propagate_natural_height(true)
            .max_content_height(420)
            .child(child)
            .build();
        s.content.append(&scroll);
        s
    }

    pub fn set_count(&self, text: &str) {
        self.count.set_text(text);
        self.count.set_visible(!text.is_empty());
    }
}
