//! Métadonnées du film et nommage du fichier de sortie.
//!
//! 1. Le nom du disque (`LE_GRAND_FILM_2019_DISC1`) est nettoyé → « Le Grand Film », 2019.
//! 2. Si une clé TMDB est configurée, on recherche le film : titre localisé,
//!    année, identifiant TMDB, résumé et affiche.
//! 3. Les caractéristiques techniques sont relevées sur le disque (définition,
//!    codecs, canaux, langues) pour composer le nom final, ex. :
//!    `Sympathy for Mister Vengeance (2002) [tmdbid-4689] - 576i MPEG2 AC3 5.1 MULTi.mkv`

use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;

use crate::makemkv::disc::{MediaType, Stream, StreamKind, Title};
use crate::selection::{normalize_lang, Selection};

/// Mots parasites fréquents dans les noms de volume.
const NOISE: &[&str] = &[
    "DISC", "DISK", "D1", "D2", "DISC1", "DISC2", "BLURAY", "BLU", "RAY", "BD", "BD25", "BD50", "BD66", "UHD", "4K",
    "DVD", "DVD5", "DVD9", "WS", "FS", "16X9", "NTSC", "PAL", "SE", "CE", "HD", "FR", "FRA", "FRENCH", "ZONE", "REGION",
];

