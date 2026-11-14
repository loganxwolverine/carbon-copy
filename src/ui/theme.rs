//! Habillage : feuille de style fixe + couleur d'ambiance tirée de l'affiche.
//!
//! Chaque section a sa couleur (pastille d'icône + liseré). La section « Film »
//! et le fond de la fenêtre prennent la couleur dominante de l'affiche : chaque
//! film donne sa propre ambiance.

use gtk::gdk;
use gtk::prelude::*;

const CSS: &str = "
/* --- Sections homogènes ------------------------------------------------ */
.section {
  background-color: alpha(@card_bg_color, 0.92);
  border-radius: 18px;
  border-top: 4px solid @accent_bg_color;
  box-shadow: 0 2px 6px alpha(black, 0.22);
  padding: 14px 16px 16px 16px;
}
.section-title { font-weight: 800; font-size: 1.08em; }
.section-badge {
  border-radius: 10px;
  min-width: 34px; min-height: 34px;
  color: white;
  background-color: @accent_bg_color;
}
.section-count {
  border-radius: 999px; padding: 2px 10px;
  font-size: 0.85em; font-weight: bold;
  background-color: alpha(currentColor, 0.08);
}
.section list.boxed-list { background-color: alpha(currentColor, 0.035); }

.sec-drive  { border-top-color: #2ec27e; } .sec-drive  .section-badge { background-color: #26a269; }
.sec-disc   { border-top-color: #3584e4; } .sec-disc   .section-badge { background-color: #3584e4; }
.sec-audio  { border-top-color: #ff7800; } .sec-audio  .section-badge { background-color: #e66100; }
.sec-subs   { border-top-color: #c061cb; } .sec-subs   .section-badge { background-color: #9141ac; }
.sec-output { border-top-color: #f5c211; } .sec-output .section-badge { background-color: #e5a50a; }

.sec-audio  .section-count { color: #ffa348; background-color: alpha(#ff7800, 0.15); }
.sec-subs   .section-count { color: #dc8add; background-color: alpha(#c061cb, 0.15); }

/* --- Puces de capacités du lecteur ---------------------------------------- */
.chip { padding: 1px 9px; border-radius: 999px; font-size: 0.85em; background-color: alpha(currentColor, 0.08); }
.chip-accent { background-color: alpha(#2ec27e, 0.25); color: #57e389; font-weight: bold; }

/* --- Assistant TMDB ------------------------------------------------------- */
.step-badge {
  min-width: 28px; min-height: 28px; border-radius: 999px;
  font-weight: 800; color: white; background-color: #01b4e4;
}

/* --- Fiche du film ---------------------------------------------------------- */
.poster { border-radius: 12px; box-shadow: 0 6px 18px alpha(black, 0.45); }
.movie-title { font-size: 2.1em; font-weight: 800; }
.movie-chip { padding: 3px 11px; border-radius: 999px; font-size: 0.88em; background-color: alpha(currentColor, 0.16); border: 1px solid alpha(currentColor, 0.14); }
.movie-chip.rating { background-color: alpha(#f5c211, 0.25); color: #f8e45c; font-weight: 800; }
.movie-chip.genre { background-color: alpha(#3584e4, 0.22); }
.movie-chip.person { background-color: alpha(white, 0.08); }
.already-ripped { padding: 6px 12px; border-radius: 10px; background-color: alpha(#e5a50a, 0.22); }

/* --- Pistes ------------------------------------------------------------------ */
.codec { padding: 1px 8px; border-radius: 6px; font-size: 0.8em; font-weight: bold; background-color: alpha(currentColor, 0.10); }
.codec.dts    { background-color: alpha(#ff7800, 0.28); }
.codec.truehd { background-color: alpha(#c061cb, 0.30); }
.codec.ac3    { background-color: alpha(#3584e4, 0.28); }
.codec.lossless { background-color: alpha(#2ec27e, 0.28); }
.codec.layout { background-color: alpha(#9a9996, 0.22); }
.flag { font-size: 1.25em; }

/* --- Frise des titres ---------------------------------------------------------- */
.timeline { margin-top: 4px; }

/* --- Nom du fichier en blocs ---------------------------------------------------- */
.name-part { padding: 2px 8px; border-radius: 6px; font-family: monospace; font-size: 0.9em; }
.name-part.title { background-color: alpha(#f5c211, 0.28); font-weight: bold; }
.name-part.year  { background-color: alpha(#3584e4, 0.28); }
.name-part.tmdb  { background-color: alpha(#01b4e4, 0.25); }
.name-part.tech  { background-color: alpha(currentColor, 0.10); }
.name-part.lang  { background-color: alpha(#2ec27e, 0.28); font-weight: bold; }

/* --- Écran de rip ---------------------------------------------------------------- */
.rip-poster { border-radius: 16px; box-shadow: 0 10px 30px alpha(black, 0.55); }
.rip-title { font-size: 2.4em; font-weight: 800; }
.rip-percent { font-size: 2.6em; font-weight: 800; }
.rip-eta { font-size: 1.6em; font-weight: 700; }
.rip-speed { font-size: 1.1em; }
.step { padding: 6px 14px; border-radius: 999px; background-color: alpha(currentColor, 0.06); color: alpha(currentColor, 0.45); }
.step.active { background-color: alpha(#3584e4, 0.30); color: inherit; font-weight: bold; }
.step.done { background-color: alpha(#2ec27e, 0.25); color: inherit; }
.step-arrow { color: alpha(currentColor, 0.35); }
.done-title { font-size: 2.2em; font-weight: 800; color: #57e389; }
.done-fact { padding: 8px 16px; border-radius: 12px; background-color: alpha(currentColor, 0.07); }
.done-fact .value { font-size: 1.3em; font-weight: 800; }

/* --- Vidéothèque ------------------------------------------------------------------ */
.library-card { padding: 6px; border-radius: 14px; }
.library-card:hover { background-color: alpha(currentColor, 0.07); }
.library-poster { border-radius: 10px; box-shadow: 0 4px 12px alpha(black, 0.4); background-color: alpha(currentColor, 0.06); }
.library-missing { opacity: 0.45; }
.stat-value { font-size: 2em; font-weight: 800; }
.hero-title { font-size: 2em; font-weight: 800; }
.tag-row label.mono { font-family: monospace; }

/* --- Zone d'action ---------------------------------------------------------- */
.action-zone {
  padding: 12px 18px;
  background-color: alpha(@headerbar_bg_color, 0.85);
  border-top: 1px solid alpha(currentColor, 0.10);
}
button.rip-button {
  padding: 10px 34px;
  border-radius: 999px;
  font-weight: 800;
  font-size: 1.1em;
  color: white;
  background-image: linear-gradient(120deg, #3584e4, #9141ac 60%, #e01b24);
  box-shadow: 0 3px 10px alpha(#9141ac, 0.45);
}
button.rip-button:hover { background-image: linear-gradient(120deg, #62a0ea, #c061cb 60%, #f66151); }
button.rip-button.cancel { background-image: linear-gradient(120deg, #c01c28, #a51d2d); box-shadow: none; }
progressbar.rip > trough { min-height: 10px; border-radius: 999px; }
progressbar.rip > trough > progress {
  min-height: 10px; border-radius: 999px;
  background-image: linear-gradient(90deg, #3584e4, #9141ac, #ff7800);
}
";

pub struct Theme {
    dynamic: gtk::CssProvider,
    color: std::cell::Cell<(u8, u8, u8)>,
    backdrop: std::cell::RefCell<Option<std::path::PathBuf>>,
}

impl Theme {
    /// Installe la feuille de style fixe et prépare la couche dynamique.
    pub fn install() -> Self {
        let fixed = gtk::CssProvider::new();
        fixed.load_from_string(CSS);
        let dynamic = gtk::CssProvider::new();
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &fixed, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            gtk::style_context_add_provider_for_display(&display, &dynamic, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
        }
        let t = Self { dynamic, color: std::cell::Cell::new(DEFAULT_COLOR), backdrop: Default::default() };
        t.reset();
        t
    }

    /// Couleur d'ambiance par défaut (violet GNOME), sans image de fond.
    pub fn reset(&self) {
        self.backdrop.replace(None);
        self.set_color(DEFAULT_COLOR);
    }

    pub fn color(&self) -> (u8, u8, u8) {
        self.color.get()
    }

    /// Image de fond de la fiche du film (fichier local), ou aucune.
    pub fn set_backdrop(&self, path: Option<std::path::PathBuf>) {
        self.backdrop.replace(path);
        self.apply();
    }

    /// Applique la couleur d'ambiance `(r, g, b)` à la fiche du film et au fond.
    pub fn set_color(&self, rgb: (u8, u8, u8)) {
        self.color.set(rgb);
        self.apply();
    }

    fn apply(&self) {
        let (r, g, b) = self.color.get();
        let c = format!("rgb({r},{g},{b})");
        // Image de fond : basse définition étirée (flou naturel), assombrie
        // par un dégradé pour garder le texte lisible.
        let (film_bg, rip_bg) = match self.backdrop.borrow().as_ref() {
            Some(p) => {
                let url = gtk::gio::File::for_path(p).uri();
                (
                    format!(
                        "background-image: linear-gradient(90deg, alpha(@card_bg_color, 0.97) 0%, alpha(@card_bg_color, 0.90) 45%, alpha(@card_bg_color, 0.62) 75%, alpha({c}, 0.40) 100%), url(\"{url}\");\n  background-size: cover, cover;\n  background-position: center, center 30%;"
                    ),
                    format!(
                        "background-image: linear-gradient(alpha(@window_bg_color, 0.80), alpha(@window_bg_color, 0.93)), url(\"{url}\");\n  background-size: cover, cover;\n  background-position: center, center;"
                    ),
                )
            }
            None => (format!("background-image: linear-gradient(105deg, alpha({c}, 0.30), alpha({c}, 0.0) 70%);"), String::new()),
        };
        self.dynamic.load_from_string(&format!(
            "
.ambient {{
  background-image:
    radial-gradient(circle at 12% 0%, alpha({c}, 0.38), transparent 55%),
    radial-gradient(circle at 100% 100%, alpha({c}, 0.16), transparent 45%);
}}
.sec-film {{
  border-top-color: {c};
  {film_bg}
}}
.sec-film .section-badge {{ background-color: {c}; }}
.rip-screen {{
  {rip_bg}
}}
.step.active {{ background-color: alpha({c}, 0.45); }}
"
        ));
    }
}

const DEFAULT_COLOR: (u8, u8, u8) = (0x91, 0x41, 0xac);

/// Couleur dominante « vive » d'une affiche.
///
/// On répartit les pixels par teinte (12 secteurs), pondérés par saturation ×
/// luminosité, et on garde la moyenne du secteur le plus représenté ; la
/// couleur est ensuite ravivée pour rester lisible sur fond sombre comme clair.
pub fn dominant_color(texture: &gdk::Texture) -> (u8, u8, u8) {
    let (w, h) = (texture.width().max(1) as usize, texture.height().max(1) as usize);
    let stride = w * 4;
    let mut buf = vec![0u8; stride * h];
    texture.download(&mut buf, stride);

    let mut buckets = [(0f64, 0f64, 0f64, 0f64); 12];
    for px in buf.chunks_exact(4).step_by(3) {
        // Format de `download` : B8G8R8A8 prémultiplié.
        let (b, g, r, a) = (px[0] as f64, px[1] as f64, px[2] as f64, px[3] as f64);
        if a < 128.0 {
            continue;
        }
        let (hue, sat, val) = rgb_to_hsv(r / 255.0, g / 255.0, b / 255.0);
        let weight = sat * val;
        if weight < 0.12 {
            continue; // Gris, noirs et blancs n'ont pas de teinte utile.
        }
        let i = ((hue / 30.0) as usize).min(11);
        buckets[i].0 += r * weight;
        buckets[i].1 += g * weight;
        buckets[i].2 += b * weight;
        buckets[i].3 += weight;
    }
    let Some(best) = buckets.iter().filter(|b| b.3 > 0.0).max_by(|a, b| a.3.total_cmp(&b.3)) else {
        return DEFAULT_COLOR;
    };
    let (r, g, b) = (best.0 / best.3 / 255.0, best.1 / best.3 / 255.0, best.2 / best.3 / 255.0);
    let (hue, sat, _) = rgb_to_hsv(r, g, b);
    let (r, g, b) = hsv_to_rgb(hue, sat.clamp(0.55, 0.85), 0.80);
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

fn rgb_to_hsv(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (r + m, g + m, b + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_roundtrip() {
        let (h, s, v) = rgb_to_hsv(0.8, 0.2, 0.2);
        let (r, g, b) = hsv_to_rgb(h, s, v);
        assert!((r - 0.8).abs() < 1e-9 && (g - 0.2).abs() < 1e-9 && (b - 0.2).abs() < 1e-9);
    }
}
