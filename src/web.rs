//! HTTP: plakatvisning, infoside, ret match, scan og Windows-hjælperen.
use askama::Template;
use axum::extract::{Form, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::db::{self, Game};
use crate::{download, import, scan, Shared};

const KINDS: [(&str, &str); 5] =
    [("iso", "ISO"), ("rar", "RAR"), ("setup", "Installer"), ("portable", "Portable"), ("mappe", "Mappe")];
const SORTS: [(&str, &str, &str); 4] = [
    ("navn", "Navn", "title collate nocase"),
    ("aar", "År", "year desc, title collate nocase"),
    ("nyeste", "Nyeste", "added desc"),
    ("stoerrelse", "Størrelse", "size desc"),
];

pub fn router(app: Shared) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/spil/{id}", get(game))
        .route("/spil/{id}/match", post(rematch))
        .route("/spil/{id}/hent", get(download::file))
        .route("/spil/{id}/zip", get(download::folder))
        .route("/spil/{id}/crc", get(download::crc))
        .route("/scan", post(scan_now))
        .route("/status", get(status))
        .route("/api/spil", get(api_games))
        .route("/import", get(imports).post(import_now))
        .route("/import/{hash}/retry", post(retry))
        .route("/img/{appid}/{kind}", get(img))
        .route("/windows", get(windows))
        .route("/windows/{name}", get(windows_file))
        .route("/linux/{name}", get(linux_file))
        .route("/static/style.css", get(css))
        .with_state(app)
}

