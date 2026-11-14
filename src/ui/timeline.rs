//! Frise des titres du disque : une barre par titre, proportionnelle à sa
//! durée. Le titre principal porte une étoile, le titre choisi est en
//! couleur ; un clic sur une barre choisit le titre.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

/// Nombre maximal de barres (les plus longues) : les Blu-ray comptent
/// parfois des dizaines de titres.
const MAX_ROWS: usize = 8;
const ROW_H: f64 = 18.0;
const GAP: f64 = 6.0;
const LABEL_W: f64 = 64.0;

#[derive(Clone, Debug)]
pub struct Bar {
    pub index: usize,
    pub secs: u64,
    pub main: bool,
}

#[derive(Default)]
struct Data {
    bars: Vec<Bar>,
    selected: usize,
}

type Pick = Rc<RefCell<Option<Box<dyn Fn(usize)>>>>;

pub struct Timeline {
    pub root: gtk::DrawingArea,
    data: Rc<RefCell<Data>>,
    on_pick: Pick,
}

impl Timeline {
    pub fn new() -> Self {
        let on_pick: Pick = Rc::default();
        let data: Rc<RefCell<Data>> = Rc::default();
        let root = gtk::DrawingArea::builder()
            .hexpand(true)
            .css_classes(["timeline"])
            .tooltip_text("Durée des titres du disque (pastille dorée : titre principal). Cliquez pour en choisir un.")
            .build();
        {
            let data = data.clone();
            root.set_draw_func(move |w, cr, width, _| {
                let d = data.borrow();
                let max = d.bars.iter().map(|b| b.secs).max().unwrap_or(1).max(1) as f64;
                let fg = w.color();
                for (row, b) in d.bars.iter().enumerate() {
                    let y = row as f64 * (ROW_H + GAP);
                    // Libellé « ★ T2 ».
                    cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, if b.index == d.selected { 1.0 } else { 0.6 });
                    cr.select_font_face("Sans", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
                    cr.set_font_size(11.0);
                    cr.move_to(14.0, y + ROW_H - 5.0);
                    let _ = cr.show_text(&format!("T{}", b.index));
                    // Titre principal : pastille dorée devant le libellé.
                    if b.main {
                        cr.set_source_rgba(0.96, 0.76, 0.07, 1.0);
                        cr.arc(5.0, y + ROW_H / 2.0, 4.0, 0.0, 2.0 * std::f64::consts::PI);
                        let _ = cr.fill();
                    }
                    // Barre.
                    let avail = (width as f64 - LABEL_W).max(10.0);
                    let len = (b.secs as f64 / max * avail).max(4.0);
                    if b.index == d.selected {
                        cr.set_source_rgba(0.21, 0.52, 0.89, 1.0);
                    } else {
                        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.22);
                    }
                    rounded(cr, LABEL_W, y + 3.0, len, ROW_H - 6.0);
                    let _ = cr.fill();
                }
            });
        }
        let click = gtk::GestureClick::new();
        {
            let (data, on_pick) = (data.clone(), on_pick.clone());
            click.connect_released(move |_, _, _, y| {
                let row = (y / (ROW_H + GAP)) as usize;
                let idx = data.borrow().bars.get(row).map(|b| b.index);
                if let (Some(i), Some(f)) = (idx, on_pick.borrow().as_ref()) {
                    f(i);
                }
            });
        }
        root.add_controller(click);
        Self { root, data, on_pick }
    }

    /// Clic sur la barre d'un titre.
    pub fn connect_pick(&self, f: impl Fn(usize) + 'static) {
        self.on_pick.replace(Some(Box::new(f)));
    }

    /// Titres affichés : les plus longs, dans l'ordre du disque.
    pub fn set_titles(&self, mut bars: Vec<Bar>) {
        bars.sort_by(|a, b| b.secs.cmp(&a.secs));
        bars.truncate(MAX_ROWS);
        bars.sort_by_key(|b| b.index);
        let n = bars.len() as f64;
        self.root.set_content_height((n * (ROW_H + GAP)) as i32);
        self.data.borrow_mut().bars = bars;
        self.root.queue_draw();
    }

    pub fn set_selected(&self, index: usize) {
        self.data.borrow_mut().selected = index;
        self.root.queue_draw();
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64) {
    let r = (h / 2.0).min(w / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + r, r, std::f64::consts::FRAC_PI_2, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
}
