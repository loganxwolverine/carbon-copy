//! Modèle de données du disque (titres, flux) reconstruit à partir des lignes
//! `CINFO` / `TINFO` / `SINFO` émises par `makemkvcon -r info`.

use std::collections::BTreeMap;

use super::robot::{attr, parse_duration, stream_flags, RobotLine};

/// Type de média détecté.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Dvd,
    BluRay,
    UhdBluRay,
    Unknown,
}

impl MediaType {
    pub fn label(self) -> &'static str {
        match self {
            MediaType::Dvd => "DVD",
            MediaType::BluRay => "Blu-ray",
            MediaType::UhdBluRay => "Blu-ray UHD (4K)",
            MediaType::Unknown => "Inconnu",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    Other,
}

/// Un flux (piste) d'un titre.
#[derive(Debug, Clone, Default)]
pub struct Stream {
    pub index: usize,
    pub attrs: BTreeMap<u32, String>,
}

impl Stream {
    fn get(&self, id: u32) -> &str {
        self.attrs.get(&id).map(String::as_str).unwrap_or("")
    }

    /// Type du flux.
    ///
    /// Le libellé `SINFO 1` est TRADUIT par MakeMKV (« Vidéo », « Sous-titres »
    /// en français) : on se fonde d'abord sur l'identifiant de codec Matroska,
    /// stable quelle que soit la langue.
    pub fn kind(&self) -> StreamKind {
        match self.codec_id().get(..2) {
            Some("V_") => return StreamKind::Video,
            Some("A_") => return StreamKind::Audio,
            Some("S_") => return StreamKind::Subtitle,
            _ => {}
        }
        match self.get(attr::TYPE) {
            "Video" | "Vidéo" => StreamKind::Video,
            "Audio" => StreamKind::Audio,
            "Subtitles" | "Sous-titres" => StreamKind::Subtitle,
            _ => StreamKind::Other,
        }
    }

    /// Identifiant de codec Matroska (`V_MPEG2`, `A_DTS`…).
    pub fn codec_id(&self) -> &str {
        self.get(attr::CODEC_ID)
    }

    /// Nombre de canaux audio (0 si inconnu).
    pub fn channels(&self) -> u32 {
        self.get(attr::AUDIO_CHANNELS).trim().parse().unwrap_or(0)
    }

    /// Disposition des canaux, ex. « 5.1(side) ».
    pub fn audio_layout(&self) -> &str {
        self.get(attr::AUDIO_LAYOUT)
    }

    /// Code langue ISO 639-2 normalisé en variante « T » (ex. `fre` → `fra`).
    pub fn lang(&self) -> String {
        crate::selection::normalize_lang(self.get(attr::LANG_CODE))
    }

    /// Nom de la langue en français (repli sur le libellé MakeMKV, en anglais).
    pub fn lang_name(&self) -> String {
        if let Some(fr) = crate::selection::french_lang_name(&self.lang()) {
            return fr.to_string();
        }
        match self.get(attr::LANG_NAME) {
            "" => "Langue inconnue".into(),
            s => s.to_string(),
        }
    }

    pub fn codec(&self) -> &str {
        match self.get(attr::CODEC_SHORT) {
            "" => self.get(attr::CODEC_LONG),
            s => s,
        }
    }

    pub fn flags(&self) -> u32 {
        self.get(attr::STREAM_FLAGS).trim().parse().unwrap_or(0)
    }

    /// Texte descriptif complet fourni par MakeMKV (utile pour repérer « forced »).
    fn descriptive_text(&self) -> String {
        format!(
            "{} {} {}",
            self.get(attr::NAME),
            self.get(attr::TREE_INFO),
            self.get(attr::MKV_FLAGS_TEXT)
        )
        .to_lowercase()
    }

    /// Sous-titre « forcé » (forced only) ?
    ///
    /// MakeMKV expose les sous-titres forcés des Blu-ray comme un flux dérivé
    /// portant le drapeau `FORCED_SUBTITLES` (4096). Par sécurité on regarde
    /// aussi le libellé (« forced » / « forcé »), certaines versions ou certains
    /// DVD ne positionnant que le texte.
    pub fn is_forced(&self) -> bool {
        if self.kind() != StreamKind::Subtitle {
            return false;
        }
        let t = self.descriptive_text();
        self.flags() & stream_flags::FORCED_SUBTITLES != 0
            || t.contains("forced")
            || t.contains("forcé")
    }

    pub fn is_commentary(&self) -> bool {
        self.flags() & (stream_flags::DIRECTORS_COMMENTS | stream_flags::ALT_DIRECTORS_COMMENTS) != 0
            || self.descriptive_text().contains("comment")
    }

    /// Flux audio « cœur » dérivé (ex. AC3 extrait d'un TrueHD) : doublon.
    pub fn is_derived_core(&self) -> bool {
        self.kind() == StreamKind::Audio
            && self.flags() & (stream_flags::CORE_AUDIO | stream_flags::DERIVED_STREAM) != 0
    }

