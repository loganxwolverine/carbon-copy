//! Télécommande web : petit serveur HTTP sur le réseau local pour suivre et
//! lancer un rip sans écran (HTPC).
//!
//! Le serveur ne touche jamais à l'état de l'application : il passe par le
//! canal d'événements existant, comme les tâches de fond.
//! - lecture : `AppEvent::RemoteQuery` → le thread GTK renvoie un [`Snapshot`] ;
//! - action  : `AppEvent::Remote(Command)` → le thread GTK exécute la même
//!   chose que le bouton correspondant.
//!
//! Pas d'authentification (choix de l'utilisateur : réseau local de confiance).
//! Les actions exigent l'en-tête `X-Intact` : un site web tiers ouvert dans un
//! navigateur du réseau ne peut pas l'envoyer sans autorisation CORS, qu'on ne
//! donne jamais.

use std::net::{Ipv4Addr, UdpSocket};
use std::path::PathBuf;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::events::{emit, log, AppEvent, Tx};

pub const DEFAULT_PORT: u16 = 8642;

/// Page unique de la télécommande (HTML, CSS et JS embarqués).
const INDEX: &str = include_str!("index.html");

/// Action demandée depuis la télécommande.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Rip,
    Cancel,
    Eject,
    Analyze,
    NextDisc,
    SelectTitle(usize),
    /// Coche (`on`) ou décoche une piste audio / sous-titres du titre choisi.
    Track { subtitle: bool, index: usize, on: bool },
    /// Fiche TMDB choisie par son identifiant.
    Movie(u64),
    /// Film saisi à la main (absent de TMDB) : titre et année.
    ManualMovie { title: String, year: Option<u32> },
    /// Dossier de destination.
    Folder(PathBuf),
}

