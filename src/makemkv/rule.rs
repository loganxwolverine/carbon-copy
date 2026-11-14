//! Traduction de la sélection de pistes en règle de sélection MakeMKV.
//!
//! `makemkvcon` n'accepte pas de liste de pistes en ligne de commande : il
//! applique la règle `app_DefaultSelectionString`, une suite de jetons
//! `+sel:condition` / `-sel:condition` évalués dans l'ordre. Une condition ne
//! désigne pas une piste précise mais un ensemble (type, langue, forcé,
//! cœur audio…). On construit donc, groupe par groupe (type × langue), la
//! règle la plus fidèle possible :
//!
//! - groupe entièrement coché → `+sel:(audio&fre)` ;
//! - seuls les sous-titres forcés → `+sel:(subtitle&fre&forced)` ;
//! - tout sauf les forcés → `+sel:(subtitle&fre),-sel:(subtitle&fre&forced)` ;
//! - pareil pour les flux audio « cœur » dérivés (`core`) ;
//! - sinon, le groupe entier est extrait (sur-ensemble) et `mkvmerge`
//!   retirera ensuite les pistes en trop.
//!
//! La même description sert à PRÉDIRE les pistes extraites ; le fichier
//! produit est ensuite vérifié (voir `backend`), car la règle est interprétée
//! par MakeMKV et non par nous.

use std::collections::BTreeSet;

use super::disc::{Stream, StreamKind, Title};
use super::robot::{attr, stream_flags};
use crate::selection::Selection;

/// Condition d'un jeton.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cond {
    kind: StreamKind,
    /// Code langue tel que MakeMKV le donne (`fre`), ou `None` = sans langue.
    lang: Option<String>,
    forced: bool,
    core: bool,
    /// `Some(true)` : `lossless` (DTS-HD MA, TrueHD, PCM, FLAC) ;
    /// `Some(false)` : `lossy`.
    lossless: Option<bool>,
}

impl Cond {
    fn group(kind: StreamKind, lang: &Option<String>) -> Self {
        Self { kind, lang: lang.clone(), forced: false, core: false, lossless: None }
    }

    fn render(&self) -> String {
        let mut parts = vec![match self.kind {
            StreamKind::Audio => "audio",
            StreamKind::Subtitle => "subtitle",
            _ => "video",
        }
        .to_string()];
        parts.push(self.lang.clone().unwrap_or_else(|| "nolang".into()));
        if self.forced {
            parts.push("forced".into());
        }
        if self.core {
            parts.push("core".into());
        }
        match self.lossless {
            Some(true) => parts.push("lossless".into()),
            Some(false) => parts.push("lossy".into()),
            None => {}
        }
        format!("({})", parts.join("&"))
    }

    fn matches(&self, s: &Stream) -> bool {
        s.kind() == self.kind
            && lang_key(s) == self.lang
            && (!self.forced || is_forced_flag(s))
            && (!self.core || is_core_flag(s))
            && self.lossless.is_none_or(|l| is_lossless(s) == l)
    }
}

/// Langue brute d'un flux (`None` si absente ou indéterminée).
fn lang_key(s: &Stream) -> Option<String> {
    let raw = s.attrs.get(&attr::LANG_CODE).map(|v| v.trim().to_lowercase()).unwrap_or_default();
    (!raw.is_empty() && raw != "und").then_some(raw)
}

/// MakeMKV juge « forcé » d'après le drapeau du flux (et non d'après le libellé).
fn is_forced_flag(s: &Stream) -> bool {
    s.flags() & stream_flags::FORCED_SUBTITLES != 0
}

fn is_core_flag(s: &Stream) -> bool {
    s.flags() & stream_flags::CORE_AUDIO != 0
}

/// Audio sans perte au sens de MakeMKV (les cœurs DTS / AC-3 extraits d'une
/// piste HD sont, eux, avec perte).
fn is_lossless(s: &Stream) -> bool {
    if s.kind() != StreamKind::Audio || is_core_flag(s) {
        return false;
    }
    let id = s.codec_id();
    let short = s.get(attr::CODEC_SHORT).to_uppercase();
    id.starts_with("A_TRUEHD") || id.starts_with("A_PCM") || id.starts_with("A_FLAC") || id.starts_with("A_MLP") || (id.starts_with("A_DTS") && short.contains("MA"))
}

fn is_mvc(s: &Stream) -> bool {
    s.codec_id().contains("MVC")
}

