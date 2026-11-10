//! Scanner spilmappen og henter metadata. Blokerende – kaldes fra spawn_blocking.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use regex::Regex;
use walkdir::WalkDir;

use crate::db::{self, Scanned};
use crate::steam::{self, Meta};
use crate::{now, App};

/// Mapper, der indeholder flere spil i stedet for at være ét.
const CONTAINERS: [&str; 1] = ["portable"];

pub fn folders(games: &Path) -> Vec<(PathBuf, bool)> {
    let dirs = |p: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        v.retain(|p| !p.file_name().unwrap_or_default().to_string_lossy().starts_with('.'));
        v.sort();
        v
    };
    let mut out = vec![];
    for d in dirs(games) {
        let name = d.file_name().unwrap_or_default().to_string_lossy().to_string();
        if CONTAINERS.contains(&name.as_str()) {
            out.extend(dirs(&d).into_iter().map(|s| (s, true)));
        } else {
            out.push((d, false));
        }
    }
    out
}

fn rel(games: &Path, p: &Path) -> String {
    p.strip_prefix(games).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

/// ISO'er, første rar-volumen og installere (alt under updates/ og sample/ ignoreres).
fn analyze(games: &Path, d: &Path) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>, i64) {
    let part = Regex::new(r"\.part(\d+)\.rar$").expect("regex");
    let (mut isos, mut rars, mut setups, mut size) = (vec![], vec![], vec![], 0i64);
    let walk = WalkDir::new(d)
        .into_iter()
        .filter_entry(|e| !(e.file_type().is_dir() && e.file_name().to_string_lossy().eq_ignore_ascii_case("sample")));
    for e in walk.flatten().filter(|e| e.file_type().is_file()) {
        size += e.metadata().map(|m| m.len() as i64).unwrap_or(0);
        let inner = e.path().strip_prefix(d).unwrap_or(e.path());
        if inner.components().next().is_some_and(|c| c.as_os_str() == "updates") {
            continue;
        }
        let lf = e.file_name().to_string_lossy().to_lowercase();
        let r = rel(games, e.path());
        if lf.ends_with(".iso") {
            isos.push(r);
        } else if lf.ends_with(".rar") {
            let first = part.captures(&lf).map_or(true, |c| c[1].parse::<u32>().ok() == Some(1));
            if first {
                rars.push(r);
            }
        } else if lf.starts_with("setup") && lf.ends_with(".exe") && e.depth() <= 2 {
            setups.push(r);
        }
    }
    let mut updates: Vec<String> = std::fs::read_dir(d.join("updates"))
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    updates.sort();
    isos.sort();
    rars.sort();
    setups.sort_by_key(|s| (s.len(), s.clone()));
    (isos, rars, setups, updates, size)
}

fn kind_of(portable: bool, isos: &[String], rars: &[String], setups: &[String]) -> &'static str {
    if portable {
        "portable"
    } else if !isos.is_empty() {
        "iso"
    } else if !rars.is_empty() {
        "rar"
    } else if !setups.is_empty() {
        "setup"
    } else {
        "mappe"
    }
}

fn set_msg(app: &App, msg: impl Into<String>) {
    app.status.lock().unwrap().msg = msg.into();
}

pub fn scan(app: &App, force: bool) {
    let Ok(guard) = app.scan_lock.try_lock() else { return };
    run(app, force, guard);
}

/// Som scan, men venter, hvis en scanning allerede kører (bruges efter import).
pub fn scan_wait(app: &App) {
    let guard = app.scan_lock.lock().unwrap();
    run(app, false, guard);
}