/// État de l'application vu par la télécommande.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    /// `setup`, `error`, `nodisc`, `analysis`, `disc`, `rip` ou `done`.
    pub screen: String,
    pub status: String,
    pub error: Option<String>,
    pub analysis: Option<Bar>,
    pub disc: Option<DiscView>,
    pub movie: Option<MovieView>,
    pub filename: String,
    pub output_dir: String,
    /// Le bouton « Lancer le rip » serait actif.
    pub can_rip: bool,
    pub ripping: bool,
    /// Préparation en arrière-plan (avant le rip) ou progression du rip.
    pub progress: Option<ProgressView>,
    pub result: Option<RipResult>,
    pub notice: Option<Notice>,
    pub log: Vec<String>,
    pub version: String,
    /// Clé TMDB configurée : la recherche de film est possible.
    pub tmdb_ready: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Bar {
    pub label: String,
    pub fraction: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DiscView {
    pub kind: String,
    pub name: String,
    pub title_index: usize,
    pub titles: Vec<TitleView>,
    pub audio: Vec<String>,
    pub subtitles: Vec<String>,
    /// Toutes les pistes du titre choisi, cochées ou non.
    pub tracks: Vec<TrackView>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TrackView {
    pub subtitle: bool,
    pub index: usize,
    pub lang: String,
    pub codec: String,
    pub layout: String,
    /// Commentaire, audiodescription, cœur dérivé…
    pub extras: String,
    pub forced: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TitleView {
    pub index: usize,
    pub duration: String,
    pub chapters: u32,
    pub size: String,
    pub main: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MovieView {
    pub title: String,
    pub year: Option<u32>,
    pub overview: String,
    pub poster: Option<String>,
    pub tmdb_id: Option<u64>,
    /// Fiche TMDB chargée (sinon : titre saisi ou nom du disque).
    pub identified: bool,
    pub original_title: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProgressView {
    /// `copy`, `verify`, `finalize`, `eject` ; `prepare` hors rip.
    pub phase: String,
    pub label: String,
    pub fraction: f64,
    pub speed: Option<String>,
    pub remaining: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RipResult {
    pub ok: bool,
    pub file: String,
    pub size: String,
    pub elapsed: String,
    pub error: Option<String>,
}

/// Dernière notification (toast) : `id` croissant pour ne l'afficher qu'une fois.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Notice {
    pub id: u64,
    pub text: String,
}

/// Démarre le serveur ; l'annulation du jeton renvoyé l'arrête.
pub fn start(port: u16, tx: Tx) -> CancellationToken {
    let cancel = CancellationToken::new();
    crate::backend::runtime().spawn(serve(port, tx, cancel.clone()));
    cancel
}

async fn serve(port: u16, tx: Tx, cancel: CancellationToken) {
    let listener = match tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await {
        Ok(l) => l,
        Err(e) => {
            log(&tx, format!("✖ Télécommande web : port {port} indisponible ({e})"));
            emit(&tx, AppEvent::Toast(format!("Télécommande web : port {port} indisponible")));
            return;
        }
    };
    log(&tx, format!("Télécommande web active : {}", urls(port).join("  ou  ")));
    let app = Router::new()
        .route("/", get(index))
        .route("/api/state", get(state))
        .route("/api/history", get(history))
        .route("/api/search", get(search))
        .route("/api/dirs", get(dirs))
        .route("/api/title/{index}", post(select_title))
        .route("/api/track", post(track))
        .route("/api/movie", post(movie))
        .route("/api/folder", post(folder))
        .route("/api/mkdir", post(mkdir))
        .route("/api/{action}", post(action))
        .with_state(tx.clone());
    let res = axum::serve(listener, app).with_graceful_shutdown(cancel.cancelled_owned()).await;
    match res {
        Ok(()) => log(&tx, "Télécommande web arrêtée."),
        Err(e) => log(&tx, format!("✖ Télécommande web : {e}")),
    }
}

/// Adresses à saisir sur le téléphone : IP locale, puis nom de la machine
/// (`.local`, résolu par mDNS / Avahi quand il est actif).
pub fn urls(port: u16) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(ip) = lan_ip() {
        out.push(format!("http://{ip}:{port}/"));
    }
    let host = gtk::glib::host_name();
    if !host.is_empty() && host != "localhost" {
        out.push(format!("http://{}.local:{port}/", host.trim_end_matches(".local")));
    }
    if out.is_empty() {
        out.push(format!("http://localhost:{port}/"));
    }
    out
}

/// IP de l'interface qui sert la route par défaut. `connect` sur UDP ne fait
/// que choisir la route : aucun paquet n'est envoyé.
fn lan_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

async fn index() -> Response {
    ([(header::CACHE_CONTROL, "no-cache")], Html(INDEX)).into_response()
}

async fn state(State(tx): State<Tx>) -> Response {
    let (reply, rx) = oneshot::channel();
    if tx.send(AppEvent::RemoteQuery(reply)).await.is_err() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match tokio::time::timeout(Duration::from_secs(5), rx).await {
        Ok(Ok(snap)) => ([(header::CACHE_CONTROL, "no-store")], Json(snap)).into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

#[derive(Serialize)]
struct HistoryItem {
    title: String,
    year: Option<u32>,
    poster: Option<String>,
    file: String,
    size: String,
    date: String,
}

/// Derniers rips (historique de l'application, sans parcourir le dossier :
/// un partage réseau lent ou coupé ne doit pas bloquer la page).
async fn history() -> Response {
    let entries = tokio::task::spawn_blocking(crate::history::load).await.unwrap_or_default();
    let items: Vec<HistoryItem> = entries
        .into_iter()
        .take(12)
        .map(|e| HistoryItem {
            title: e.title.clone(),
            year: e.year,
            poster: e.poster_path.as_deref().map(poster_url),
            file: e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            size: e.size_label(),
            date: e.date_label(),
        })
        .collect();
    ([(header::CACHE_CONTROL, "no-store")], Json(items)).into_response()
}

/// URL d'une affiche TMDB (chargée directement par le navigateur).
pub fn poster_url(path: &str) -> String {
    format!("https://image.tmdb.org/t/p/w342{path}")
}

fn allowed(headers: &HeaderMap) -> bool {
    headers.contains_key("x-intact")
}

async fn action(State(tx): State<Tx>, headers: HeaderMap, Path(action): Path<String>) -> StatusCode {
    if !allowed(&headers) {
        return StatusCode::FORBIDDEN;
    }
    let cmd = match action.as_str() {
        "rip" => Command::Rip,
        "cancel" => Command::Cancel,
        "eject" => Command::Eject,
        "analyze" => Command::Analyze,
        "next" => Command::NextDisc,
        _ => return StatusCode::NOT_FOUND,
    };
    emit(&tx, AppEvent::Remote(cmd));
    StatusCode::ACCEPTED
}

async fn select_title(State(tx): State<Tx>, headers: HeaderMap, Path(index): Path<usize>) -> StatusCode {
    if !allowed(&headers) {
        return StatusCode::FORBIDDEN;
    }
    emit(&tx, AppEvent::Remote(Command::SelectTitle(index)));
    StatusCode::ACCEPTED
}

#[derive(Deserialize)]
struct TrackReq {
    subtitle: bool,
    index: usize,
    on: bool,
}

async fn track(State(tx): State<Tx>, headers: HeaderMap, Json(r): Json<TrackReq>) -> StatusCode {
    if !allowed(&headers) {
        return StatusCode::FORBIDDEN;
    }
    emit(&tx, AppEvent::Remote(Command::Track { subtitle: r.subtitle, index: r.index, on: r.on }));
    StatusCode::ACCEPTED
}

/// Fiche du film : identifiant / lien TMDB, ou titre et année saisis.
#[derive(Deserialize)]
struct MovieReq {
    #[serde(default)]
    tmdb: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    year: Option<u32>,
}

async fn movie(State(tx): State<Tx>, headers: HeaderMap, Json(r): Json<MovieReq>) -> StatusCode {
    if !allowed(&headers) {
        return StatusCode::FORBIDDEN;
    }
    let cmd = match (r.tmdb.as_deref().map(str::trim).filter(|t| !t.is_empty()), r.title) {
        (Some(t), _) => match crate::metadata::parse_tmdb_id(t) {
            Some(id) => Command::Movie(id),
            None => return StatusCode::BAD_REQUEST,
        },
        (None, Some(title)) if !title.trim().is_empty() => {
            Command::ManualMovie { title: title.trim().to_string(), year: r.year.filter(|y| (1880..=2100).contains(y)) }
        }
        _ => return StatusCode::BAD_REQUEST,
    };
    emit(&tx, AppEvent::Remote(cmd));
    StatusCode::ACCEPTED
}

#[derive(Deserialize)]
struct SearchReq {
    q: String,
    #[serde(default)]
    year: Option<String>,
}

#[derive(Serialize)]
struct SearchItem {
    id: u64,
    title: String,
    original_title: String,
    year: Option<u32>,
    overview: String,
    poster: Option<String>,
}

/// Recherche TMDB (titre, identifiant ou lien), comme la fenêtre
/// « Identifier le film ».
async fn search(Query(r): Query<SearchReq>) -> Response {
    let cfg = tokio::task::spawn_blocking(crate::config::Config::load).await.unwrap_or_default();
    let key = cfg.tmdb_api_key.trim().to_string();
    if key.is_empty() {
        return (StatusCode::PRECONDITION_FAILED, "Clé TMDB absente : à ajouter dans les préférences de l'application").into_response();
    }
    let q = r.q.trim();
    if q.is_empty() {
        return Json(Vec::<SearchItem>::new()).into_response();
    }
    let year = r.year.and_then(|y| y.trim().parse().ok());
    let http = crate::backend::http();
    let res = match crate::metadata::parse_tmdb_id(q) {
        Some(id) => crate::metadata::tmdb_movie(http, &key, id, &cfg.language).await.map(|m| vec![m]),
        None => crate::metadata::tmdb_search(http, &key, q, year, &cfg.language).await,
    };
    match res {
        Ok(movies) => Json(
            movies
                .into_iter()
                .map(|m| SearchItem {
                    id: m.id,
                    year: m.year(),
                    poster: m.poster_path.as_deref().map(poster_url),
                    title: m.title,
                    original_title: m.original_title,
                    overview: m.overview,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("TMDB : {e:#}")).into_response(),
    }
}

#[derive(Deserialize)]
struct DirsReq {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Serialize)]
struct DirList {
    path: String,
    parent: Option<String>,
    home: String,
    dirs: Vec<String>,
}

/// Sous-dossiers d'un dossier (navigation pour choisir la destination).
/// Délai limité : un partage réseau qui ne répond pas ne bloque pas la page.
async fn dirs(Query(r): Query<DirsReq>) -> Response {
    let home = gtk::glib::home_dir();
    let path = r.path.map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.clone());
    let task = tokio::task::spawn_blocking(move || -> std::io::Result<DirList> {
        let mut dirs: Vec<String> = std::fs::read_dir(&path)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false) || e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        dirs.sort_by_key(|n| n.to_lowercase());
        Ok(DirList {
            parent: path.parent().map(|p| p.display().to_string()),
            path: path.display().to_string(),
            home: home.display().to_string(),
            dirs,
        })
    });
    match tokio::time::timeout(Duration::from_secs(10), task).await {
        Ok(Ok(Ok(list))) => ([(header::CACHE_CONTROL, "no-store")], Json(list)).into_response(),
        Ok(Ok(Err(e))) => (StatusCode::NOT_FOUND, format!("Dossier illisible : {e}")).into_response(),
        _ => (StatusCode::GATEWAY_TIMEOUT, "Le dossier ne répond pas").into_response(),
    }
}

#[derive(Deserialize)]
struct FolderReq {
    path: String,
}

async fn folder(State(tx): State<Tx>, headers: HeaderMap, Json(r): Json<FolderReq>) -> StatusCode {
    if !allowed(&headers) {
        return StatusCode::FORBIDDEN;
    }
    let path = PathBuf::from(r.path);
    if !path.is_absolute() {
        return StatusCode::BAD_REQUEST;
    }
    emit(&tx, AppEvent::Remote(Command::Folder(path)));
    StatusCode::ACCEPTED
}

#[derive(Deserialize)]
struct MkdirReq {
    parent: String,
    name: String,
}

/// Crée un sous-dossier ; renvoie son chemin.
async fn mkdir(headers: HeaderMap, Json(r): Json<MkdirReq>) -> Response {
    if !allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let name = r.name.trim().to_string();
    let parent = PathBuf::from(r.parent);
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || !parent.is_absolute() {
        return (StatusCode::BAD_REQUEST, "Nom de dossier invalide").into_response();
    }
    let path = parent.join(name);
    let p = path.clone();
    match tokio::task::spawn_blocking(move || std::fs::create_dir(&p)).await {
        Ok(Ok(())) => Json(serde_json::json!({ "path": path.display().to_string() })).into_response(),
        Ok(Err(e)) => (StatusCode::CONFLICT, format!("Création impossible : {e}")).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
