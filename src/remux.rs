//! Finalisation du MKV produit par MakeMKV.
//!
//! MakeMKV extrait les pistes décrites par le plan (voir `makemkv::rule`).
//! On identifie les pistes du fichier avec `mkvmerge -J`, on vérifie qu'elles
//! correspondent au plan, puis :
//! - si le fichier contient exactement la sélection, `mkvpropedit` pose le
//!   titre et les drapeaux (`default`, `forced`) EN PLACE, en quelques secondes ;
//! - sinon (pistes en trop), `mkvmerge` réécrit un fichier avec uniquement la
//!   vidéo principale, l'audio et les sous-titres cochés.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::events::{log, Tx};
use crate::makemkv::disc::{Stream, StreamKind, Title};
use crate::process::{run_capture, run_streaming, Tool};
use crate::selection::{normalize_lang, Selection};

#[derive(Debug, Deserialize)]
struct Identify {
    tracks: Vec<MkvTrack>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MkvTrack {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub properties: TrackProps,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TrackProps {
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub forced_track: Option<bool>,
    /// Numéro de piste Matroska (sélecteur `track:@N` de mkvpropedit).
    #[serde(default)]
    pub number: Option<u64>,
}

impl MkvTrack {
    fn stream_kind(&self) -> StreamKind {
        match self.kind.as_str() {
            "video" => StreamKind::Video,
            "audio" => StreamKind::Audio,
            "subtitles" => StreamKind::Subtitle,
            _ => StreamKind::Other,
        }
    }
    fn lang(&self) -> String {
        normalize_lang(self.properties.language.as_deref().unwrap_or("und"))
    }
}

pub async fn identify(mkvmerge: &Tool, file: &Path) -> Result<Vec<MkvTrack>> {
    let dir = file.parent().unwrap_or(Path::new("/"));
    let mut c = mkvmerge.command(&[dir]);
    c.arg("-J").arg(file);
    let (st, out) = run_capture(c).await?;
    if !st.success() {
        bail!("mkvmerge -J a échoué : {out}");
    }
    // La sortie JSON peut être précédée d'avertissements Flatpak sur stderr :
    // on part de la première accolade.
    let json = out.find('{').map(|i| &out[i..]).unwrap_or(&out);
    let id: Identify = serde_json::from_str(json).context("JSON mkvmerge illisible")?;
    Ok(id.tracks)
}

/// Flux du disque présents dans le fichier extrait, pour un type donné.
fn extracted<'a>(title: &'a Title, contents: &'a Selection, kind: StreamKind) -> Vec<&'a Stream> {
    let set: &BTreeSet<usize> = match kind {
        StreamKind::Audio => &contents.audio,
        StreamKind::Subtitle => &contents.subtitles,
        _ => return title.streams_of(kind).collect(),
    };
    title.streams_of(kind).filter(|s| set.contains(&s.index)).collect()
}

/// Langues identiques deux à deux (une langue inconnue d'un côté est tolérée).
fn same_langs(disc: &[&Stream], file: &[&MkvTrack]) -> bool {
    disc.len() == file.len()
        && disc.iter().zip(file).all(|(s, t)| {
            let (a, b) = (s.lang(), t.lang());
            a == b || a.is_empty() || a == "und" || b == "und"
        })
}

/// Le fichier contient-il exactement les pistes audio / sous-titres prévues,
/// dans l'ordre du disque ?
pub fn matches_contents(title: &Title, contents: &Selection, mkv: &[MkvTrack]) -> bool {
    [StreamKind::Audio, StreamKind::Subtitle].into_iter().all(|k| {
        let file: Vec<_> = mkv.iter().filter(|t| t.stream_kind() == k).collect();
        same_langs(&extracted(title, contents, k), &file)
    })
}

/// Fait correspondre chaque flux sélectionné à une piste du fichier.
///
/// `contents` : flux présents dans le fichier. MakeMKV les écrit dans l'ordre
/// du disque : on mappe donc par rang au sein de chaque type. Si les effectifs
/// ou les langues diffèrent (cas imprévu), on se rabat sur une correspondance
/// par langue (+ drapeau forcé pour les sous-titres), dans l'ordre.
pub fn map_tracks<'a>(
    title: &Title,
    contents: &Selection,
    mkv: &'a [MkvTrack],
    kind: StreamKind,
    wanted: &[usize],
) -> Vec<(usize, &'a MkvTrack)> {
    let disc = extracted(title, contents, kind);
    let file: Vec<&MkvTrack> = mkv.iter().filter(|t| t.stream_kind() == kind).collect();
    let mut out = Vec::new();

    if same_langs(&disc, &file) {
        for (s, t) in disc.iter().zip(&file) {
            if wanted.contains(&s.index) {
                out.push((s.index, *t));
            }
        }
        return out;
    }

    let mut used = std::collections::HashSet::new();
    for s in title.streams_of(kind).filter(|s| wanted.contains(&s.index)) {
        let forced = s.is_forced();
        let found = file.iter().find(|t| {
            !used.contains(&t.id)
                && t.lang() == s.lang()
                && (kind != StreamKind::Subtitle || t.properties.forced_track.unwrap_or(false) == forced)
        });
        if let Some(t) = found {
            used.insert(t.id);
            out.push((s.index, *t));
        }
    }
    out
}

/// Piste audio par défaut : la première dans la langue préférée (hors
/// commentaire), sinon la première sélectionnée.
pub fn default_audio(title: &Title, sel: &Selection, pref_lang: &str) -> Option<usize> {
    let lang = normalize_lang(pref_lang);
    let picked: Vec<&Stream> = title.streams_of(StreamKind::Audio).filter(|s| sel.audio.contains(&s.index)).collect();
    picked
        .iter()
        .find(|s| s.lang() == lang && !s.is_commentary())
        .or(picked.first())
        .map(|s| s.index)
}

/// Drapeaux des sous-titres : (forcé, par défaut). Les sous-titres forcés
/// sont aussi « par défaut » (le premier seulement) pour que les lecteurs
/// (Plex, Kodi, VLC…) les affichent d'office ; les complets jamais.
fn subtitle_flags(title: &Title, subs: &[usize]) -> Vec<(usize, bool, bool)> {
    let mut given = false;
    subs.iter()
        .map(|&i| {
            let forced = title.streams.get(i).is_some_and(|s| s.is_forced());
            let default = forced && !given;
            given |= default;
            (i, forced, default)
        })
        .collect()
}

/// Pose le titre et les drapeaux en place avec `mkvpropedit` (aucune réécriture).
pub async fn set_properties(
    mkvpropedit: &Tool,
    title: &Title,
    sel: &Selection,
    mkv: &[MkvTrack],
    file: &Path,
    movie_title: &str,
    pref_lang: &str,
    tx: &Tx,
) -> Result<()> {
    let audio_wanted: Vec<_> = sel.audio.iter().copied().collect();
    let subs_wanted: Vec<_> = sel.subtitles.iter().copied().collect();
    let audio = map_tracks(title, sel, mkv, StreamKind::Audio, &audio_wanted);
    let subs = map_tracks(title, sel, mkv, StreamKind::Subtitle, &subs_wanted);
    let def_audio = default_audio(title, sel, pref_lang);

    let num = |t: &MkvTrack| t.properties.number.map(|n| format!("track:@{n}"));
    let mut args: Vec<String> = vec![file.display().to_string(), "--edit".into(), "info".into(), "--set".into(), format!("title={movie_title}")];
    let mut edit = |sel: Option<String>, props: &[(&str, bool)]| -> Result<()> {
        let sel = sel.context("numéro de piste absent de la sortie de mkvmerge")?;
        args.extend(["--edit".into(), sel]);
        for (k, v) in props {
            args.extend(["--set".into(), format!("{k}={}", *v as u8)]);
        }
        Ok(())
    };
    if let Some(v) = mkv.iter().find(|t| t.stream_kind() == StreamKind::Video) {
        edit(num(v), &[("flag-default", true)])?;
    }
    for (idx, t) in &audio {
        edit(num(t), &[("flag-default", Some(*idx) == def_audio)])?;
    }
    let flags = subtitle_flags(title, &subs.iter().map(|(i, _)| *i).collect::<Vec<_>>());
    for ((_, t), (_, forced, default)) in subs.iter().zip(flags) {
        edit(num(t), &[("flag-forced", forced), ("flag-default", default)])?;
    }

    log(tx, format!("mkvpropedit {}", args.join(" ")));
    let dir = file.parent().unwrap_or(Path::new("/"));
    let mut cmd = mkvpropedit.command(&[dir]);
    cmd.args(&args);
    let (st, out) = run_capture(cmd).await?;
    // Code 1 = avertissements seulement.
    match st.code() {
        Some(0) | Some(1) => Ok(()),
        _ => bail!("mkvpropedit a échoué (code {st}) : {}", out.trim()),
    }
}

/// Remuxe `input` vers `output` en ne gardant que la sélection.
#[allow(clippy::too_many_arguments)]
pub async fn remux(
    mkvmerge: &Tool,
    title: &Title,
    sel: &Selection,
    contents: &Selection,
    input: &Path,
    output: &Path,
    movie_title: &str,
    pref_lang: &str,
    tx: &Tx,
    cancel: CancellationToken,
    on_progress: impl Fn(f64) + Send + Sync,
) -> Result<PathBuf> {
    let tracks = identify(mkvmerge, input).await?;
    let audio_wanted: Vec<_> = sel.audio.iter().copied().collect();
    let subs_wanted: Vec<_> = sel.subtitles.iter().copied().collect();

    let video = tracks.iter().find(|t| t.kind == "video").map(|t| t.id);
    let mut audio: Vec<(usize, u64)> =
        map_tracks(title, contents, &tracks, StreamKind::Audio, &audio_wanted).into_iter().map(|(i, t)| (i, t.id)).collect();
    let subs: Vec<(usize, u64)> =
        map_tracks(title, contents, &tracks, StreamKind::Subtitle, &subs_wanted).into_iter().map(|(i, t)| (i, t.id)).collect();

    if audio.len() != audio_wanted.len() || subs.len() != subs_wanted.len() {
        log(tx, "⚠ Certaines pistes sélectionnées n'ont pas été retrouvées dans le fichier extrait.");
    }
    // La piste audio par défaut passe en tête.
    if let Some(d) = default_audio(title, sel, pref_lang) {
        if let Some(pos) = audio.iter().position(|(i, _)| *i == d) {
            let a = audio.remove(pos);
            audio.insert(0, a);
        }
    }

    let mut args: Vec<String> = vec!["--gui-mode".into(), "-o".into(), output.display().to_string()];
    args.extend(["--title".into(), movie_title.to_string()]);

    let ids = |v: &[(usize, u64)]| v.iter().map(|(_, id)| id.to_string()).collect::<Vec<_>>().join(",");
    match video {
        Some(v) => args.extend(["--video-tracks".into(), v.to_string()]),
        None => args.push("--no-video".into()),
    }
    if audio.is_empty() {
        args.push("--no-audio".into());
    } else {
        args.extend(["--audio-tracks".into(), ids(&audio)]);
        for (i, (_, id)) in audio.iter().enumerate() {
            args.extend(["--default-track-flag".into(), format!("{id}:{}", (i == 0) as u8)]);
        }
    }
    if subs.is_empty() {
        args.push("--no-subtitles".into());
    } else {
        args.extend(["--subtitle-tracks".into(), ids(&subs)]);
        let flags = subtitle_flags(title, &subs.iter().map(|(i, _)| *i).collect::<Vec<_>>());
        for ((_, id), (_, forced, default)) in subs.iter().zip(flags) {
            args.extend(["--forced-display-flag".into(), format!("{id}:{}", forced as u8)]);
            args.extend(["--default-track-flag".into(), format!("{id}:{}", default as u8)]);
        }
    }
    // Ordre des pistes : vidéo, audio, sous-titres (fichier source n° 0).
    let order: Vec<String> = video
        .into_iter()
        .chain(audio.iter().map(|a| a.1))
        .chain(subs.iter().map(|s| s.1))
        .map(|id| format!("0:{id}"))
        .collect();
    if !order.is_empty() {
        args.extend(["--track-order".into(), order.join(",")]);
    }
    args.push(input.display().to_string());

    log(tx, format!("mkvmerge {}", args.join(" ")));
    let in_dir = input.parent().unwrap_or(Path::new("/"));
    let out_dir = output.parent().unwrap_or(Path::new("/"));
    let mut cmd = mkvmerge.command(&[in_dir, out_dir]);
    cmd.args(&args);

    // En mode « --gui-mode », mkvmerge écrit « #GUI#progress 42% ».
    let status = run_streaming(cmd, Some(cancel), |_, line| {
        if let Some(p) = line.strip_prefix("#GUI#progress ") {
            if let Ok(v) = p.trim_end_matches('%').trim().parse::<f64>() {
                on_progress(v / 100.0);
            }
        } else if !line.trim().is_empty() {
            log(tx, format!("[mkvmerge] {line}"));
        }
    })
    .await?;

    // Code 1 = avertissements seulement (fichier valide), 2 = erreur.
    match status.code() {
        Some(0) | Some(1) => Ok(output.to_path_buf()),
        _ => bail!("mkvmerge a échoué (code {status})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::makemkv::disc::DiscInfo;
    use crate::makemkv::robot::parse_line;

    fn t(id: u64, kind: &str, lang: &str, forced: bool) -> MkvTrack {
        MkvTrack {
            id,
            kind: kind.into(),
            properties: TrackProps { language: Some(lang.into()), forced_track: Some(forced), number: Some(id + 1) },
        }
    }

    #[test]
    fn maps_by_rank_then_by_language() {
        let raw = r#"SINFO:0,0,1,0,"Video"
SINFO:0,1,1,0,"Audio"
SINFO:0,1,3,0,"eng"
SINFO:0,2,1,0,"Audio"
SINFO:0,2,3,0,"fre"
SINFO:0,3,1,0,"Subtitles"
SINFO:0,3,3,0,"fre"
SINFO:0,4,1,0,"Subtitles"
SINFO:0,4,3,0,"fre"
SINFO:0,4,22,0,"4096""#;
        let lines: Vec<_> = raw.lines().map(parse_line).collect();
        let d = DiscInfo::from_lines(&lines);
        let title = &d.titles[0];

        let all = Selection { audio: [1, 2].into(), subtitles: [3, 4].into() };
        let ids = |v: Vec<(usize, &MkvTrack)>| v.into_iter().map(|(i, t)| (i, t.id)).collect::<Vec<_>>();

        // Fichier complet : par rang.
        let mkv = vec![t(0, "video", "und", false), t(1, "audio", "eng", false), t(2, "audio", "fre", false),
                       t(3, "subtitles", "fre", false), t(4, "subtitles", "fre", true)];
        assert!(matches_contents(title, &all, &mkv));
        assert_eq!(ids(map_tracks(title, &all, &mkv, StreamKind::Audio, &[2])), vec![(2, 2)]);
        assert_eq!(ids(map_tracks(title, &all, &mkv, StreamKind::Subtitle, &[4])), vec![(4, 4)]);

        // Fichier partiel conforme au plan : par rang parmi les pistes extraites.
        let part = Selection { audio: [2].into(), subtitles: [4].into() };
        let mkv2 = vec![t(0, "video", "und", false), t(1, "audio", "fre", false), t(2, "subtitles", "fre", true)];
        assert!(matches_contents(title, &part, &mkv2));
        assert!(!matches_contents(title, &all, &mkv2));
        assert_eq!(ids(map_tracks(title, &part, &mkv2, StreamKind::Subtitle, &[4])), vec![(4, 2)]);
        assert_eq!(ids(map_tracks(title, &part, &mkv2, StreamKind::Audio, &[2])), vec![(2, 1)]);

        // Plan non respecté : repli par langue + drapeau forcé.
        assert_eq!(ids(map_tracks(title, &all, &mkv2, StreamKind::Subtitle, &[4])), vec![(4, 2)]);
        assert_eq!(ids(map_tracks(title, &all, &mkv2, StreamKind::Audio, &[2])), vec![(2, 1)]);
    }

    #[test]
    fn default_audio_prefers_language_over_disc_order() {
        let raw = r#"SINFO:0,0,5,0,"A_AC3"
SINFO:0,0,3,0,"eng"
SINFO:0,1,5,0,"A_AC3"
SINFO:0,1,3,0,"fre""#;
        let lines: Vec<_> = raw.lines().map(parse_line).collect();
        let d = DiscInfo::from_lines(&lines);
        let sel = Selection { audio: [0, 1].into(), subtitles: Default::default() };
        assert_eq!(default_audio(&d.titles[0], &sel, "fra"), Some(1));
        assert_eq!(default_audio(&d.titles[0], &sel, "deu"), Some(0));
    }
}

/// Test d'intégration avec un vrai mkvmerge : `cargo test -- --ignored`.
#[cfg(test)]
mod integration {
    use super::*;
    use crate::makemkv::disc::DiscInfo;
    use crate::makemkv::robot::parse_line;

    #[tokio::test]
    #[ignore = "nécessite mkvmerge"]
    async fn remux_keeps_only_forced_french() {
        let dir = std::env::temp_dir().join(format!("carbon-copy-it-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nBonjour\n";
        for n in ["a", "b", "c"] {
            std::fs::write(dir.join(format!("{n}.srt")), srt).unwrap();
        }
        // MKV « brut » façon MakeMKV : fr complets, fr forcés, en complets.
        let input = dir.join("raw.mkv");
        let st = std::process::Command::new("mkvmerge")
            .args(["-q", "-o"]).arg(&input)
            .args(["--language", "0:fre"]).arg(dir.join("a.srt"))
            .args(["--language", "0:fre", "--forced-display-flag", "0:1"]).arg(dir.join("b.srt"))
            .args(["--language", "0:eng"]).arg(dir.join("c.srt"))
            .status().unwrap();
        assert!(st.success());

        let raw = r#"SINFO:0,0,1,0,"Subtitles"
SINFO:0,0,3,0,"fre"
SINFO:0,1,1,0,"Subtitles"
SINFO:0,1,3,0,"fre"
SINFO:0,1,22,0,"6144"
SINFO:0,2,1,0,"Subtitles"
SINFO:0,2,3,0,"eng""#;
        let lines: Vec<_> = raw.lines().map(parse_line).collect();
        let d = DiscInfo::from_lines(&lines);
        let sel = Selection { audio: Default::default(), subtitles: [1].into() };
        let (tx, _rx) = async_channel::unbounded();
        let out = dir.join("final.mkv");
        let tool = Tool::Native("mkvmerge".into());
        let all = Selection { audio: Default::default(), subtitles: [0, 1, 2].into() };
        remux(&tool, &d.titles[0], &sel, &all, &input, &out, "Test (2020)", "fra", &tx, CancellationToken::new(), |_| {})
            .await
            .unwrap();

        let tracks = identify(&tool, &out).await.unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].lang(), "fra");
        assert_eq!(tracks[0].properties.forced_track, Some(true));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    #[ignore = "nécessite mkvmerge et mkvpropedit"]
    async fn propedit_sets_flags_in_place() {
        let dir = std::env::temp_dir().join(format!("carbon-copy-pe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nBonjour\n";
        for n in ["a", "b"] {
            std::fs::write(dir.join(format!("{n}.srt")), srt).unwrap();
        }
        // Fichier tel que MakeMKV le produirait avec la règle exacte :
        // sous-titres fr complets + fr forcés, sans drapeau.
        let file = dir.join("B1_t00.mkv");
        let st = std::process::Command::new("mkvmerge")
            .args(["-q", "-o"]).arg(&file)
            .args(["--language", "0:fre", "--default-track-flag", "0:1"]).arg(dir.join("a.srt"))
            .args(["--language", "0:fre", "--default-track-flag", "0:0"]).arg(dir.join("b.srt"))
            .status().unwrap();
        assert!(st.success());
        let size_before = std::fs::metadata(&file).unwrap().len();

        let raw = r#"SINFO:0,0,5,0,"S_VOBSUB"
SINFO:0,0,3,0,"fre"
SINFO:0,1,5,0,"S_VOBSUB"
SINFO:0,1,3,0,"fre"
SINFO:0,1,22,0,"6144""#;
        let lines: Vec<_> = raw.lines().map(parse_line).collect();
        let d = DiscInfo::from_lines(&lines);
        let sel = Selection { audio: Default::default(), subtitles: [0, 1].into() };
        let tool = Tool::Native("mkvmerge".into());
        let tracks = identify(&tool, &file).await.unwrap();
        assert!(matches_contents(&d.titles[0], &sel, &tracks));
        let (tx, _rx) = async_channel::unbounded();
        let pe = tool.sibling("mkvpropedit").expect("mkvpropedit");
        set_properties(&pe, &d.titles[0], &sel, &tracks, &file, "Test (2020)", "fra", &tx).await.unwrap();

        let out = std::process::Command::new("mkvmerge").arg("-J").arg(&file).output().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["container"]["properties"]["title"], "Test (2020)");
        let t = &v["tracks"];
        assert_eq!(t[0]["properties"]["default_track"], false);
        assert_eq!(t[0]["properties"]["forced_track"], false);
        assert_eq!(t[1]["properties"]["default_track"], true);
        assert_eq!(t[1]["properties"]["forced_track"], true);
        // Modifié en place : pas de réécriture du fichier.
        assert!(std::fs::metadata(&file).unwrap().len().abs_diff(size_before) < 4096);
        let _ = std::fs::remove_dir_all(dir);
    }
}