/// Nettoie un nom de volume ; renvoie (titre lisible, année éventuelle).
pub fn clean_disc_name(raw: &str) -> (String, Option<u32>) {
    let year_re = Regex::new(r"^(19[0-9]{2}|20[0-9]{2})$").unwrap();
    let disc_re = Regex::new(r"^(DISC|DISK|D)\d+$").unwrap();
    let mut year = None;
    let words: Vec<String> = raw
        .split(|c: char| c == '_' || c == '.' || c == '-' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .filter(|w| {
            if year_re.is_match(w) {
                year = w.parse().ok();
                return false;
            }
            let up = w.to_uppercase();
            // « DISC1 », « D2 »… et mots parasites.
            !(NOISE.contains(&up.as_str()) || disc_re.is_match(&up))
        })
        .map(|w| capitalize(&w.to_lowercase()))
        .collect();
    let title = if words.is_empty() { raw.trim().to_string() } else { words.join(" ") };
    (title, year)
}

fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

// --- Caractéristiques techniques -------------------------------------------

/// Définition : « 576i », « 1080p », « 2160p ».
///
/// MakeMKV ne dit pas si la vidéo est entrelacée : les DVD le sont (quasi)
/// toujours, les Blu-ray sont en balayage progressif dans l'immense majorité.
fn resolution_tag(media: MediaType, video: &Stream) -> Option<String> {
    let h: u32 = video.video_size().split('x').nth(1)?.trim().parse().ok()?;
    let scan = if media == MediaType::Dvd { 'i' } else { 'p' };
    Some(format!("{h}{scan}"))
}

fn video_codec_tag(video: &Stream) -> String {
    let id = video.codec_id();
    let tag = match id {
        "V_MPEG2" => "MPEG2",
        "V_MPEG1" => "MPEG1",
        "V_MPEG4/ISO/AVC" => "H264",
        "V_MPEGH/ISO/HEVC" => "HEVC",
        "V_MS/VFW/WVC1" | "V_MS/VFW/FOURCC" => "VC-1",
        _ => "",
    };
    if !tag.is_empty() {
        return tag.into();
    }
    // Repli sur le libellé court (« Mpeg2 », « MPEG-4 AVC »…).
    let c = video.codec().to_uppercase().replace([' ', '-'], "");
    if c.contains("HEVC") || c.contains("H265") {
        "HEVC".into()
    } else if c.contains("AVC") || c.contains("H264") {
        "H264".into()
    } else if c.contains("VC1") {
        "VC-1".into()
    } else {
        c
    }
}

fn audio_codec_tag(a: &Stream) -> String {
    let short = a.codec().to_uppercase();
    match a.codec_id() {
        "A_AC3" => "AC3".into(),
        "A_EAC3" => "EAC3".into(),
        "A_TRUEHD" if short.contains("ATMOS") => "TrueHD Atmos".into(),
        "A_TRUEHD" => "TrueHD".into(),
        "A_DTS" if short.contains("MA") => "DTS-HD MA".into(),
        "A_DTS" if short.contains("HR") || short.contains("HI-RES") => "DTS-HD HR".into(),
        "A_DTS" if short.contains("X") => "DTS-X".into(),
        "A_DTS" => "DTS".into(),
        "A_LPCM" | "A_PCM/INT/LIT" | "A_PCM/INT/BIG" => "LPCM".into(),
        "A_FLAC" => "FLAC".into(),
        "A_AAC" => "AAC".into(),
        "A_MPEG/L2" => "MP2".into(),
        "A_MPEG/L3" => "MP3".into(),
        _ => a.codec().replace(' ', ""),
    }
}

/// « 5.1 », « 2.0 », « 7.1 »…
fn channels_tag(a: &Stream) -> Option<String> {
    let layout = a.audio_layout();
    let re = Regex::new(r"^(\d\.\d)").unwrap();
    if let Some(c) = re.captures(layout) {
        return Some(c[1].to_string());
    }
    match a.channels() {
        0 => None,
        1 => Some("1.0".into()),
        2 => Some("2.0".into()),
        n @ 3..=5 => Some(format!("{}.0", n)),
        n => Some(format!("{}.1", n - 1)),
    }
}

/// Étiquette de langue : MULTi si plusieurs langues audio, sinon FRENCH,
/// VOSTFR, ENGLISH…
fn language_tag(title: &Title, sel: &Selection, pref: &str) -> Option<String> {
    let audio: Vec<&Stream> = title
        .streams_of(StreamKind::Audio)
        .filter(|s| sel.audio.contains(&s.index) && !s.is_commentary() && !s.is_visually_impaired())
        .collect();
    let mut langs: Vec<String> = audio.iter().map(|s| s.lang()).filter(|l| !l.is_empty() && l != "und").collect();
    langs.sort();
    langs.dedup();
    match langs.as_slice() {
        [] => None,
        [one] if one == pref => Some(english_lang_name(one).to_uppercase()),
        [one] => {
            let subs_in_pref = title
                .streams_of(StreamKind::Subtitle)
                .any(|s| sel.subtitles.contains(&s.index) && s.lang() == pref && !s.is_forced());
            if subs_in_pref && pref == "fra" {
                Some("VOSTFR".into())
            } else if subs_in_pref {
                Some(format!("VOST{}", pref.get(..2).unwrap_or("").to_uppercase()))
            } else {
                Some(english_lang_name(one).to_uppercase())
            }
        }
        _ => Some("MULTi".into()),
    }
}

fn english_lang_name(code: &str) -> &str {
    match code {
        "fra" => "French",
        "eng" => "English",
        "deu" => "German",
        "spa" => "Spanish",
        "ita" => "Italian",
        "por" => "Portuguese",
        "nld" => "Dutch",
        "jpn" => "Japanese",
        "kor" => "Korean",
        "zho" => "Chinese",
        "rus" => "Russian",
        other => other,
    }
}

/// Caractéristiques du média : « 576i MPEG2 AC3 5.1 MULTi ».
///
/// L'audio décrit est la première piste retenue dans la langue préférée
/// (à défaut la première piste retenue).
pub fn media_tags(media: MediaType, title: &Title, sel: &Selection, pref_lang: &str) -> String {
    let pref = normalize_lang(pref_lang);
    let mut parts = Vec::new();
    if let Some(v) = title.streams_of(StreamKind::Video).next() {
        parts.extend(resolution_tag(media, v));
        parts.push(video_codec_tag(v));
    }
    let chosen: Vec<&Stream> = title
        .streams_of(StreamKind::Audio)
        .filter(|s| sel.audio.contains(&s.index) && !s.is_commentary())
        .collect();
    if let Some(a) = chosen.iter().find(|s| s.lang() == pref).or(chosen.first()) {
        parts.push(audio_codec_tag(a));
        parts.extend(channels_tag(a));
    }
    parts.extend(language_tag(title, sel, &pref));
    parts.retain(|p| !p.is_empty());
    parts.join(" ")
}

// --- Nom de fichier ----------------------------------------------------------

/// Retire les caractères interdits ou gênants dans un nom de fichier.
fn sanitize(s: &str) -> String {
    let s = s.replace(" : ", " - ").replace(": ", " - ").replace(':', "-");
    let s: String = s.chars().filter(|c| !matches!(c, '/' | '\\' | '?' | '*' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    let s = Regex::new(r"\s+").unwrap().replace_all(s.trim(), " ").into_owned();
    s.trim_end_matches('.').to_string()
}

/// Nom de fichier final :
/// `Titre (Année) [tmdbid-123] - 576i MPEG2 AC3 5.1 MULTi.mkv`.
/// Les éléments inconnus sont simplement omis.
pub fn output_filename(title: &str, year: Option<u32>, tmdb_id: Option<u64>, tags: &str) -> String {
    let mut name = sanitize(title);
    if name.is_empty() {
        name = "Film".into();
    }
    if let Some(y) = year {
        name.push_str(&format!(" ({y})"));
    }
    if let Some(id) = tmdb_id {
        name.push_str(&format!(" [tmdbid-{id}]"));
    }
    let tags = sanitize(tags);
    if !tags.is_empty() {
        name.push_str(" - ");
        name.push_str(&tags);
    }
    format!("{name}.mkv")
}

// --- TMDB --------------------------------------------------------------------

/// Code de langue TMDB à partir d'un code ISO 639-2.
fn tmdb_language(lang: &str) -> &'static str {
    match normalize_lang(lang).as_str() {
        "fra" => "fr-FR",
        "deu" => "de-DE",
        "spa" => "es-ES",
        "ita" => "it-IT",
        "por" => "pt-PT",
        "nld" => "nl-NL",
        "jpn" => "ja-JP",
        _ => "en-US",
    }
}

/// Un film TMDB.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Movie {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub original_title: String,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub poster_path: Option<String>,
    /// Durée en minutes (fiche détaillée uniquement, absente des recherches).
    #[serde(default)]
    pub runtime: Option<u32>,
    /// Image de fond (scène du film).
    #[serde(default)]
    pub backdrop_path: Option<String>,
    /// Note moyenne sur 10.
    #[serde(default)]
    pub vote_average: Option<f64>,
    /// Genres (fiche détaillée uniquement).
    #[serde(default)]
    pub genres: Vec<Named>,
    /// Distribution et équipe (fiche détaillée, `append_to_response=credits`).
    #[serde(default)]
    pub credits: Option<Credits>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Named {
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Credits {
    #[serde(default)]
    pub cast: Vec<Named>,
    #[serde(default)]
    pub crew: Vec<CrewMember>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct CrewMember {
    pub name: String,
    #[serde(default)]
    pub job: String,
}

impl Movie {
    pub fn year(&self) -> Option<u32> {
        self.release_date.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok())
    }

    /// Réalisateur(s).
    pub fn directors(&self) -> Vec<String> {
        self.credits.iter().flat_map(|c| &c.crew).filter(|p| p.job == "Director").map(|p| p.name.clone()).collect()
    }

    /// Premiers rôles.
    pub fn top_cast(&self, n: usize) -> Vec<String> {
        self.credits.iter().flat_map(|c| &c.cast).take(n).map(|p| p.name.clone()).collect()
    }

    /// Note « 7,1 » (absente si aucun vote).
    pub fn rating_label(&self) -> Option<String> {
        self.vote_average.filter(|v| *v > 0.0).map(|v| format!("{v:.1}").replace('.', ","))
    }
}

#[derive(Deserialize)]
struct SearchResponse {
    results: Vec<Movie>,
}

fn tmdb_get(http: &reqwest::Client, api_key: &str, path: &str) -> reqwest::RequestBuilder {
    let req = http.get(format!("https://api.themoviedb.org/3{path}"));
    // Clé v4 (jeton JWT « eyJ… ») → en-tête Bearer ; clé v3 → paramètre api_key.
    if api_key.starts_with("eyJ") {
        req.bearer_auth(api_key)
    } else {
        req.query(&[("api_key", api_key)])
    }
}

/// Recherche des films sur TMDB (meilleurs résultats en premier).
pub async fn tmdb_search(http: &reqwest::Client, api_key: &str, query: &str, year: Option<u32>, lang: &str) -> Result<Vec<Movie>> {
    // On tente d'abord avec l'année (plus précis), puis sans.
    let attempts: Vec<Option<u32>> = if year.is_some() { vec![year, None] } else { vec![None] };
    for y in attempts {
        let mut req = tmdb_get(http, api_key, "/search/movie").query(&[
            ("query", query),
            ("language", tmdb_language(lang)),
            ("include_adult", "false"),
        ]);
        if let Some(y) = y {
            req = req.query(&[("year", y.to_string())]);
        }
        let resp: SearchResponse = req.send().await?.error_for_status().context("requête TMDB refusée")?.json().await?;
        if !resp.results.is_empty() {
            return Ok(resp.results.into_iter().take(12).collect());
        }
    }
    Ok(Vec::new())
}

/// Fiche d'un film à partir de son identifiant TMDB.
pub async fn tmdb_movie(http: &reqwest::Client, api_key: &str, id: u64, lang: &str) -> Result<Movie> {
    tmdb_get(http, api_key, &format!("/movie/{id}"))
        .query(&[("language", tmdb_language(lang)), ("append_to_response", "credits")])
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("film TMDB {id} introuvable"))?
        .json()
        .await
        .context("réponse TMDB illisible")
}

/// Vérifie qu'une clé est acceptée par TMDB.
pub async fn tmdb_check(http: &reqwest::Client, api_key: &str) -> Result<()> {
    let resp = tmdb_get(http, api_key.trim(), "/configuration").send().await.context("TMDB injoignable")?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        anyhow::bail!("clé refusée par TMDB");
    }
    resp.error_for_status().context("réponse inattendue de TMDB")?;
    Ok(())
}

/// Ressemble à une clé TMDB : clé v3 (32 caractères hexadécimaux) ou jeton
/// de lecture v4 (JWT « eyJ… »).
pub fn looks_like_tmdb_key(s: &str) -> bool {
    let s = s.trim();
    (s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit()))
        || (s.starts_with("eyJ") && s.len() > 100 && s.split('.').count() == 3 && !s.contains(char::is_whitespace))
}

