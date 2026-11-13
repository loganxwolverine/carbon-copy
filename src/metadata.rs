//! Métadonnées du film et nommage du fichier de sortie.
//!
//! 1. Le nom du disque (`LE_GRAND_FILM_2019_DISC1`) est nettoyé → « Le Grand Film », 2019.
//! 2. Si une clé TMDB est configurée, on recherche le film pour obtenir le titre
//!    officiel localisé et l'année de sortie.
//! 3. Le nom de fichier est normalisé : `Le_Grand_Film_2019.mkv`.

use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;

/// Mots parasites fréquents dans les noms de volume.
const NOISE: &[&str] = &[
    "DISC", "DISK", "D1", "D2", "DISC1", "DISC2", "BLURAY", "BLU", "RAY", "BD", "BD25", "BD50", "BD66", "UHD", "4K",
    "DVD", "DVD5", "DVD9", "WS", "FS", "16X9", "NTSC", "PAL", "SE", "CE", "HD", "FR", "FRA", "FRENCH", "ZONE", "REGION",
];

/// Nettoie un nom de volume ; renvoie (titre lisible, année éventuelle).
pub fn clean_disc_name(raw: &str) -> (String, Option<u32>) {
    let year_re = Regex::new(r"^(19[0-9]{2}|20[0-9]{2})$").unwrap();
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
            !(NOISE.contains(&up.as_str()) || Regex::new(r"^(DISC|DISK|D)\d+$").unwrap().is_match(&up))
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

/// Nom de fichier normalisé : ASCII, mots capitalisés séparés par `_`.
pub fn output_filename(title: &str, year: Option<u32>) -> String {
    let ascii = deunicode::deunicode(title);
    let mut parts: Vec<String> = ascii
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(capitalize)
        .collect();
    if parts.is_empty() {
        parts.push("Film".into());
    }
    if let Some(y) = year {
        parts.push(y.to_string());
    }
    format!("{}.mkv", parts.join("_"))
}

/// Code de langue TMDB à partir d'un code ISO 639-2.
fn tmdb_language(lang: &str) -> &'static str {
    match crate::selection::normalize_lang(lang).as_str() {
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

#[derive(Deserialize)]
struct SearchResponse {
    results: Vec<Movie>,
}

#[derive(Deserialize)]
struct Movie {
    title: String,
    #[serde(default)]
    release_date: Option<String>,
}

/// Recherche un film sur TMDB ; renvoie (titre localisé, année).
pub async fn tmdb_search(
    http: &reqwest::Client,
    api_key: &str,
    query: &str,
    year: Option<u32>,
    lang: &str,
) -> Result<Option<(String, Option<u32>)>> {
    // On tente d'abord avec l'année (plus précis), puis sans.
    let attempts: Vec<Option<u32>> = if year.is_some() { vec![year, None] } else { vec![None] };
    for y in attempts {
        let mut req = http.get("https://api.themoviedb.org/3/search/movie").query(&[
            ("query", query),
            ("language", tmdb_language(lang)),
            ("include_adult", "false"),
        ]);
        if let Some(y) = y {
            req = req.query(&[("year", y.to_string())]);
        }
        // Clé v4 (jeton JWT « eyJ… ») → en-tête Bearer ; clé v3 → paramètre api_key.
        req = if api_key.starts_with("eyJ") {
            req.bearer_auth(api_key)
        } else {
            req.query(&[("api_key", api_key)])
        };
        let resp: SearchResponse =
            req.send().await?.error_for_status().context("requête TMDB refusée")?.json().await?;
        if let Some(m) = resp.results.into_iter().next() {
            let year = m.release_date.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok());
            return Ok(Some((m.title, year)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_volume_names() {
        assert_eq!(clean_disc_name("LE_GRAND_FILM_2019"), ("Le Grand Film".into(), Some(2019)));
        assert_eq!(clean_disc_name("THE_MATRIX_DISC1_BLURAY"), ("The Matrix".into(), None));
        assert_eq!(clean_disc_name("INCEPTION_UHD_D1"), ("Inception".into(), None));
    }

    #[test]
    fn builds_filenames() {
        assert_eq!(output_filename("Le Fabuleux Destin d'Amélie Poulain", Some(2001)), "Le_Fabuleux_Destin_D_Amelie_Poulain_2001.mkv");
        assert_eq!(output_filename("Alien : le 8e passager", None), "Alien_Le_8e_Passager.mkv");
        assert_eq!(output_filename("", None), "Film.mkv");
    }
}