    pub fn is_visually_impaired(&self) -> bool {
        self.flags() & stream_flags::VISUALLY_IMPAIRED != 0
    }

    pub fn is_secondary_audio(&self) -> bool {
        self.flags() & stream_flags::SECONDARY_AUDIO != 0
    }

    pub fn video_size(&self) -> &str {
        self.get(attr::VIDEO_SIZE)
    }

    /// Libellé lisible pour l'interface, ex. « TrueHD Atmos · 7.1 ».
    pub fn summary(&self) -> String {
        let mut parts = vec![self.codec().to_string()];
        match self.get(attr::AUDIO_LAYOUT) {
            "" => {}
            l => parts.push(l.to_string()),
        }
        if self.is_commentary() {
            parts.push("commentaire".into());
        }
        if self.is_visually_impaired() {
            parts.push("audiodescription".into());
        }
        if self.is_derived_core() {
            parts.push("cœur dérivé".into());
        }
        parts.retain(|p| !p.is_empty());
        parts.join(" · ")
    }
}

/// Un titre (playlist) du disque.
#[derive(Debug, Clone, Default)]
pub struct Title {
    pub index: usize,
    pub attrs: BTreeMap<u32, String>,
    pub streams: Vec<Stream>,
}

impl Title {
    fn get(&self, id: u32) -> &str {
        self.attrs.get(&id).map(String::as_str).unwrap_or("")
    }
    pub fn duration_secs(&self) -> u64 {
        parse_duration(self.get(attr::DURATION))
    }
    pub fn chapters(&self) -> u32 {
        self.get(attr::CHAPTER_COUNT).parse().unwrap_or(0)
    }
    pub fn size_bytes(&self) -> u64 {
        self.get(attr::DISK_SIZE_BYTES).parse().unwrap_or(0)
    }
    pub fn size_label(&self) -> &str {
        self.get(attr::DISK_SIZE)
    }
    pub fn source(&self) -> &str {
        self.get(attr::SOURCE_FILE)
    }

    /// Segments lus dans l'ordre croissant ?
    ///
    /// Les Blu-ray « protégés » (Disney, Lionsgate…) contiennent des dizaines
    /// de playlists de même durée qui enchaînent les segments dans le désordre ;
    /// seule la vraie version du film les lit dans l'ordre. Un plan inconnu
    /// compte comme ordonné.
    pub fn segments_in_order(&self) -> bool {
        let mut seq: Vec<u64> = Vec::new();
        for part in self.get(attr::SEGMENTS_MAP).split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('-') {
                Some((a, b)) => match (a.trim().parse::<u64>(), b.trim().parse::<u64>()) {
                    (Ok(a), Ok(b)) if a <= b => seq.extend([a, b]),
                    (Ok(_), Ok(_)) => return false,
                    _ => return true,
                },
                None => match part.parse() {
                    Ok(n) => seq.push(n),
                    Err(_) => return true,
                },
            }
        }
        seq.windows(2).all(|w| w[0] < w[1])
    }
    pub fn streams_of(&self, kind: StreamKind) -> impl Iterator<Item = &Stream> {
        self.streams.iter().filter(move |s| s.kind() == kind)
    }
}

/// Informations complètes du disque.
#[derive(Debug, Clone, Default)]
pub struct DiscInfo {
    pub device: String,
    pub attrs: BTreeMap<u32, String>,
    pub titles: Vec<Title>,
}

