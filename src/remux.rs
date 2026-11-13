//! Remux final avec `mkvmerge` : ne conserve que les pistes sélectionnées.
//!
//! MakeMKV extrait TOUTES les pistes (règle `+sel:all`). On identifie ensuite
//! les pistes du MKV intermédiaire avec `mkvmerge -J`, on les fait correspondre
//! aux flux `SINFO` du disque, puis on produit le fichier final avec uniquement
//! la vidéo principale, l'audio et les sous-titres cochés, et les bons drapeaux
//! (`default`, `forced`).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::events::{log, Tx};
use crate::makemkv::disc::{StreamKind, Title};
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

async fn identify(mkvmerge: &Tool, file: &Path) -> Result<Vec<MkvTrack>> {
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

/// Fait correspondre chaque flux SINFO sélectionné à un identifiant de piste MKV.
///
/// Avec `+sel:all`, MakeMKV écrit les pistes dans l'ordre du disque : on mappe
/// donc par rang au sein de chaque type (k-ième audio ↔ k-ième audio). Si les
/// effectifs diffèrent (cas rare), on se rabat sur une correspondance par
/// langue (+ drapeau forcé pour les sous-titres), dans l'ordre.
pub fn map_tracks(title: &Title, mkv: &[MkvTrack], kind: StreamKind, wanted: &[usize]) -> Vec<(usize, u64)> {
    let disc: Vec<_> = title.streams_of(kind).collect();
    let file: Vec<_> = mkv.iter().filter(|t| t.stream_kind() == kind).collect();
    let mut out = Vec::new();

    if disc.len() == file.len() {
        for (s, t) in disc.iter().zip(&file) {
            if wanted.contains(&s.index) {
                out.push((s.index, t.id));
            }
        }
        return out;
    }

    let mut used = std::collections::HashSet::new();
    for s in disc.iter().filter(|s| wanted.contains(&s.index)) {
        let forced = s.is_forced();
        let found = file.iter().find(|t| {
            !used.contains(&t.id)
                && t.lang() == s.lang()
                && (kind != StreamKind::Subtitle || t.properties.forced_track.unwrap_or(false) == forced)
        });
        if let Some(t) = found {
            used.insert(t.id);
            out.push((s.index, t.id));
        }
    }
    out
}

/// Remuxe `input` vers `output` en ne gardant que la sélection.
pub async fn remux(
    mkvmerge: &Tool,
    title: &Title,
    sel: &Selection,
    input: &Path,
    output: &Path,
    movie_title: &str,
    tx: &Tx,
    cancel: CancellationToken,
    on_progress: impl Fn(f64) + Send + Sync,
) -> Result<PathBuf> {
    let tracks = identify(mkvmerge, input).await?;
    let audio_wanted: Vec<_> = sel.audio.iter().copied().collect();
    let subs_wanted: Vec<_> = sel.subtitles.iter().copied().collect();

    let video = tracks.iter().find(|t| t.kind == "video").map(|t| t.id);
    let audio = map_tracks(title, &tracks, StreamKind::Audio, &audio_wanted);
    let subs = map_tracks(title, &tracks, StreamKind::Subtitle, &subs_wanted);

    if audio.len() != audio_wanted.len() || subs.len() != subs_wanted.len() {
        log(tx, "⚠ Certaines pistes sélectionnées n'ont pas été retrouvées dans le fichier extrait.");
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
        // Première piste audio sélectionnée = piste par défaut.
        for (i, (_, id)) in audio.iter().enumerate() {
            args.extend(["--default-track-flag".into(), format!("{id}:{}", (i == 0) as u8)]);
        }
    }
    if subs.is_empty() {
        args.push("--no-subtitles".into());
    } else {
        args.extend(["--subtitle-tracks".into(), ids(&subs)]);
        // Sous-titres forcés : drapeau « forced » ET « default », pour que les
        // lecteurs (Plex, Kodi, VLC…) les affichent automatiquement.
        // Sous-titres complets : jamais par défaut.
        let mut forced_default_given = false;
        for (idx, id) in &subs {
            let forced = title.streams.get(*idx).is_some_and(|s| s.is_forced());
            args.extend(["--forced-display-flag".into(), format!("{id}:{}", forced as u8)]);
            let default = forced && !forced_default_given;
            forced_default_given |= default;
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
            properties: TrackProps { language: Some(lang.into()), forced_track: Some(forced) },
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

        // Même effectif : par rang.
        let mkv = vec![t(0, "video", "und", false), t(1, "audio", "eng", false), t(2, "audio", "fre", false),
                       t(3, "subtitles", "fre", false), t(4, "subtitles", "fre", true)];
        assert_eq!(map_tracks(title, &mkv, StreamKind::Audio, &[2]), vec![(2, 2)]);
        assert_eq!(map_tracks(title, &mkv, StreamKind::Subtitle, &[4]), vec![(4, 4)]);

        // Effectif différent : par langue + drapeau forcé.
        let mkv2 = vec![t(0, "video", "und", false), t(1, "audio", "fre", false), t(2, "subtitles", "fre", true)];
        assert_eq!(map_tracks(title, &mkv2, StreamKind::Subtitle, &[4]), vec![(4, 2)]);
        assert_eq!(map_tracks(title, &mkv2, StreamKind::Audio, &[2]), vec![(2, 1)]);
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
        remux(&tool, &d.titles[0], &sel, &input, &out, "Test (2020)", &tx, CancellationToken::new(), |_| {})
            .await
            .unwrap();

        let tracks = identify(&tool, &out).await.unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].lang(), "fra");
        assert_eq!(tracks[0].properties.forced_track, Some(true));
        let _ = std::fs::remove_dir_all(dir);
    }
}
