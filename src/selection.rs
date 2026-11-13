//! Règles métier de pré-sélection des pistes audio et sous-titres.
//!
//! Règle critique pour les sous-titres, dans la langue préférée (ex. `fra`) :
//! - les sous-titres **forcés** sont toujours pré-sélectionnés ;
//! - les sous-titres **complets** ne sont PAS pré-sélectionnés si le disque
//!   propose une piste audio dans cette langue (on regarde le film en VF, seuls
//!   les passages en langue étrangère doivent être sous-titrés) ;
//! - s'il n'existe aucune piste audio dans la langue préférée, les sous-titres
//!   complets de cette langue sont pré-sélectionnés (sinon le film serait
//!   incompréhensible).
//!
//! L'utilisateur peut ensuite tout modifier dans l'interface.

use std::collections::BTreeSet;

use crate::config::Config;
use crate::makemkv::disc::{Stream, StreamKind, Title};

/// Sélection courante : indices de flux (index `SINFO`) retenus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub audio: BTreeSet<usize>,
    pub subtitles: BTreeSet<usize>,
}

/// Normalise un code ISO 639-2 « bibliographique » (B) en « terminologique » (T).
///
/// MakeMKV et mkvmerge utilisent souvent la forme B (`fre`, `ger`…), alors
/// que l'utilisateur pensera plutôt `fra`, `deu`. On compare toujours la forme T.
pub fn normalize_lang(code: &str) -> String {
    let c = code.trim().to_lowercase();
    let t = match c.as_str() {
        "fre" => "fra",
        "ger" => "deu",
        "dut" => "nld",
        "chi" => "zho",
        "cze" => "ces",
        "gre" => "ell",
        "per" => "fas",
        "rum" => "ron",
        "slo" => "slk",
        "ice" => "isl",
        "arm" => "hye",
        "alb" => "sqi",
        "baq" => "eus",
        "bur" => "mya",
        "geo" => "kat",
        "mac" => "mkd",
        "mao" => "mri",
        "may" => "msa",
        "tib" => "bod",
        "wel" => "cym",
        // Codes IETF courts éventuellement saisis par l'utilisateur.
        "fr" => "fra",
        "en" => "eng",
        "de" => "deu",
        "es" => "spa",
        "it" => "ita",
        other => return other.to_string(),
    };
    t.to_string()
}

/// Nom français des langues les plus courantes.
pub fn french_lang_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "fra" => "Français",
        "eng" => "Anglais",
        "deu" => "Allemand",
        "spa" => "Espagnol",
        "ita" => "Italien",
        "por" => "Portugais",
        "nld" => "Néerlandais",
        "jpn" => "Japonais",
        "zho" => "Chinois",
        "kor" => "Coréen",
        "rus" => "Russe",
        "pol" => "Polonais",
        "ces" => "Tchèque",
        "hun" => "Hongrois",
        "swe" => "Suédois",
        "dan" => "Danois",
        "nor" => "Norvégien",
        "fin" => "Finnois",
        "tur" => "Turc",
        "ara" => "Arabe",
        "heb" => "Hébreu",
        "hin" => "Hindi",
        "tha" => "Thaï",
        "ell" => "Grec",
        "und" => "Indéterminée",
        _ => return None,
    })
}

/// Calcule la sélection par défaut d'un titre selon les préférences.
pub fn default_selection(title: &Title, cfg: &Config) -> Selection {
    let lang = normalize_lang(&cfg.language);
    let mut sel = Selection::default();

    // --- Audio -------------------------------------------------------------
    let audio: Vec<_> = title.streams_of(StreamKind::Audio).collect();
    let is_candidate = |s: &Stream| !s.is_commentary() && !s.is_visually_impaired() && !s.is_secondary_audio();

    let preferred: Vec<_> = audio.iter().copied().filter(|s| s.lang() == lang && is_candidate(s)).collect();
    // On évite les doublons « cœur dérivé » (AC3 extrait d'un TrueHD…) sauf
    // s'il n'existe rien d'autre dans la langue.
    if let Some(first) = preferred.iter().find(|s| !s.is_derived_core()).or(preferred.first()) {
        sel.audio.insert(first.index);
    }

    // VO : première piste audio non dérivée d'une autre langue.
    if cfg.keep_original_audio {
        if let Some(vo) = audio
            .iter()
            .find(|s| is_candidate(s) && !s.is_derived_core() && s.lang() != lang)
        {
            sel.audio.insert(vo.index);
        }
    }

    // Filet de sécurité : jamais de fichier sans audio.
    if sel.audio.is_empty() {
        if let Some(s) = audio.iter().find(|s| !s.is_derived_core()).or(audio.first()) {
            sel.audio.insert(s.index);
        }
    }

    // --- Sous-titres -------------------------------------------------------
    let has_audio_in_lang = audio.iter().any(|s| s.lang() == lang);
    for s in title.streams_of(StreamKind::Subtitle).filter(|s| s.lang() == lang) {
        if s.is_forced() || !has_audio_in_lang {
            sel.subtitles.insert(s.index);
        }
    }

    sel
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::makemkv::disc::DiscInfo;
    use crate::makemkv::robot::parse_line;

    fn disc(raw: &str) -> DiscInfo {
        let lines: Vec<_> = raw.lines().map(parse_line).collect();
        DiscInfo::from_lines(&lines)
    }

    const VF_DISC: &str = r#"TINFO:0,9,0,"2:00:00"
SINFO:0,0,1,0,"Video"
SINFO:0,1,1,0,"Audio"
SINFO:0,1,3,0,"eng"
SINFO:0,2,1,0,"Audio"
SINFO:0,2,3,0,"fre"
SINFO:0,3,1,0,"Audio"
SINFO:0,3,3,0,"fre"
SINFO:0,3,22,0,"2048"
SINFO:0,4,1,0,"Subtitles"
SINFO:0,4,3,0,"fre"
SINFO:0,5,1,0,"Subtitles"
SINFO:0,5,3,0,"fre"
SINFO:0,5,22,0,"6144"
SINFO:0,6,1,0,"Subtitles"
SINFO:0,6,3,0,"eng""#;

    #[test]
    fn only_forced_subs_when_audio_matches() {
        let d = disc(VF_DISC);
        let cfg = Config::default();
        let sel = default_selection(&d.titles[0], &cfg);
        // Audio VF (non dérivée) + VO anglaise (keep_original_audio par défaut).
        assert_eq!(sel.audio, BTreeSet::from([1, 2]));
        // Seul le sous-titre forcé français.
        assert_eq!(sel.subtitles, BTreeSet::from([5]));
    }

    #[test]
    fn full_subs_when_no_audio_in_language() {
        let d = disc(
            r#"SINFO:0,0,1,0,"Audio"
SINFO:0,0,3,0,"eng"
SINFO:0,1,1,0,"Subtitles"
SINFO:0,1,3,0,"fre"
SINFO:0,2,1,0,"Subtitles"
SINFO:0,2,3,0,"fre"
SINFO:0,2,30,0,"PGS French (forced only)""#,
        );
        let cfg = Config { keep_original_audio: false, ..Config::default() };
        let sel = default_selection(&d.titles[0], &cfg);
        assert_eq!(sel.audio, BTreeSet::from([0]));
        assert_eq!(sel.subtitles, BTreeSet::from([1, 2]));
    }

    #[test]
    fn lang_normalization() {
        assert_eq!(normalize_lang("FRE"), "fra");
        assert_eq!(normalize_lang("fr"), "fra");
        assert_eq!(normalize_lang("jpn"), "jpn");
    }
}
