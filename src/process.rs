//! Exécution asynchrone de processus externes avec lecture ligne à ligne.
//!
//! Point clé : on ne doit JAMAIS bloquer le thread GTK. Tous les processus
//! sont lancés sur le runtime tokio (threads de travail) ; leurs sorties sont
//! lues de façon asynchrone et chaque ligne est transmise à un callback, qui
//! la parse puis l'envoie à l'interface via un canal (`async_channel`).

use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

/// Un exécutable externe : binaire natif ou commande exposée par un Flatpak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tool {
    Native(PathBuf),
    Flatpak { app_id: String, command: String },
}

impl Tool {
    /// Prépare une commande tokio.
    ///
    /// `paths` : dossiers auxquels l'outil doit pouvoir accéder. Pour un
    /// Flatpak on les ajoute au bac à sable via `flatpak run --filesystem=…`
    /// (le Flatpak MakeMKV n'a par défaut accès qu'à `~/.MakeMKV` et aux Vidéos).
    pub fn command(&self, paths: &[&Path]) -> Command {
        match self {
            Tool::Native(p) => Command::new(p),
            Tool::Flatpak { app_id, command } => {
                let mut c = Command::new("flatpak");
                // --die-with-parent : sans lui, tuer `flatpak run` laisse makemkvcon
                // tourner dans le bac à sable et monopoliser le lecteur.
                c.arg("run").arg("--die-with-parent").arg(format!("--command={command}"));
                for p in paths {
                    c.arg(format!("--filesystem={}", p.display()));
                }
                c.arg(app_id);
                c
            }
        }
    }

    /// Autre commande du même paquet (ex. `mkvpropedit` à côté de `mkvmerge`).
    pub fn sibling(&self, name: &str) -> Option<Tool> {
        match self {
            Tool::Native(p) => {
                let next = p.with_file_name(name);
                if next.is_file() {
                    Some(Tool::Native(next))
                } else {
                    which(name).map(Tool::Native)
                }
            }
            Tool::Flatpak { app_id, .. } => Some(Tool::Flatpak { app_id: app_id.clone(), command: name.into() }),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Tool::Native(p) => format!("{} (natif)", p.display()),
            Tool::Flatpak { app_id, .. } => format!("{app_id} (Flatpak)"),
        }
    }

    /// Libellé court pour la barre de titre.
    pub fn short_label(&self) -> &'static str {
        match self {
            Tool::Native(_) => "natif",
            Tool::Flatpak { .. } => "Flatpak",
        }
    }
}

/// Recherche un exécutable dans le `PATH` (équivalent de `which`).
pub fn which(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false))
}

/// Origine d'une ligne lue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Lance `cmd` et appelle `on_line` pour chaque ligne de stdout ET stderr.
///
/// - Les deux flux sont lus en parallèle (`tokio::join!`) : si on lisait
///   stdout jusqu'au bout avant stderr, un processus qui remplit le tampon
///   de stderr (64 Kio) se bloquerait indéfiniment (interblocage classique).
/// - On lit des octets (`read_until`) puis on décode en UTF-8 « avec perte » :
///   les noms de disques peuvent contenir de l'ISO-8859-1 invalide en UTF-8,
///   et `lines()` échouerait alors sur toute la suite du flux.
/// - `cancel` permet d'interrompre : le processus est tué (`kill_on_drop`).
pub async fn run_streaming<F>(mut cmd: Command, cancel: Option<CancellationToken>, on_line: F) -> Result<ExitStatus>
where
    F: Fn(Stream, String) + Send + Sync,
{
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null()).kill_on_drop(true);
    let program = format!("{:?}", cmd.as_std().get_program());
    let mut child = cmd.spawn().with_context(|| format!("impossible de lancer {program}"))?;

    let stdout = child.stdout.take().expect("stdout pipé");
    let stderr = child.stderr.take().expect("stderr pipé");
    let on_line = &on_line;

    let readers = async {
        tokio::join!(pump(stdout, |l| on_line(Stream::Stdout, l)), pump(stderr, |l| on_line(Stream::Stderr, l)));
        child.wait().await
    };

    let token = cancel.unwrap_or_default();
    tokio::select! {
        status = readers => Ok(status?),
        _ = token.cancelled() => {
            // Le `child` est détruit à la sortie de `readers` → kill_on_drop.
            anyhow::bail!("opération annulée")
        }
    }
}

async fn pump<R: AsyncRead + Unpin>(r: R, f: impl Fn(String)) {
    let mut reader = BufReader::new(r);
    let mut buf = Vec::with_capacity(512);
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim_end_matches(['\n', '\r']);
                if !line.is_empty() {
                    f(line.to_string());
                }
            }
        }
    }
}

/// Lance une commande et récupère toute sa sortie standard (petites commandes).
pub async fn run_capture(mut cmd: Command) -> Result<(ExitStatus, String)> {
    cmd.stdin(Stdio::null()).kill_on_drop(true);
    let out = cmd.output().await.context("échec d'exécution")?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status, s))
}
