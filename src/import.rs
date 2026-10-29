//! Fase 2: importerer færdige torrents fra qB-kategorien til spilmappen.
//! Almindelige filer hardlinkes, rar-arkiver pakkes ud. Blokerende – kaldes fra spawn_blocking.
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::{db, now, scan, steam, App};

/// Skjult mappe i spilmappen, hvor importen bygges, før den flyttes på plads (scanneren springer den over).
const TMP: &str = ".spilarr-import";

#[derive(Deserialize)]
struct Torrent {
    hash: String,
    name: String,
    content_path: String,
}

fn completed(app: &App) -> Result<Vec<Torrent>, String> {
    let url = format!("{}/api/v2/torrents/info", app.cfg.qbit_url.trim_end_matches('/'));
    steam::http()
        .get(url)
        .query(&[("category", app.cfg.qbit_category.as_str()), ("filter", "completed")])
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|e| format!("qBittorrent: {e}"))
}

fn set_msg(app: &App, msg: impl Into<String>) {
    app.status.lock().unwrap().msg = msg.into();
}

/// Spørger qB efter færdige torrents og importerer dem, der ikke er importeret før.
pub fn run(app: &App) {
    if app.cfg.qbit_url.is_empty() {
        return;
    }
    let Ok(_guard) = app.import_lock.try_lock() else { return };
    let list = match completed(app) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    let todo: Vec<Torrent> = {
        let c = app.db.lock().unwrap();
        list.into_iter().filter(|t| !db::imported(&c, &t.hash)).collect()
    };
    if todo.is_empty() {
        return;
    }
    for t in &todo {
        set_msg(app, format!("Importerer {}", t.name));
        let src = PathBuf::from(&t.content_path);
        let res = import(app, &src);
        let (ok, dest, msg) = match &res {
            Ok(d) => (true, d.clone(), String::new()),
            Err(e) => (false, String::new(), e.clone()),
        };
        eprintln!("import {}: {}", t.name, if ok { format!("ok → {dest}") } else { msg.clone() });
        let _ = db::save_import(&app.db.lock().unwrap(), &t.hash, &t.name, &t.content_path, &dest, ok, &msg, now());
    }
    set_msg(app, "");
    scan::scan_wait(app);
}

/// Bygger importen i en tmp-mappe og flytter den på plads. Returnerer det nye mappenavn.
fn import(app: &App, src: &Path) -> Result<String, String> {
    if !src.exists() {
        return Err(format!("{} findes ikke (forkert sti eller mount?)", src.display()));
    }
    let raw = src.file_name().unwrap_or_default().to_string_lossy().to_string();
    let name = dest_name(&raw, src.is_file());
    let games = &app.cfg.games;
    let dest = games.join(&name);
    if dest.exists() {
        return Err(format!("{name} findes allerede i spilmappen"));
    }
    let tmp = games.join(TMP).join(&name);
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).map_err(|e| format!("kan ikke rydde gammel tmp-mappe: {e}"))?;
    }
    std::fs::create_dir_all(&tmp).map_err(|e| format!("kan ikke oprette {}: {e}", tmp.display()))?;
    let res = fill(app, src, &tmp).and_then(|_| {
        std::fs::rename(&tmp, &dest).map_err(|e| format!("kan ikke flytte på plads: {e}"))
    });
    if res.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    res.map(|_| name)
}

/// Hardlinker alt undtagen rar-volumener og sample, og pakker første rar-volumen af hvert arkiv ud.
fn fill(app: &App, src: &Path, tmp: &Path) -> Result<(), String> {
    let files: Vec<(PathBuf, PathBuf)> = if src.is_file() {
        vec![(src.to_path_buf(), PathBuf::from(src.file_name().unwrap_or_default()))]
    } else {
        WalkDir::new(src)
            .into_iter()
            .filter_entry(|e| !(e.file_type().is_dir() && e.file_name().to_string_lossy().eq_ignore_ascii_case("sample")))
            .flatten()
            .filter(|e| e.file_type().is_file())
            .map(|e| (e.path().to_path_buf(), e.path().strip_prefix(src).unwrap_or(e.path()).to_path_buf()))
            .collect()
    };
    for (path, rel) in files {
        let lf = rel.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        let out = tmp.join(&rel);
        let dir = out.parent().unwrap_or(tmp).to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|e| format!("kan ikke oprette {}: {e}", dir.display()))?;
        match rar_volume(&lf) {
            Some(true) => {
                set_msg(app, format!("Pakker {} ud", rel.display()));
                extract(&path, &dir)?;
            }
            Some(false) => {}
            None if lf.ends_with(".sfv") => {}
            None => std::fs::hard_link(&path, &out)
                .map_err(|e| format!("kan ikke hardlinke {}: {e} (samme share/disk?)", rel.display()))?,
        }
    }
    Ok(())
}

