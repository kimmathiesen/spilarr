//! SpilarrOS-klienten: `spilarr klient` kører på en Raspberry Pi i kiosk-tilstand. Den viser biblioteket fra
//! Spilarr-serveren (via Tailscale) og henter ISO'er eller hele mapper som zip ned på en ekstern disk med
//! en kø, der overlever afbrydelser og genstarter.
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path as UrlPath, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::krav::{self, Specs};

const GB: f64 = 1e9;

pub struct Cfg {
    pub server: String,
    pub dir: PathBuf,
    pub state: PathBuf,
    pub port: String,
    /// Kun 127.0.0.1 som standard – kiosken kører på samme maskine
    pub bind: String,
}

impl Cfg {
    pub fn from_env() -> Cfg {
        let e = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        Cfg {
            server: e("SERVER", "http://192.168.68.58:8790").trim_end_matches('/').to_string(),
            dir: e("DOWNLOAD_DIR", "/media/spildisk").into(),
            state: e("STATE_DIR", "/var/lib/spilarr-klient").into(),
            port: e("PORT", "8080"),
            bind: e("BIND", "127.0.0.1"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Tilstand {
    Venter,
    Henter,
    Faerdig,
    Fejl,
}

#[derive(Serialize, Deserialize, Clone)]
struct Job {
    n: u64,
    id: i64,
    title: String,
    /// Filnavnet, som vises i køen
    file: String,
    /// Sti på serveren, fx /spil/3/zip
    url: String,
    /// Sti på disken relativt til DOWNLOAD_DIR
    dest: String,
    zip: bool,
    size: u64,
    #[serde(default)]
    done: u64,
    state: Tilstand,
    #[serde(default)]
    msg: String,
    /// Unix-tid for næste forsøg efter en fejl
    #[serde(default)]
    retry: f64,
}

#[derive(Serialize, Clone, Default)]
struct Disk {
    ok: bool,
    fs: String,
    free: u64,
    total: u64,
    msg: String,
}

#[derive(Default)]
struct Live {
    /// Job, der hentes lige nu, og om det skal stoppes
    current: Option<u64>,
    stop: bool,
    speed: f64,
    server_ok: bool,
    server_msg: String,
}

pub struct K {
    cfg: Cfg,
    jobs: Mutex<Vec<Job>>,
    live: Mutex<Live>,
    specs: Mutex<Specs>,
    http: reqwest::Client,
}

type Shared = Arc<K>;

fn now() -> f64 {
    crate::now()
}

impl K {
    fn save(&self, jobs: &[Job]) {
        let p = self.cfg.state.join("jobs.json");
        let tmp = p.with_extension("tmp");
        if let Ok(s) = serde_json::to_vec_pretty(jobs) {
            if fs::write(&tmp, s).is_ok() {
                let _ = fs::rename(tmp, p);
            }
        }
    }

    fn update(&self, n: u64, f: impl FnOnce(&mut Job)) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(j) = jobs.iter_mut().find(|j| j.n == n) {
            f(j);
        }
        self.save(&jobs);
    }
}

/// Monteringen i DOWNLOAD_DIR: filsystem og plads. Kun en rigtig montering tæller (ikke SD-kortet).
fn disk(dir: &Path) -> Disk {
    let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let want = dir.to_string_lossy();
    let Some(fs_type) = mounts.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (_, mp, t) = (it.next()?, it.next()?, it.next()?);
        (mp.replace("\\040", " ") == want).then(|| t.to_string())
    }) else {
        return Disk { msg: "Ingen disk – sæt en USB-disk i".into(), ..Default::default() };
    };
    let (free, total) = statvfs(dir).unwrap_or((0, 0));
    let msg = if fs_type == "vfat" { "FAT32: kun filer under 4 GB".into() } else { String::new() };
    Disk { ok: true, fs: fs_type, free, total, msg }
}

fn statvfs(p: &Path) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(p.to_string_lossy().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let f = s.f_frsize as u64;
    Some((s.f_bavail as u64 * f, s.f_blocks as u64 * f))
}

pub async fn run(cfg: Cfg) {
    fs::create_dir_all(cfg.state.join("covers")).expect("kan ikke oprette STATE_DIR");
    let mut jobs: Vec<Job> = fs::read(cfg.state.join("jobs.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    // En download, der var i gang ved nedlukning, fortsætter
    for j in jobs.iter_mut().filter(|j| j.state == Tilstand::Henter) {
        j.state = Tilstand::Venter;
    }
    let addr = format!("{}:{}", cfg.bind, cfg.port);
    let specs = fs::read(cfg.state.join("specs.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let k: Shared = Arc::new(K {
        cfg,
        jobs: Mutex::new(jobs),
        live: Mutex::new(Live::default()),
        specs: Mutex::new(specs),
        http: reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).build().expect("http"),
    });

    let w = k.clone();
    std::thread::spawn(move || worker(&w));

    let c = k.clone();
    tokio::spawn(async move {
        loop {
            let r = c.http.get(format!("{}/status", c.cfg.server)).timeout(Duration::from_secs(8)).send().await;
            let (ok, msg) = match r {
                Ok(r) if r.status().is_success() => (true, String::new()),
                Ok(r) => (false, format!("Serveren svarer {}", r.status())),
                Err(_) => (false, "Ingen forbindelse hjem – tjek kabel og Tailscale".into()),
            };
            {
                let mut l = c.live.lock().unwrap();
                l.server_ok = ok;
                l.server_msg = msg;
            }
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });

    let app = Router::new()
        .route("/", get(page))
        .route("/static/style.css", get(css))
        .route("/api/spil", get(games))
        .route("/api/specs", get(get_specs).post(set_specs))
        .route("/img/{appid}/{kind}", get(img))
        .route("/api/status", get(status))
        .route("/api/hent", post(enqueue))
        .route("/api/job/{n}/fjern", post(remove))
        .route("/api/job/{n}/igen", post(retry))
        .route("/api/udskub", post(eject))
        .route("/api/sluk", post(poweroff))
        .with_state(k);
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("kan ikke binde port");
    eprintln!("SpilarrOS-klient lytter på {addr}");
    axum::serve(listener, app).await.expect("server fejlede");
}

// ---------- Download-tråden ----------

fn worker(k: &K) {
    let http = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(None)
        .build()
        .expect("http");
    loop {
        let next = {
            let jobs = k.jobs.lock().unwrap();
            jobs.iter()
                .find(|j| j.state == Tilstand::Venter || (j.state == Tilstand::Fejl && j.retry > 0.0 && j.retry <= now()))
                .cloned()
        };
        let Some(job) = next else {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        let d = disk(&k.cfg.dir);
        if !d.ok {
            k.update(job.n, |j| j.msg = d.msg.clone());
            std::thread::sleep(Duration::from_secs(5));
            continue;
        }
        {
            let mut l = k.live.lock().unwrap();
            l.current = Some(job.n);
            l.stop = false;
            l.speed = 0.0;
        }
        k.update(job.n, |j| {
            j.state = Tilstand::Henter;
            j.msg.clear();
        });
        let res = fetch(k, &http, &job, &d);
        let stopped = {
            let mut l = k.live.lock().unwrap();
            l.current = None;
            std::mem::take(&mut l.stop)
        };
        if stopped {
            continue; // fjernet fra køen imens
        }
        match res {
            Ok(()) => k.update(job.n, |j| {
                j.state = Tilstand::Faerdig;
                j.done = j.size;
                j.msg.clear();
            }),
            Err(e) => {
                eprintln!("{}: {e}", job.file);
                k.update(job.n, |j| {
                    j.state = Tilstand::Fejl;
                    j.msg = e;
                    j.retry = now() + 30.0;
                })
            }
        }
    }
}

/// Henter ét job til `<dest>.part` (fortsætter, hvis den findes) og omdøber til sidst.
fn fetch(k: &K, http: &reqwest::blocking::Client, job: &Job, d: &Disk) -> Result<(), String> {
    let dest = k.cfg.dir.join(&job.dest);
    if dest.metadata().is_ok_and(|m| m.len() == job.size) {
        return Ok(());
    }
    if d.fs == "vfat" && job.size >= 1 << 32 {
        return Err("Disken er FAT32 og kan ikke rumme filer over 4 GB – formatér den som exFAT".into());
    }
    let part = PathBuf::from(format!("{}.part", dest.display()));
    if let Some(p) = part.parent() {
        fs::create_dir_all(p).map_err(|e| format!("Kan ikke oprette mappe: {e}"))?;
    }
    // Zip'en kan ikke genoptages (indholdet laves undervejs) – start forfra
    let mut start = if job.zip { 0 } else { part.metadata().map(|m| m.len()).unwrap_or(0) };
    if start > job.size {
        start = 0;
    }
    let need = job.size - start;
    if d.free < need + (GB as u64 / 2) {
        return Err(format!("Ikke plads nok på disken ({:.1} GB fri, {:.1} GB mangler)", d.free as f64 / GB, need as f64 / GB));
    }
    let mut req = http.get(format!("{}{}", k.cfg.server, job.url));
    if start > 0 {
        req = req.header(header::RANGE, format!("bytes={start}-"));
    }
    let mut res = req.send().map_err(|_| "Ingen forbindelse til serveren".to_string())?;
    let status = res.status();
    if status == StatusCode::OK {
        start = 0;
    } else if status != StatusCode::PARTIAL_CONTENT {
        return Err(format!("Serveren svarede {status}"));
    }
    let mut f = OpenOptions::new().create(true).write(true).truncate(start == 0).open(&part)
        .map_err(|e| format!("Kan ikke skrive til disken: {e}"))?;
    if start > 0 {
        use std::io::Seek;
        f.seek(std::io::SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    }
    let mut done = start;
    let mut buf = vec![0u8; 1 << 20];
    let (mut t, mut last) = (Instant::now(), done);
    loop {
        if k.live.lock().unwrap().stop {
            return Ok(());
        }
        let r = res.read(&mut buf).map_err(|_| "Forbindelsen blev afbrudt".to_string())?;
        if r == 0 {
            break;
        }
        f.write_all(&buf[..r]).map_err(|e| format!("Skrivefejl på disken: {e}"))?;
        done += r as u64;
        let dt = t.elapsed().as_secs_f64();
        if dt >= 2.0 {
            let v = (done - last) as f64 / dt;
            {
                let mut l = k.live.lock().unwrap();
                l.speed = if l.speed == 0.0 { v } else { l.speed * 0.7 + v * 0.3 };
            }
            k.update(job.n, |j| j.done = done);
            (t, last) = (Instant::now(), done);
        }
    }
    f.sync_all().map_err(|e| format!("Skrivefejl på disken: {e}"))?;
    if done != job.size {
        return Err(format!("Ufuldstændig: {:.1} af {:.1} GB", done as f64 / GB, job.size as f64 / GB));
    }
    fs::rename(&part, &dest).map_err(|e| format!("Kan ikke omdøbe: {e}"))?;
    Ok(())
}

// ---------- HTTP ----------

async fn page() -> Html<&'static str> {
    Html(include_str!("../templates/klient.html"))
}

async fn css() -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], include_str!("../static/style.css")).into_response()
}

/// Biblioteket fra serveren; uden forbindelse vises den sidst hentede udgave.
async fn games(State(k): State<Shared>) -> Response {
    let cache = k.cfg.state.join("spil.json");
    let r = k.http.get(format!("{}/api/spil", k.cfg.server)).timeout(Duration::from_secs(15)).send().await;
    let body = match r {
        Ok(r) if r.status().is_success() => match r.bytes().await {
            Ok(b) => {
                let _ = tokio::fs::write(&cache, &b).await;
                b.to_vec()
            }
            Err(_) => tokio::fs::read(&cache).await.unwrap_or_else(|_| b"[]".to_vec()),
        },
        _ => tokio::fs::read(&cache).await.unwrap_or_else(|_| b"[]".to_vec()),
    };
    let body = med_vurdering(body, &k.specs.lock().unwrap());
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// Tilføjer `kan` og `kan_why` på hvert spil, når PC'ens specs er indtastet.
fn med_vurdering(body: Vec<u8>, pc: &Specs) -> Vec<u8> {
    if pc.is_empty() {
        return body;
    }
    let Ok(mut list) = serde_json::from_slice::<Vec<Value>>(&body) else { return body };
    let strs = |v: &Value| -> Vec<String> { v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default() };
    for g in list.iter_mut().filter(|g| g["kind"] != "software") {
        let (kan, why) = krav::vurder(pc, &strs(&g["req_min"]), &strs(&g["req_rec"]));
        g["kan"] = kan.into();
        g["kan_why"] = why.into();
    }
    serde_json::to_vec(&list).unwrap_or_default()
}

async fn get_specs(State(k): State<Shared>) -> Json<Value> {
    let names = |t: &[(&str, u32)]| { let mut v: Vec<String> = t.iter().map(|m| m.0.to_string()).collect(); v.sort(); v };
    Json(serde_json::json!({ "pc": *k.specs.lock().unwrap(), "cpus": names(krav::CPUS), "gpus": names(krav::GPUS) }))
}

async fn set_specs(State(k): State<Shared>, Json(pc): Json<Specs>) -> Response {
    let ok = (pc.cpu.is_empty() || krav::CPUS.iter().any(|m| m.0 == pc.cpu)) && (pc.gpu.is_empty() || krav::GPUS.iter().any(|m| m.0 == pc.gpu));
    if !ok {
        return (StatusCode::BAD_REQUEST, "Ukendt komponent").into_response();
    }
    if let Err(e) = serde_json::to_vec(&pc).map_err(std::io::Error::other).and_then(|b| fs::write(k.cfg.state.join("specs.json"), b)) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Kan ikke gemme: {e}")).into_response();
    }
    *k.specs.lock().unwrap() = pc;
    StatusCode::NO_CONTENT.into_response()
}

/// Plakater gemmes på SD-kortet, så de ikke hentes igen hver gang.
async fn img(State(k): State<Shared>, UrlPath((appid, kind)): UrlPath<(i64, String)>) -> Response {
    if kind != "poster" {
        return StatusCode::NOT_FOUND.into_response();
    }
    let p = k.cfg.state.join("covers").join(format!("{appid}_{kind}.jpg"));
    let bytes = match tokio::fs::read(&p).await {
        Ok(b) => b,
        Err(_) => {
            let r = k.http.get(format!("{}/img/{appid}/{kind}", k.cfg.server)).timeout(Duration::from_secs(20)).send().await;
            match r {
                Ok(r) if r.status().is_success() => match r.bytes().await {
                    Ok(b) => {
                        let _ = tokio::fs::write(&p, &b).await;
                        b.to_vec()
                    }
                    Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
                },
                _ => return StatusCode::NOT_FOUND.into_response(),
            }
        }
    };
    ([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "max-age=86400")], bytes).into_response()
}