/// Identifiant TMDB saisi directement : « 4689 », « tmdbid-4689 » ou une URL
/// `https://www.themoviedb.org/movie/4689-…`.
pub fn parse_tmdb_id(input: &str) -> Option<u64> {
    let s = input.trim();
    if let Ok(id) = s.parse() {
        return Some(id);
    }
    let re = Regex::new(r"(?:themoviedb\.org/movie/|tmdbid-)(\d+)").unwrap();
    re.captures(s).and_then(|c| c[1].parse().ok())
}

/// Télécharge une affiche (`size` : w92, w185, w342, w500…).
pub async fn fetch_poster(http: &reqwest::Client, poster_path: &str, size: &str) -> Result<Vec<u8>> {
    let url = format!("https://image.tmdb.org/t/p/{size}{poster_path}");
    let bytes = http.get(url).send().await?.error_for_status()?.bytes().await?;
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::makemkv::disc::DiscInfo;
    use crate::makemkv::robot::parse_line;

    #[test]
    fn parses_movie_details() {
        let json = r#"{"id":68924,"title":"Ice Storm","release_date":"1997-09-27","runtime":112,
            "vote_average":7.06,"backdrop_path":"/b.jpg","genres":[{"id":18,"name":"Drame"}],
            "credits":{"cast":[{"name":"Kevin Kline"},{"name":"Joan Allen"}],
                       "crew":[{"name":"Ang Lee","job":"Director"},{"name":"X","job":"Editor"}]}}"#;
        let m: Movie = serde_json::from_str(json).unwrap();
        assert_eq!(m.directors(), vec!["Ang Lee"]);
        assert_eq!(m.top_cast(1), vec!["Kevin Kline"]);
        assert_eq!(m.rating_label().as_deref(), Some("7,1"));
        assert_eq!(m.genres[0].name, "Drame");
    }

    #[test]
    fn recognizes_tmdb_keys() {
        assert!(looks_like_tmdb_key(" 0123456789abcdef0123456789ABCDEF\n"));
        assert!(!looks_like_tmdb_key("0123456789abcdef"));
        let jwt = format!("eyJhbGciOiJIUzI1NiJ9.{}.sig", "a".repeat(120));
        assert!(looks_like_tmdb_key(&jwt));
        assert!(!looks_like_tmdb_key("Bonjour tout le monde"));
    }

    #[test]
    fn cleans_volume_names() {
        assert_eq!(clean_disc_name("LE_GRAND_FILM_2019"), ("Le Grand Film".into(), Some(2019)));
        assert_eq!(clean_disc_name("THE_MATRIX_DISC1_BLURAY"), ("The Matrix".into(), None));
        assert_eq!(clean_disc_name("INCEPTION_UHD_D1"), ("Inception".into(), None));
    }

    #[test]
    fn builds_filenames() {
        assert_eq!(
            output_filename("Sympathy for Mister Vengeance", Some(2002), Some(4689), "576i MPEG2 AC3 5.1 MULTi"),
            "Sympathy for Mister Vengeance (2002) [tmdbid-4689] - 576i MPEG2 AC3 5.1 MULTi.mkv"
        );
        assert_eq!(output_filename("Alien : le 8e passager", None, None, ""), "Alien - le 8e passager.mkv");
        assert_eq!(output_filename("AC/DC", Some(1991), None, ""), "ACDC (1991).mkv");
        assert_eq!(output_filename("", None, None, ""), "Film.mkv");
    }

    #[test]
    fn parses_tmdb_ids() {
        assert_eq!(parse_tmdb_id("4689"), Some(4689));
        assert_eq!(parse_tmdb_id("https://www.themoviedb.org/movie/4689-boksuneun-naui-geot"), Some(4689));
        assert_eq!(parse_tmdb_id("[tmdbid-4689]"), Some(4689));
        assert_eq!(parse_tmdb_id("Old Boy"), None);
    }

    /// Données réelles du DVD « 88 Minutes » (MakeMKV 2.0, interface en français).
    fn dvd() -> DiscInfo {
        let raw = r#"CINFO:1,6206,"Disque DVD"
TINFO:0,9,0,"1:46:31"
SINFO:0,0,1,6201,"Vidéo"
SINFO:0,0,5,0,"V_MPEG2"
SINFO:0,0,6,0,"Mpeg2"
SINFO:0,0,19,0,"720x576"
SINFO:0,1,1,6202,"Audio"
SINFO:0,1,3,0,"fre"
SINFO:0,1,5,0,"A_AC3"
SINFO:0,1,6,0,"DD"
SINFO:0,1,14,0,"6"
SINFO:0,1,40,0,"5.1(side)"
SINFO:0,2,1,6202,"Audio"
SINFO:0,2,3,0,"eng"
SINFO:0,2,5,0,"A_AC3"
SINFO:0,2,6,0,"DD"
SINFO:0,2,14,0,"6"
SINFO:0,3,1,6203,"Sous-titres"
SINFO:0,3,3,0,"fre"
SINFO:0,3,5,0,"S_VOBSUB""#;
        let lines: Vec<_> = raw.lines().map(parse_line).collect();
        DiscInfo::from_lines(&lines)
    }

    #[test]
    fn french_stream_types_are_recognized() {
        let d = dvd();
        let t = &d.titles[0];
        assert_eq!(t.streams_of(StreamKind::Video).count(), 1);
        assert_eq!(t.streams_of(StreamKind::Subtitle).count(), 1);
    }

    #[test]
    fn builds_media_tags() {
        let d = dvd();
        let t = &d.titles[0];
        let multi = Selection { audio: [1, 2].into(), subtitles: Default::default() };
        assert_eq!(media_tags(d.media_type(), t, &multi, "fra"), "576i MPEG2 AC3 5.1 MULTi");
        let fr = Selection { audio: [1].into(), subtitles: Default::default() };
        assert_eq!(media_tags(d.media_type(), t, &fr, "fra"), "576i MPEG2 AC3 5.1 FRENCH");
        let vostfr = Selection { audio: [2].into(), subtitles: [3].into() };
        assert_eq!(media_tags(d.media_type(), t, &vostfr, "fra"), "576i MPEG2 AC3 5.1 VOSTFR");
    }
}