fn render<T: Template>(t: T) -> Response {
    match t.render() {
        Ok(s) => Html(s).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn label(kind: &str) -> String {
    if kind == "software" {
        return "Software".into();
    }
    KINDS.iter().find(|k| k.0 == kind).map_or(kind, |k| k.1).to_string()
}

fn spil(action: &str, rel: &str) -> String {
    format!("spil://{action}?p={}", urlencoding::encode(rel))
}

fn fname(rel: &str) -> String {
    rel.rsplit('/').next().unwrap_or(rel).to_string()
}

fn img_url(app: &Shared, g: &Game, kind: &str) -> String {
    match g.appid {
        Some(a) if app.cfg.covers().join(format!("{a}_{kind}.jpg")).exists() => format!("/img/{a}/{kind}"),
        _ => String::new(),
    }
}

struct Card {
    id: i64,
    title: String,
    sub: String,
    kind: String,
    label: String,
    poster: String,
    search: String,
}

struct Opt {
    key: String,
    label: String,
    on: bool,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexT {
    cards: Vec<Card>,
    kinds: Vec<Opt>,
    sorts: Vec<Opt>,
    msg: String,
    software: bool,
    has_software: bool,
}

#[derive(Deserialize)]
struct SortQ {
    sort: Option<String>,
    fane: Option<String>,
}

async fn index(State(app): State<Shared>, Query(q): Query<SortQ>) -> Response {
    let sort = q.sort.unwrap_or_else(|| "navn".into());
    let order = SORTS.iter().find(|s| s.0 == sort).unwrap_or(&SORTS[0]).2;
    let software = q.fane.as_deref() == Some("software");
    let games = db::list(&app.db.lock().unwrap(), order).unwrap_or_default();
    let has_software = games.iter().any(|g| g.kind == "software");
    let cards = games
        .iter()
        .filter(|g| (g.kind == "software") == software)
        .map(|g| Card {
            id: g.id,
            title: g.title.clone(),
            sub: if g.kind == "software" {
                format!("{:.1} GB", g.size as f64 / 1e9)
            } else {
                [g.year.map(|y| y.to_string()), g.grp.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ")
            },
            kind: g.kind.clone(),
            label: label(&g.kind),
            poster: img_url(&app, g, "poster"),
            search: format!("{} {}", g.title, g.grp.as_deref().unwrap_or_default()).to_lowercase(),
        })
        .collect();
    let kinds = KINDS.iter().map(|k| Opt { key: k.0.into(), label: k.1.into(), on: false }).collect();
    let sorts = SORTS.iter().map(|s| Opt { key: s.0.into(), label: s.1.into(), on: s.0 == sort }).collect();
    let msg = app.status.lock().unwrap().msg.clone();
    render(IndexT { cards, kinds, sorts, msg, software, has_software })
}

struct Btn {
    href: String,
    label: String,
    primary: bool,
}

#[derive(Template)]
#[template(path = "game.html")]
struct GameT {
    id: i64,
    title: String,
    year: String,
    grp: String,
    kind: String,
    label: String,
    size: String,
    poster: String,
    poster_wide: bool,
    hero: String,
    has_meta: bool,
    metacritic: String,
    genres: Vec<String>,
    short: String,
    about: String,
    credits: String,
    url: String,
    screens: Vec<String>,
    req_min: Vec<(String, String)>,
    req_rec: Vec<(String, String)>,
    buttons: Vec<Btn>,
    downloads: Vec<Btn>,
    note: String,
    unc_path: String,
    files: Vec<String>,
    updates: Vec<String>,
    updates_href: String,
    current: String,
    software: bool,
}

/// "Hukommelse: 16 GB RAM" → (etiket, værdi), så etiketten kan fremhæves.
fn req_rows(lines: &[String]) -> Vec<(String, String)> {
    lines.iter().map(|l| match l.split_once(": ") {
        Some((k, v)) if k.chars().count() <= 30 => (k.to_string(), v.to_string()),
        _ => (String::new(), l.clone()),
    }).collect()
}

async fn game(State(app): State<Shared>, Path(id): Path<i64>) -> Response {
    let Some(g) = db::get(&app.db.lock().unwrap(), id) else {
        return (StatusCode::NOT_FOUND, "Ukendt spil").into_response();
    };
    let mut buttons = vec![];
    let mut note = String::new();
    match g.kind.as_str() {
        "iso" => {
            for f in &g.isos {
                let label = if g.isos.len() > 1 { format!("⬇ Installer – {}", fname(f)) } else { "⬇ Installer".into() };
                buttons.push(Btn { href: spil("mount", f), label, primary: true });
            }
            buttons.push(Btn { href: spil("unmount", &g.isos[0]), label: "Afmonter".into(), primary: false });
        }
        "setup" => {
            for (i, f) in g.setups.iter().enumerate() {
                let label = if i == 0 { "⬇ Installer".into() } else { format!("Kør {}", fname(f)) };
                buttons.push(Btn { href: spil("run", f), label, primary: i == 0 });
            }
        }
        "rar" => note = "Kun rar-filer – kan ikke mountes. Pak dem ud med spilarr unpack.".into(),
        "software" => note = "Program – hent det nedenfor, eller åbn stien under Filer i Stifinder.".into(),
        _ => {}
    }
    // spil://-hjælperen kender kun spilmappen
    if g.kind != "software" {
        buttons.push(Btn { href: spil("open", &g.rel), label: "📂 Åbn mappe".into(), primary: false });
    }

    // Download gennem browseren (fx via Tailscale, når man ikke er hjemme)
    let mut downloads: Vec<Btn> = g.isos.iter().map(|f| Btn {
        href: format!("/spil/{}/hent?f={}", g.id, urlencoding::encode(f)),
        label: if g.isos.len() > 1 { format!("⤓ {}", fname(f)) } else { "⤓ Hent ISO".into() },
        primary: false,
    }).collect();
    downloads.push(Btn { href: format!("/spil/{}/zip", g.id), label: "⤓ Hent mappe som zip".into(), primary: false });

    let files = g.isos.iter().map(|f| format!("💿 {}", fname(f)))
        .chain(g.setups.iter().map(|f| format!("⚙️ {}", fname(f))))
        .chain(g.rars.iter().map(|f| format!("📦 {}", fname(f))))
        .collect();
    let m = g.meta.clone().unwrap_or_default();
    let mut credits = m.developers.join(", ");
    if !m.publishers.is_empty() {
        credits = format!("{credits} / {}", m.publishers.join(", "));
    }
    let current = match (&g.appid, &g.meta) {
        (Some(a), Some(m)) => format!("Steam {a} – {}", m.name),
        _ => "intet match".into(),
    } + if g.locked { " (låst)" } else { "" };
    render(GameT {
        id: g.id,
        title: g.title.clone(),
        year: g.year.map_or("?".into(), |y| y.to_string()),
        grp: g.grp.clone().unwrap_or_default(),
        kind: g.kind.clone(),
        label: label(&g.kind),
        size: format!("{:.1} GB", g.size as f64 / 1e9),
        poster: img_url(&app, &g, "poster"),
        poster_wide: m.poster_wide,
        hero: img_url(&app, &g, "hero"),
        has_meta: g.meta.is_some(),
        metacritic: m.metacritic.map(|s| s.to_string()).unwrap_or_default(),
        genres: m.genres,
        short: m.short,
        about: m.about,
        credits,
        url: m.url,
        screens: m.screens,
        req_min: req_rows(&m.req_min),
        req_rec: req_rows(&m.req_rec),
        buttons,
        downloads,
        note,
        unc_path: app.cfg.unc(&g.rel),
        files,
        updates_href: if g.updates.is_empty() { String::new() } else { spil("open", &format!("{}/updates", g.rel)) },
        updates: g.updates,
        current,
        software: g.kind == "software",
    })
}

#[derive(Deserialize)]
struct MatchF {
    #[serde(default)]
    q: String,
}

async fn rematch(State(app): State<Shared>, Path(id): Path<i64>, Form(f): Form<MatchF>) -> Redirect {
    let a = app.clone();
    let _ = tokio::task::spawn_blocking(move || scan::fetch_meta(&a, id, Some(f.q), true)).await;
    Redirect::to(&format!("/spil/{id}"))
}

#[derive(Deserialize)]
struct ScanF {
    #[serde(default)]
    force: Option<String>,
}

async fn scan_now(State(app): State<Shared>, Form(f): Form<ScanF>) -> Redirect {
    let force = f.force.is_some_and(|v| v == "1");
    tokio::task::spawn_blocking(move || scan::scan(&app, force));
    Redirect::to("/")
}

struct ImportRow {
    hash: String,
    name: String,
    dest: String,
    ok: bool,
    msg: String,
    when: String,
}

#[derive(Template)]
#[template(path = "import.html")]
struct ImportT {
    rows: Vec<ImportRow>,
    enabled: bool,
    category: String,
    minutes: String,
    msg: String,
}

fn ago(t: f64) -> String {
    let m = ((crate::now() - t) / 60.0).max(0.0) as i64;
    match m {
        0 => "lige nu".into(),
        1..=59 => format!("{m} min siden"),
        60..=2879 => format!("{} t siden", m / 60),
        _ => format!("{} dage siden", m / 1440),
    }
}

async fn imports(State(app): State<Shared>) -> Response {
    let rows = db::imports(&app.db.lock().unwrap())
        .unwrap_or_default()
        .into_iter()
        .map(|i| ImportRow { when: ago(i.time), hash: i.hash, name: i.name, dest: i.dest, ok: i.ok, msg: i.msg })
        .collect();
    render(ImportT {
        rows,
        enabled: !app.cfg.qbit_url.is_empty(),
        category: app.cfg.qbit_category.clone(),
        minutes: app.cfg.import_minutes.to_string(),
        msg: app.status.lock().unwrap().msg.clone(),
    })
}

async fn import_now(State(app): State<Shared>) -> Redirect {
    tokio::task::spawn_blocking(move || import::run(&app));
    Redirect::to("/import")
}

async fn retry(State(app): State<Shared>, Path(hash): Path<String>) -> Redirect {
    let _ = db::forget_import(&app.db.lock().unwrap(), &hash);
    tokio::task::spawn_blocking(move || import::run(&app));
    Redirect::to("/import")
}

async fn status(State(app): State<Shared>) -> Json<crate::Status> {
    Json(app.status.lock().unwrap().clone())
}

/// Biblioteket som JSON til SpilarrOS-klienten.
#[derive(serde::Serialize)]
pub struct ApiGame {
    pub id: i64,
    pub rel: String,
    pub title: String,
    pub year: Option<i64>,
    pub grp: Option<String>,
    pub kind: String,
    pub label: String,
    pub size: i64,
    pub isos: Vec<String>,
    pub poster: String,
    pub poster_wide: bool,
    pub genres: Vec<String>,
    pub short: String,
    pub req_min: Vec<String>,
    pub req_rec: Vec<String>,
}

async fn api_games(State(app): State<Shared>) -> Json<Vec<ApiGame>> {
    let games = db::list(&app.db.lock().unwrap(), "title collate nocase").unwrap_or_default();
    Json(games.iter().map(|g| {
        let m = g.meta.clone().unwrap_or_default();
        ApiGame {
            id: g.id,
            rel: g.rel.clone(),
            title: g.title.clone(),
            year: g.year,
            grp: g.grp.clone(),
            kind: g.kind.clone(),
            label: label(&g.kind),
            size: g.size,
            isos: g.isos.clone(),
            poster: img_url(&app, g, "poster"),
            poster_wide: m.poster_wide,
            genres: m.genres,
            short: m.short,
            req_min: m.req_min,
            req_rec: m.req_rec,
        }
    }).collect())
}

async fn img(State(app): State<Shared>, Path((appid, kind)): Path<(i64, String)>) -> Response {
    if kind != "poster" && kind != "hero" {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(app.cfg.covers().join(format!("{appid}_{kind}.jpg"))).await {
        Ok(b) => ([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "public, max-age=86400")], b).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn server(h: &HeaderMap) -> String {
    format!("http://{}", h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("192.168.68.58:8790"))
}

#[derive(Template)]
#[template(path = "windows.html")]
struct WindowsT {
    server: String,
}

async fn windows(h: HeaderMap) -> Response {
    render(WindowsT { server: server(&h) })
}

async fn windows_file(State(app): State<Shared>, h: HeaderMap, Path(name): Path<String>) -> Response {
    let src = match name.as_str() {
        "install.ps1" => include_str!("../windows/install.ps1"),
        "spil-handler.ps1" => include_str!("../windows/spil-handler.ps1"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let body = src.replace("__SERVER__", &server(&h)).replace("__UNC_ROOT__", &app.cfg.unc_root);
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

/// Værten i UNC_ROOT (\\vært\share\…), bruges til smb://-mount på Linux.
fn smb_host(unc: &str) -> String {
    unc.trim_start_matches('\\').split('\\').next().unwrap_or_default().to_string()
}

async fn linux_file(State(app): State<Shared>, h: HeaderMap, Path(name): Path<String>) -> Response {
    let src = match name.as_str() {
        "install.sh" => include_str!("../linux/install.sh"),
        "spil-handler.sh" => include_str!("../linux/spil-handler.sh"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let body = src
        .replace("__SERVER__", &server(&h))
        .replace("__LINUX_ROOT__", &app.cfg.linux_root)
        .replace("__SMB_HOST__", &smb_host(&app.cfg.unc_root));
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

async fn css() -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=3600")],
     include_str!("../static/style.css")).into_response()
}