#[derive(Serialize)]
struct JobOut {
    #[serde(flatten)]
    job: Job,
    speed: f64,
    eta: Option<f64>,
}

#[derive(Serialize)]
struct StatusOut {
    server_ok: bool,
    server_msg: String,
    server: String,
    disk: Disk,
    jobs: Vec<JobOut>,
}

async fn status(State(k): State<Shared>) -> Json<StatusOut> {
    let d = k.clone();
    let disk = tokio::task::spawn_blocking(move || disk(&d.cfg.dir)).await.unwrap_or_default();
    let l = k.live.lock().unwrap();
    let jobs = k.jobs.lock().unwrap().iter().map(|j| {
        let active = l.current == Some(j.n);
        let speed = if active { l.speed } else { 0.0 };
        let eta = (active && speed > 0.0).then(|| (j.size - j.done.min(j.size)) as f64 / speed);
        JobOut { job: j.clone(), speed, eta }
    }).collect();
    Json(StatusOut { server_ok: l.server_ok, server_msg: l.server_msg.clone(), server: k.cfg.server.clone(), disk, jobs })
}

#[derive(Deserialize)]
struct HentF {
    id: i64,
    #[serde(default)]
    f: Option<String>,
}

#[derive(Deserialize)]
struct ApiGame {
    id: i64,
    title: String,
    rel: String,
    isos: Vec<String>,
}