impl DiscInfo {
    /// Construit le modèle à partir des lignes robot accumulées.
    pub fn from_lines<'a>(lines: impl IntoIterator<Item = &'a RobotLine>) -> Self {
        let mut d = DiscInfo::default();
        for l in lines {
            match l {
                RobotLine::DiscAttr { id, value } => {
                    d.attrs.insert(*id, value.clone());
                }
                RobotLine::TitleAttr { title, id, value } => {
                    d.title_mut(*title).attrs.insert(*id, value.clone());
                }
                RobotLine::StreamAttr { title, stream, id, value } => {
                    let t = d.title_mut(*title);
                    while t.streams.len() <= *stream {
                        let index = t.streams.len();
                        t.streams.push(Stream { index, ..Default::default() });
                    }
                    t.streams[*stream].attrs.insert(*id, value.clone());
                }
                _ => {}
            }
        }
        d
    }

    fn title_mut(&mut self, idx: usize) -> &mut Title {
        while self.titles.len() <= idx {
            let index = self.titles.len();
            self.titles.push(Title { index, ..Default::default() });
        }
        &mut self.titles[idx]
    }

    /// Nom du disque (CINFO 2), à défaut nom de volume (CINFO 32).
    pub fn name(&self) -> String {
        for id in [attr::NAME, attr::VOLUME_NAME] {
            if let Some(v) = self.attrs.get(&id).filter(|v| !v.trim().is_empty()) {
                return v.clone();
            }
        }
        "Disque sans nom".into()
    }

    pub fn volume_name(&self) -> &str {
        self.attrs.get(&attr::VOLUME_NAME).map(String::as_str).unwrap_or("")
    }

    /// Détermine DVD / Blu-ray / UHD.
    ///
    /// CINFO 1 donne « DVD disc » ou « Blu-ray disc » ; MakeMKV ne distingue
    /// pas explicitement l'UHD, on le déduit donc d'un flux vidéo 2160p.
    pub fn media_type(&self) -> MediaType {
        let t = self.attrs.get(&attr::TYPE).map(|s| s.to_lowercase()).unwrap_or_default();
        let has_2160 = self
            .titles
            .iter()
            .flat_map(|t| t.streams_of(StreamKind::Video))
            .any(|s| s.video_size().contains("2160"));
        if t.contains("dvd") {
            MediaType::Dvd
        } else if has_2160 {
            MediaType::UhdBluRay
        } else if t.contains("blu") {
            MediaType::BluRay
        } else {
            MediaType::Unknown
        }
    }

    /// Titre principal sans autre information : voir [`Self::main_title_for`].
    pub fn main_title(&self) -> Option<usize> {
        self.main_title_for(None)
    }

    /// Titre principal, en s'aidant si possible de la durée du film (TMDB).
    ///
    /// Par ordre de priorité :
    /// 1. une durée proche de celle du film (± 12 % : marge pour les génériques
    ///    et l'accélération PAL de 4 % des DVD), ce qui écarte les « lecture
    ///    intégrale » et les bonus enchaînés ;
    /// 2. des segments lus dans l'ordre (écarte les fausses playlists) ;
    /// 3. la durée la plus longue, puis le plus de chapitres, puis la taille.
    pub fn main_title_for(&self, runtime_secs: Option<u64>) -> Option<usize> {
        let near = |t: &Title| match runtime_secs {
            Some(r) if r > 0 => t.duration_secs().abs_diff(r) * 100 <= r * 12,
            _ => true,
        };
        self.titles
            .iter()
            .max_by_key(|t| (near(t), t.segments_in_order(), t.duration_secs(), t.chapters(), t.size_bytes()))
            .map(|t| t.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::makemkv::robot::parse_line;

    pub(crate) fn sample() -> DiscInfo {
        let raw = r#"CINFO:1,6209,"Blu-ray disc"
CINFO:2,0,"LE_GRAND_FILM_2019"
CINFO:32,0,"LE_GRAND_FILM"
TCOUNT:2
TINFO:0,9,0,"0:21:10"
TINFO:1,9,0,"2:05:42"
TINFO:1,8,0,"24"
SINFO:1,0,1,6201,"Video"
SINFO:1,0,19,0,"1920x1080"
SINFO:1,1,1,6202,"Audio"
SINFO:1,1,3,0,"fre"
SINFO:1,1,4,0,"French"
SINFO:1,1,6,0,"DTS-HD MA"
SINFO:1,2,1,6202,"Audio"
SINFO:1,2,3,0,"eng"
SINFO:1,3,1,6203,"Subtitles"
SINFO:1,3,3,0,"fre"
SINFO:1,4,1,6203,"Subtitles"
SINFO:1,4,3,0,"fre"
SINFO:1,4,22,0,"6144"
SINFO:1,5,1,6203,"Subtitles"
SINFO:1,5,3,0,"eng""#;
        let lines: Vec<_> = raw.lines().map(parse_line).collect();
        DiscInfo::from_lines(&lines)
    }

    #[test]
    fn main_title_uses_runtime_and_segment_order() {
        let raw = r#"TINFO:0,9,0,"3:10:00"
TINFO:0,26,0,"1-40"
TINFO:1,9,0,"1:58:30"
TINFO:1,26,0,"1,5,3,2,4"
TINFO:2,9,0,"1:58:30"
TINFO:2,26,0,"1-5"
TINFO:3,9,0,"1:58:30"
TINFO:3,26,0,"2,1,3-5""#;
        let lines: Vec<_> = raw.lines().map(parse_line).collect();
        let d = DiscInfo::from_lines(&lines);
        // Sans durée : la « lecture intégrale » de 3 h 10 l'emporte.
        assert_eq!(d.main_title(), Some(0));
        // Film de 2 h 02 : parmi les titres proches, celui aux segments ordonnés.
        assert_eq!(d.main_title_for(Some(122 * 60)), Some(2));
        assert!(!d.titles[3].segments_in_order());
    }

    #[test]
    fn builds_model() {
        let d = sample();
        assert_eq!(d.titles.len(), 2);
        assert_eq!(d.main_title(), Some(1));
        assert_eq!(d.media_type(), MediaType::BluRay);
        let t = &d.titles[1];
        assert_eq!(t.streams.len(), 6);
        assert_eq!(t.streams[1].lang(), "fra");
        assert!(t.streams[4].is_forced());
        assert!(!t.streams[3].is_forced());
    }
}