fn run(app: &App, force: bool, _guard: std::sync::MutexGuard<()>) {
    {
        let mut s = app.status.lock().unwrap();
        s.scanning = true;
        s.msg = "Scanner mapper".into();
    }
    let name_re = Regex::new(r"^(?P<title>.+?)(?: \((?P<year>\d{4})\))?(?: - (?P<group>[^-()]+))?$").expect("regex");
    let games = &app.cfg.games;
    let t = now();
    let mut seen = HashSet::new();
    for (d, portable) in folders(games) {
        let r = rel(games, &d);
        seen.insert(r.clone());
        let name = d.file_name().unwrap_or_default().to_string_lossy().to_string();
        // En ny update ændrer kun updates/, ikke selve spilmappen
        let mtime_of = |p: &Path| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .map(|m| m.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64())
                .unwrap_or(0.0)
        };
        let mtime = mtime_of(&d).max(mtime_of(&d.join("updates")));
        let old = db::find(&app.db.lock().unwrap(), &r);
        if let Some((_, omt)) = old {
            if (omt - mtime).abs() < 0.001 && !force {
                continue;
            }
        }
        set_msg(app, format!("Scanner {name}"));
        let (title, year, grp) = match name_re.captures(&name) {
            Some(c) => (c["title"].to_string(), c.name("year").and_then(|y| y.as_str().parse().ok()), c.name("group").map(|g| g.as_str().to_string())),
            None => (name.clone(), None, None),
        };
        let (isos, rars, setups, updates, size) = analyze(games, &d);
        let s = Scanned {
            kind: kind_of(portable, &isos, &rars, &setups).into(),
            title, year, grp, portable, isos, rars, setups, updates, size, mtime,
        };
        if let Err(e) = db::save_scanned(&app.db.lock().unwrap(), old.map(|o| o.0), &r, &s, t) {
            eprintln!("kan ikke gemme {r}: {e}");
        }
    }
    if let Some(sw) = &app.cfg.software {
        scan_software(app, sw, force, t, &mut seen);
    }
    for (sys, dir) in &app.cfg.roms {
        scan_roms(app, sys, dir, force, t, &mut seen);
    }
    // Fjern spil, hvis mappe er væk – men aldrig alt på én gang (fx hvis arrayet er stoppet).
    let todo = {
        let c = app.db.lock().unwrap();
        if !seen.is_empty() {
            let _ = db::delete_missing(&c, &seen);
            let _ = db::touch_all(&c, t);
        }
        db::todo(&c, t - 7.0 * 86400.0).unwrap_or_default()
    };
    for (i, id) in todo.iter().enumerate() {
        set_msg(app, format!("Henter metadata ({}/{})", i + 1, todo.len()));
        fetch_meta(app, *id, None, false);
        std::thread::sleep(Duration::from_millis(1500));
    }
    let todo = db::todo_reqs(&app.db.lock().unwrap()).unwrap_or_default();
    for (i, id) in todo.iter().enumerate() {
        set_msg(app, format!("Henter systemkrav ({}/{})", i + 1, todo.len()));
        fetch_reqs(app, *id);
        std::thread::sleep(Duration::from_millis(1500));
    }
    let mut s = app.status.lock().unwrap();
    s.scanning = false;
    s.last = Some(now());
    s.msg.clear();
}

/// Hver mappe i SOFTWARE_DIR er ét program. Ingen metadata; ISO'er findes, så de kan hentes enkeltvis.
fn scan_software(app: &App, sw: &Path, force: bool, t: f64, seen: &mut HashSet<String>) {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(sw)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    dirs.retain(|p| !p.file_name().unwrap_or_default().to_string_lossy().starts_with('.'));
    dirs.sort();
    for d in dirs {
        let name = d.file_name().unwrap_or_default().to_string_lossy().to_string();
        let r = format!("{}{name}", crate::SOFTWARE);
        seen.insert(r.clone());
        let mtime = std::fs::metadata(&d)
            .and_then(|m| m.modified())
            .map(|m| m.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64())
            .unwrap_or(0.0);
        let old = db::find(&app.db.lock().unwrap(), &r);
        if let Some((_, omt)) = old {
            if (omt - mtime).abs() < 0.001 && !force {
                continue;
            }
        }
        set_msg(app, format!("Scanner {name}"));
        let (mut isos, mut size) = (vec![], 0i64);
        for e in WalkDir::new(&d).into_iter().flatten().filter(|e| e.file_type().is_file()) {
            size += e.metadata().map(|m| m.len() as i64).unwrap_or(0);
            if e.file_name().to_string_lossy().to_lowercase().ends_with(".iso") {
                isos.push(format!("{}{}", crate::SOFTWARE, rel(sw, e.path())));
            }
        }
        isos.sort();
        let s = Scanned {
            kind: "software".into(), title: name, year: None, grp: None, portable: false,
            isos, rars: vec![], setups: vec![], updates: vec![], size, mtime,
        };
        if let Err(e) = db::save_scanned(&app.db.lock().unwrap(), old.map(|o| o.0), &r, &s, t) {
            eprintln!("kan ikke gemme {r}: {e}");
        }
    }
}

const ROM_EXT: [&str; 7] = ["bin", "cue", "iso", "chd", "pbp", "img", "pkg"];

/// Spillet, en rom-fil hører til: navnet uden endelse og uden " (Track 2)". None = ikke en rom.
pub fn rom_key(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    if !ROM_EXT.contains(&ext.to_lowercase().as_str()) {
        return None;
    }
    static TRACK: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new(r"(?i) \(track \d+\)$").expect("regex"));
    Some(TRACK.replace(stem, "").to_string())
}

/// Filerne i `dir`, der hører til spillet `key` (fx en .cue og dens .bin'er).
pub fn rom_group(dir: &Path, key: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    v.retain(|p| rom_key(&p.file_name().unwrap_or_default().to_string_lossy()).as_deref() == Some(key));
    v.sort();
    v
}