/// Sti på USB-disken: spil i roden (uden portable/), programmer under Software/.
fn disk_path(rel: &str) -> String {
    match rel.strip_prefix(crate::SOFTWARE) {
        Some(r) => format!("Software/{r}"),
        None => rel.strip_prefix("portable/").unwrap_or(rel).to_string(),
    }
}

fn base(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// Lægger en ISO (f) eller hele mappen (uden f) i køen. Størrelsen hentes med HEAD.
async fn enqueue(State(k): State<Shared>, Json(h): Json<HentF>) -> Response {
    let fail = |s: &str| (StatusCode::BAD_REQUEST, s.to_string()).into_response();
    let list: Vec<ApiGame> = match k.http.get(format!("{}/api/spil", k.cfg.server)).timeout(Duration::from_secs(15)).send().await {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(_) => return fail("Ingen forbindelse til serveren"),
    };
    let Some(g) = list.into_iter().find(|g| g.id == h.id) else { return fail("Ukendt spil") };
    let (url, dest, file, zip) = match &h.f {
        Some(f) if g.isos.contains(f) => {
            (format!("/spil/{}/hent?f={}", g.id, urlencoding::encode(f)), disk_path(f), base(f).to_string(), false)
        }
        Some(_) => return fail("Ukendt fil"),
        None => {
            let name = format!("{}.zip", base(&g.rel));
            (format!("/spil/{}/zip", g.id), disk_path(&format!("{}.zip", g.rel)), name, true)
        }
    };
    if let Some(j) = k.jobs.lock().unwrap().iter().find(|j| j.url == url && j.state != Tilstand::Faerdig) {
        return Json(serde_json::json!({ "n": j.n })).into_response();
    }
    let size = match k.http.head(format!("{}{url}", k.cfg.server)).timeout(Duration::from_secs(60)).send().await {
        // content_length() er 0 ved HEAD (ingen krop) – læs headeren
        Ok(r) if r.status().is_success() => r.headers().get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok()?.parse().ok())
            .unwrap_or(0),
        Ok(r) => return fail(&format!("Serveren svarede {}", r.status())),
        Err(_) => return fail("Ingen forbindelse til serveren"),
    };
    if size == 0 {
        return fail("Serveren oplyste ikke størrelsen");
    }
    let mut jobs = k.jobs.lock().unwrap();
    let n = jobs.iter().map(|j| j.n).max().unwrap_or(0) + 1;
    jobs.push(Job { n, id: g.id, title: g.title, file, url, dest, zip, size, done: 0, state: Tilstand::Venter, msg: String::new(), retry: 0.0 });
    k.save(&jobs);
    Json(serde_json::json!({ "n": n })).into_response()
}