/// Some(true) = første volumen, Some(false) = øvrige volumener (.partN.rar, .r00), None = ikke rar.
fn rar_volume(lf: &str) -> Option<bool> {
    if let Some(c) = Regex::new(r"\.part(\d+)\.rar$").expect("regex").captures(lf) {
        return Some(c[1].parse::<u32>().ok() == Some(1));
    }
    if lf.ends_with(".rar") {
        return Some(true);
    }
    Regex::new(r"\.r\d{2,3}$").expect("regex").is_match(lf).then_some(false)
}

/// Pakker et (evt. flerdelt) rar-arkiv ud i dir.
pub fn extract(first: &Path, dir: &Path) -> Result<(), String> {
    let err = |e: unrar::error::UnrarError| format!("unrar {}: {e}", first.display());
    let mut a = unrar::Archive::new(first).open_for_processing().map_err(err)?;
    while let Some(h) = a.read_header().map_err(err)? {
        a = if h.entry().is_file() { h.extract_with_base(dir).map_err(err)? } else { h.skip().map_err(err)? };
    }
    Ok(())
}

/// Ord i scene-navne, der ikke hører til titlen (alt fra første træf og frem skæres af).
const TAGS: &str = r"(?i)\b(multi\d*|repack|proper|update|v\d+(\.\d+)*|build \d+|x64|x86|iso|incl|dlc|crack(fix)?|readnfo|gog)\b.*$";

/// "Silent.Hill.2.Remake-RUNE" → "Silent Hill 2 Remake (2024) - RUNE". Navne, der allerede følger mønsteret, beholdes.
pub fn dest_name(raw: &str, is_file: bool) -> String {
    let mut s = raw.to_string();
    if is_file {
        if let Some(i) = s.rfind('.') {
            s.truncate(i);
        }
    }
    let s = s.trim();
    if Regex::new(r"^.+ \(\d{4}\)( - .+)?$").expect("regex").is_match(s) {
        return clean(s);
    }
    let (title, grp) = split_name(s);
    let year = steam::best_match(&title).and_then(|m| steam::details(m.0)).and_then(|m| year_of(&m.release));
    let mut out = title;
    if let Some(y) = year {
        out = format!("{out} ({y})");
    }
    if let Some(g) = grp {
        out = format!("{out} - {g}");
    }
    clean(&out)
}

/// Deler et scene-navn i titel og gruppe.
pub fn split_name(s: &str) -> (String, Option<String>) {
    let mut grp = None;
    let mut s = s.to_string();
    // [FitGirl Repack] og lignende
    if let Some(c) = Regex::new(r"\s*\[([^\]]+)\]\s*").expect("regex").captures(&s) {
        grp = Some(c[1].trim().to_string());
        s = s.replace(&c[0], " ");
    }
    // Gruppen er sidste "-Ord" uden mellemrum, fx "-RUNE"
    if let Some(c) = Regex::new(r"-([A-Za-z0-9]+)$").expect("regex").captures(s.trim()) {
        grp.get_or_insert_with(|| c[1].to_string());
        let n = s.trim().len() - c[0].len();
        s = s.trim()[..n].to_string();
    }
    let s = s.replace(['.', '_'], " ");
    let s = Regex::new(TAGS).expect("regex").replace(&s, "");
    let title = Regex::new(r"\s+").expect("regex").replace_all(s.trim(), " ").trim_matches([' ', '-']).to_string();
    (title, grp)
}

fn year_of(release: &str) -> Option<i64> {
    Regex::new(r"\b(19|20)\d{2}\b").expect("regex").find(release).and_then(|m| m.as_str().parse().ok())
}

/// Fjerner tegn, Windows ikke tillader i mappenavne.
fn clean(s: &str) -> String {
    s.chars().filter(|c| !r#"<>:"/\|?*"#.contains(*c)).collect::<String>().trim_end_matches(['.', ' ']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_names() {
        assert_eq!(split_name("Silent.Hill.2.Remake-RUNE"), ("Silent Hill 2 Remake".into(), Some("RUNE".into())));
        assert_eq!(split_name("Hollow.Knight.Silksong.v1.0.28324-TENOKE"), ("Hollow Knight Silksong".into(), Some("TENOKE".into())));
        assert_eq!(split_name("Cyberpunk 2077 [FitGirl Repack]"), ("Cyberpunk 2077".into(), Some("FitGirl Repack".into())));
        assert_eq!(split_name("Anno.1800.MULTi12-ElAmigos"), ("Anno 1800".into(), Some("ElAmigos".into())));
    }

    #[test]
    fn rar_volumes() {
        assert_eq!(rar_volume("rune-sh2.part01.rar"), Some(true));
        assert_eq!(rar_volume("rune-sh2.part02.rar"), Some(false));
        assert_eq!(rar_volume("game.rar"), Some(true));
        assert_eq!(rar_volume("game.r00"), Some(false));
        assert_eq!(rar_volume("game.iso"), None);
    }

    #[test]
    fn kept_names() {
        assert_eq!(dest_name("Anno 1800 (2019) - voices38", false), "Anno 1800 (2019) - voices38");
    }
}