/// Plan d'extraction : la règle à appliquer et les pistes qu'elle doit produire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Valeur de `app_DefaultSelectionString`.
    pub rule: String,
    /// Pistes audio et sous-titres attendues dans le fichier (⊇ sélection).
    pub contents: Selection,
    /// Nombre de pistes vidéo attendues.
    pub video: usize,
}

impl Plan {
    /// Le fichier contiendra exactement la sélection : aucun remux nécessaire.
    pub fn is_exact(&self, sel: &Selection) -> bool {
        &self.contents == sel && self.video <= 1
    }

    /// Plan de secours : toutes les pistes (comportement historique).
    pub fn all(title: &Title) -> Self {
        let idx = |k| title.streams_of(k).map(|s| s.index).collect::<BTreeSet<_>>();
        Self {
            rule: crate::makemkv::settings::SELECT_ALL.into(),
            contents: Selection { audio: idx(StreamKind::Audio), subtitles: idx(StreamKind::Subtitle) },
            video: title.streams_of(StreamKind::Video).count(),
        }
    }
}

/// Construit le plan d'extraction de `sel` pour `title`.
pub fn plan(title: &Title, sel: &Selection) -> Plan {
    let mut tokens: Vec<(bool, Cond)> = Vec::new();
    for (kind, wanted) in [(StreamKind::Audio, &sel.audio), (StreamKind::Subtitle, &sel.subtitles)] {
        // Groupes par langue, dans l'ordre d'apparition sur le disque.
        let mut langs: Vec<Option<String>> = Vec::new();
        for s in title.streams_of(kind) {
            let k = lang_key(s);
            if !langs.contains(&k) {
                langs.push(k);
            }
        }
        for lang in langs {
            let group: Vec<&Stream> = title.streams_of(kind).filter(|s| lang_key(s) == lang).collect();
            let picked: BTreeSet<usize> = group.iter().map(|s| s.index).filter(|i| wanted.contains(i)).collect();
            if picked.is_empty() {
                continue;
            }
            let all: BTreeSet<usize> = group.iter().map(|s| s.index).collect();
            let base = Cond::group(kind, &lang);
            if picked == all {
                tokens.push((true, base));
                continue;
            }
            // Sous-ensemble distinguable par un attribut (forcé, cœur audio,
            // sans perte / avec perte) ? On essaie chaque attribut, en positif
            // (« seulement ceux-là ») puis en négatif (« tous sauf ceux-là »).
            let refinements: Vec<(fn(&Stream) -> bool, Cond)> = match kind {
                StreamKind::Subtitle => vec![(is_forced_flag, Cond { forced: true, ..base.clone() })],
                _ => vec![
                    (is_core_flag, Cond { core: true, ..base.clone() }),
                    (is_lossless, Cond { lossless: Some(true), ..base.clone() }),
                    (|s: &Stream| !is_lossless(s), Cond { lossless: Some(false), ..base.clone() }),
                ],
            };
            let mut done = false;
            for (flag, refine) in refinements {
                let flagged: BTreeSet<usize> = group.iter().filter(|s| flag(s)).map(|s| s.index).collect();
                if flagged.is_empty() || flagged == all {
                    continue;
                }
                if picked == flagged {
                    tokens.push((true, refine));
                } else if picked == &all - &flagged {
                    tokens.push((true, base.clone()));
                    tokens.push((false, refine));
                } else {
                    continue;
                }
                done = true;
                break;
            }
            if !done {
                // Indiscernable par une règle : groupe entier, tri par mkvmerge.
                tokens.push((true, base));
            }
        }
    }

    let mut rule = vec!["-sel:all".to_string(), "+sel:video".into(), "-sel:mvcvideo".into()];
    rule.extend(tokens.iter().map(|(add, c)| format!("{}sel:{}", if *add { '+' } else { '-' }, c.render())));

    // Prédiction : application séquentielle des jetons, comme MakeMKV.
    let mut contents = Selection::default();
    for s in &title.streams {
        let mut on = false;
        for (add, c) in &tokens {
            if c.matches(s) {
                on = *add;
            }
        }
        if on {
            match s.kind() {
                StreamKind::Audio => contents.audio.insert(s.index),
                StreamKind::Subtitle => contents.subtitles.insert(s.index),
                _ => false,
            };
        }
    }
    let video = title.streams_of(StreamKind::Video).filter(|s| !is_mvc(s)).count();
    Plan { rule: rule.join(","), contents, video }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::makemkv::disc::DiscInfo;
    use crate::makemkv::robot::parse_line;

    fn title(raw: &str) -> Title {
        let lines: Vec<_> = raw.lines().map(parse_line).collect();
        DiscInfo::from_lines(&lines).titles.remove(0)
    }

    // Vidéo, audio fre AC3 / eng AC3 / fre DTS, sous-titres fre complets,
    // fre forcés, eng.
    const DISC: &str = r#"SINFO:0,0,5,0,"V_MPEG2"
SINFO:0,1,5,0,"A_AC3"
SINFO:0,1,3,0,"fre"
SINFO:0,2,5,0,"A_AC3"
SINFO:0,2,3,0,"eng"
SINFO:0,3,5,0,"A_DTS"
SINFO:0,3,3,0,"fre"
SINFO:0,4,5,0,"S_VOBSUB"
SINFO:0,4,3,0,"fre"
SINFO:0,5,5,0,"S_VOBSUB"
SINFO:0,5,3,0,"fre"
SINFO:0,5,22,0,"6144"
SINFO:0,6,5,0,"S_VOBSUB"
SINFO:0,6,3,0,"eng""#;

    fn sel(a: &[usize], s: &[usize]) -> Selection {
        Selection { audio: a.iter().copied().collect(), subtitles: s.iter().copied().collect() }
    }

    #[test]
    fn exact_rule_for_language_groups_and_forced() {
        let t = title(DISC);
        let want = sel(&[2], &[5]);
        let p = plan(&t, &want);
        assert_eq!(p.rule, "-sel:all,+sel:video,-sel:mvcvideo,+sel:(audio&eng),+sel:(subtitle&fre&forced)");
        assert!(p.is_exact(&want));
    }

    #[test]
    fn excludes_forced_with_a_negative_token() {
        let t = title(DISC);
        let want = sel(&[1, 3], &[4]);
        let p = plan(&t, &want);
        assert!(p.rule.ends_with("+sel:(audio&fre),+sel:(subtitle&fre),-sel:(subtitle&fre&forced)"), "{}", p.rule);
        assert!(p.is_exact(&want));
    }

    #[test]
    fn falls_back_to_superset_when_indistinguishable() {
        let t = title(DISC);
        // Une seule des deux pistes audio françaises : impossible par règle.
        let want = sel(&[1], &[]);
        let p = plan(&t, &want);
        assert_eq!(p.contents, sel(&[1, 3], &[]));
        assert!(!p.is_exact(&want));
    }

    /// Cas réel (Blu-ray) : anglais DTS-HD MA + cœur DTS + AC-3. Seule la
    /// piste HD est voulue : `lossless` la désigne sans remux.
    #[test]
    fn lossless_isolates_the_hd_track() {
        let t = title(
            r#"SINFO:0,0,5,0,"V_MPEG4/ISO/AVC"
SINFO:0,1,5,0,"A_DTS"
SINFO:0,1,6,0,"DTS-HD MA"
SINFO:0,1,3,0,"eng"
SINFO:0,2,5,0,"A_DTS"
SINFO:0,2,6,0,"DTS"
SINFO:0,2,3,0,"eng"
SINFO:0,2,22,0,"256"
SINFO:0,3,5,0,"A_AC3"
SINFO:0,3,3,0,"eng"
SINFO:0,4,5,0,"A_DTS"
SINFO:0,4,6,0,"DTS"
SINFO:0,4,3,0,"fre""#,
        );
        let want = sel(&[1, 4], &[]);
        let p = plan(&t, &want);
        assert!(p.rule.ends_with("+sel:(audio&eng&lossless),+sel:(audio&fre)"), "{}", p.rule);
        assert!(p.is_exact(&want));
        // Tout l'anglais sauf la piste HD.
        let want = sel(&[2, 3], &[]);
        let p = plan(&t, &want);
        assert!(p.rule.ends_with("+sel:(audio&eng),-sel:(audio&eng&lossless)"), "{}", p.rule);
        assert!(p.is_exact(&want));
        // Le cœur seul reste désigné par `core`.
        let p = plan(&t, &sel(&[2], &[]));
        assert!(p.rule.ends_with("+sel:(audio&eng&core)"), "{}", p.rule);
    }

    #[test]
    fn streams_without_language_use_nolang() {
        let t = title("SINFO:0,0,5,0,\"V_MPEG2\"\nSINFO:0,1,5,0,\"A_AC3\"\nSINFO:0,2,5,0,\"S_VOBSUB\"\nSINFO:0,2,3,0,\"und\"");
        let p = plan(&t, &sel(&[1], &[2]));
        assert!(p.rule.ends_with("+sel:(audio&nolang),+sel:(subtitle&nolang)"), "{}", p.rule);
        assert_eq!(p.contents, sel(&[1], &[2]));
    }
}