/// Fjerner et job fra køen. En ufærdig download slettes; en færdig fil bliver liggende på disken.
async fn remove(State(k): State<Shared>, UrlPath(n): UrlPath<u64>) -> StatusCode {
    {
        let mut l = k.live.lock().unwrap();
        if l.current == Some(n) {
            l.stop = true;
        }
    }
    // Vent til download-tråden har sluppet filen
    for _ in 0..50 {
        if k.live.lock().unwrap().current != Some(n) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut jobs = k.jobs.lock().unwrap();
    if let Some(i) = jobs.iter().position(|j| j.n == n) {
        let j = jobs.remove(i);
        if j.state != Tilstand::Faerdig {
            let _ = fs::remove_file(format!("{}.part", k.cfg.dir.join(&j.dest).display()));
        }
    }
    k.save(&jobs);
    StatusCode::NO_CONTENT
}

async fn retry(State(k): State<Shared>, UrlPath(n): UrlPath<u64>) -> StatusCode {
    k.update(n, |j| {
        if j.state == Tilstand::Fejl {
            j.state = Tilstand::Venter;
            j.msg.clear();
        }
    });
    StatusCode::NO_CONTENT
}

fn sudo(args: &[&str]) -> Result<(), String> {
    let o = std::process::Command::new("sudo").arg("-n").args(args).output().map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// Skub disken ud, så den kan tages ud uden at miste data. Afvises, mens der hentes.
async fn eject(State(k): State<Shared>) -> Response {
    if k.live.lock().unwrap().current.is_some() {
        return (StatusCode::CONFLICT, "Der hentes stadig – fjern eller vent på downloaden først").into_response();
    }
    let dir = k.cfg.dir.to_string_lossy().to_string();
    let r = tokio::task::spawn_blocking(move || {
        let _ = std::process::Command::new("sync").status();
        sudo(&["umount", &dir])
    }).await.unwrap_or_else(|e| Err(e.to_string()));
    match r {
        Ok(()) => (StatusCode::OK, "Disken kan nu tages ud").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Kunne ikke skubbe disken ud: {e}")).into_response(),
    }
}

async fn poweroff(State(k): State<Shared>) -> Response {
    let jobs = k.jobs.lock().unwrap().clone();
    k.save(&jobs);
    let _ = std::process::Command::new("sync").status();
    match sudo(&["systemctl", "poweroff"]) {
        Ok(()) => (StatusCode::OK, "Slukker …").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_uden_montering() {
        let d = disk(Path::new("/findes/ikke"));
        assert!(!d.ok);
        assert!(statvfs(Path::new("/")).is_some_and(|(_, t)| t > 0));
    }

    #[test]
    fn disk_paths() {
        assert_eq!(disk_path("Spil (2020) - X/a.iso"), "Spil (2020) - X/a.iso");
        assert_eq!(disk_path("portable/Cyberpunk 2077 (2020).zip"), "Cyberpunk 2077 (2020).zip");
        assert_eq!(disk_path("@software/petka.zip"), "Software/petka.zip");
        assert_eq!(disk_path("@software/X/a.iso"), "Software/X/a.iso");
    }

    #[test]
    fn job_json() {
        let j = Job { n: 1, id: 2, title: "T".into(), file: "a.iso".into(), url: "/spil/2/hent?f=a".into(), dest: "X/a.iso".into(),
            zip: false, size: 10, done: 3, state: Tilstand::Faerdig, msg: String::new(), retry: 0.0 };
        let s = serde_json::to_string(&j).unwrap();
        assert!(s.contains("\"state\":\"faerdig\""));
        let back: Job = serde_json::from_str(&s).unwrap();
        assert!(back.state == Tilstand::Faerdig && back.done == 3);
    }
}