/// Ét spil pr. mappe eller pr. gruppe af rom-filer. Én fil kan hentes direkte; flere hentes som zip.
fn scan_roms(app: &App, sys: &str, dir: &Path, force: bool, t: f64, seen: &mut HashSet<String>) {
    let mut items: std::collections::BTreeMap<String, Option<Vec<PathBuf>>> = Default::default();
    for e in std::fs::read_dir(dir).map(|rd| rd.flatten().collect::<Vec<_>>()).unwrap_or_default() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if e.path().is_dir() {
            items.entry(name).or_insert(None);
        } else if let Some(k) = rom_key(&name) {
            if let Some(files) = items.entry(k).or_insert_with(|| Some(vec![])) {
                files.push(e.path());
            }
        }
    }
    let mtime_of = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .map(|m| m.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64())
            .unwrap_or(0.0)
    };
    for (key, files) in items {
        let r = format!("{}{sys}/{key}", crate::ROMS);
        seen.insert(r.clone());
        let mtime = match &files {
            Some(f) => f.iter().map(|p| mtime_of(p)).fold(0.0, f64::max),
            None => mtime_of(&dir.join(&key)),
        };
        let old = db::find(&app.db.lock().unwrap(), &r);
        if let Some((_, omt)) = old {
            if (omt - mtime).abs() < 0.001 && !force {
                continue;
            }
        }
        set_msg(app, format!("Scanner {key}"));
        let (isos, size) = match &files {
            Some(f) => {
                let size = f.iter().map(|p| std::fs::metadata(p).map(|m| m.len() as i64).unwrap_or(0)).sum();
                let isos = if f.len() == 1 { vec![format!("{}{sys}/{}", crate::ROMS, rel(dir, &f[0]))] } else { vec![] };
                (isos, size)
            }
            None => {
                let walk = WalkDir::new(dir.join(&key)).into_iter().flatten().filter(|e| e.file_type().is_file());
                (vec![], walk.map(|e| e.metadata().map(|m| m.len() as i64).unwrap_or(0)).sum())
            }
        };
        let s = Scanned {
            kind: "rom".into(), title: key, year: None, grp: Some(sys.to_string()), portable: false,
            isos, rars: vec![], setups: vec![], updates: vec![], size, mtime,
        };
        if let Err(e) = db::save_scanned(&app.db.lock().unwrap(), old.map(|o| o.0), &r, &s, t) {
            eprintln!("kan ikke gemme {r}: {e}");
        }
    }
}

fn save_images(app: &App, m: &mut Meta) {
    let covers = app.cfg.covers();
    let mut poster = steam::image(m.appid, &["library_600x900_2x.jpg", "library_600x900.jpg"]);
    if poster.is_none() {
        poster = m.header.as_deref().and_then(steam::get_image);
        m.poster_wide = poster.is_some();
    }
    if let Some(b) = poster {
        let _ = std::fs::write(covers.join(format!("{}_poster.jpg", m.appid)), b);
    }
    let hero = steam::image(m.appid, &["library_hero.jpg"]).or_else(|| m.background.as_deref().and_then(steam::get_image));
    if let Some(b) = hero {
        let _ = std::fs::write(covers.join(format!("{}_hero.jpg", m.appid)), b);
    }
}

/// Tilføjer systemkrav til eksisterende metadata uden at røre match og billeder. Fejler Steam, prøves igen ved næste scanning.
fn fetch_reqs(app: &App, id: i64) {
    let Some(g) = db::get(&app.db.lock().unwrap(), id) else { return };
    let (Some(appid), Some(mut m)) = (g.appid, g.meta) else { return };
    let Some(n) = steam::details(appid) else { return };
    m.req_min = n.req_min;
    m.req_rec = n.req_rec;
    if let Err(e) = db::set_meta(&app.db.lock().unwrap(), id, Some(appid), Some(&m), g.locked, now()) {
        eprintln!("kan ikke gemme systemkrav for {id}: {e}");
    }
}

/// query: None = automatisk match, tal = Steam-appid, "ingen" = ingen metadata, ellers søgetekst.
pub fn fetch_meta(app: &App, id: i64, query: Option<String>, lock: bool) {
    let Some(g) = db::get(&app.db.lock().unwrap(), id) else { return };
    let q = query.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let appid = match q.as_deref() {
        Some(s) if s.eq_ignore_ascii_case("ingen") => None,
        Some(s) if s.chars().all(|c| c.is_ascii_digit()) => s.parse().ok(),
        Some(s) => steam::best_match(s).map(|m| m.0),
        None => steam::best_match(&g.title).map(|m| m.0),
    };
    let mut meta = appid.and_then(steam::details);
    if let Some(m) = meta.as_mut() {
        save_images(app, m);
    }
    let appid = meta.as_ref().map(|m| m.appid);
    if let Err(e) = db::set_meta(&app.db.lock().unwrap(), id, appid, meta.as_ref(), lock || g.locked, now()) {
        eprintln!("kan ikke gemme metadata for {id}: {e}");
    }
}
