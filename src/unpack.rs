//! Fase 3: pakker rar-arkiver ud i eksisterende spilmapper (kommandoen `spilarr unpack`).
//! Uden --apply vises kun, hvad der ville ske. Med --delete-rars slettes volumener og .sfv efter en vellykket udpakning.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::{import, scan};

const TMP: &str = ".spilarr-unpack";

struct Set {
    first: PathBuf,
    volumes: Vec<PathBuf>,
}

fn gb(b: u64) -> String {
    format!("{:.1} GB", b as f64 / 1e9)
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

#[cfg(unix)]
fn links(p: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).map(|m| m.nlink()).unwrap_or(1)
}

#[cfg(not(unix))]
fn links(_: &Path) -> u64 {
    1
}

/// Rar-sæt i en spilmappe (updates/ og sample/ ignoreres), grupperet pr. første volumen.
fn sets(d: &Path) -> Vec<Set> {
    let mut by_dir: BTreeMap<PathBuf, (Vec<PathBuf>, Vec<PathBuf>)> = BTreeMap::new();
    let walk = WalkDir::new(d).into_iter().filter_entry(|e| {
        let n = e.file_name().to_string_lossy().to_lowercase();
        !(e.file_type().is_dir() && e.depth() > 0 && (n == "sample" || n == TMP || (e.depth() == 1 && n == "updates")))
    });
    for e in walk.flatten().filter(|e| e.file_type().is_file()) {
        let lf = e.file_name().to_string_lossy().to_lowercase();
        let dir = e.path().parent().unwrap_or(d).to_path_buf();
        let entry = by_dir.entry(dir).or_default();
        match import::rar_volume(&lf) {
            Some(true) => entry.0.push(e.path().to_path_buf()),
            Some(false) => entry.1.push(e.path().to_path_buf()),
            None => {}
        }
    }
    let stem_re = regex::Regex::new(r"(\.part\d+)?\.(rar|[r-z]\d{2,3})$").expect("regex");
    // Volumener hører til det første volumen med samme stamme (før .partN / .rNN)
    let stem = |p: &Path| stem_re.replace(&p.file_name().unwrap_or_default().to_string_lossy().to_lowercase(), "").to_string();
    let mut out = vec![];
    for (_, (mut firsts, rest)) in by_dir {
        firsts.sort();
        for f in firsts {
            let s = stem(&f);
            let mut volumes: Vec<PathBuf> = rest.iter().filter(|v| stem(v) == s).cloned().collect();
            volumes.push(f.clone());
            volumes.sort();
            out.push(Set { first: f, volumes });
        }
    }
    out
}

fn list(first: &Path) -> Result<Vec<(String, u64)>, String> {
    let a = unrar::Archive::new(first).open_for_listing().map_err(|e| format!("unrar {}: {e}", first.display()))?;
    let mut v = vec![];
    for h in a {
        let h = h.map_err(|e| format!("unrar {}: {e}", first.display()))?;
        if h.is_file() {
            v.push((h.filename.to_string_lossy().to_string(), h.unpacked_size));
        }
    }
    Ok(v)
}

/// Størrelse fra rar-headeren; meget store tal betyder "ukendt".
fn known(sz: u64) -> Option<u64> {
    (sz < 1 << 50).then_some(sz)
}

/// Findes filen allerede med samme størrelse, regnes den som udpakket.
fn same(p: &Path, sz: Option<u64>) -> bool {
    sz.is_some_and(|sz| std::fs::metadata(p).is_ok_and(|m| m.len() == sz))
}

/// Udpakker sættene via en tmp-mappe og flytter filerne på plads. Filer, der allerede findes med samme
/// størrelse, beholdes som de er. Returnerer antal flyttede filer.
fn apply(d: &Path, sets: &[&Set]) -> Result<usize, String> {
    let tmp = d.join(TMP);
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).map_err(|e| format!("kan ikke rydde {}: {e}", tmp.display()))?;
    }
    let res = (|| {
        for s in sets {
            let rel = s.first.parent().unwrap_or(d).strip_prefix(d).unwrap_or(Path::new("")).to_path_buf();
            let out = tmp.join(&rel);
            std::fs::create_dir_all(&out).map_err(|e| format!("kan ikke oprette {}: {e}", out.display()))?;
            println!("  pakker {} ud", s.first.strip_prefix(d).unwrap_or(&s.first).display());
            import::extract(&s.first, &out)?;
        }
        let files: Vec<PathBuf> = WalkDir::new(&tmp).into_iter().flatten().filter(|e| e.file_type().is_file())
            .map(|e| e.path().strip_prefix(&tmp).unwrap_or(e.path()).to_path_buf()).collect();
        // Tjek for navnekonflikter, før noget flyttes
        if let Some(c) = files.iter().find(|f| d.join(f).exists() && !same(&d.join(f), Some(size(&tmp.join(f))))) {
            return Err(format!("{} findes allerede med en anden størrelse – intet flyttet", c.display()));
        }
        let mut n = 0;
        for f in &files {
            let dst = d.join(f);
            if dst.exists() {
                continue;
            }
            if let Some(p) = dst.parent() {
                std::fs::create_dir_all(p).map_err(|e| format!("kan ikke oprette {}: {e}", p.display()))?;
            }
            std::fs::rename(tmp.join(f), &dst).map_err(|e| format!("kan ikke flytte {}: {e}", f.display()))?;
            n += 1;
        }
        Ok(n)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    res
}

/// `spilarr unpack [--apply] [--delete-rars] [filter]`
pub fn cli(games: &Path, args: &[String]) -> i32 {
    let apply_ = args.iter().any(|a| a == "--apply");
    let delete = args.iter().any(|a| a == "--delete-rars");
    let filter = args.iter().find(|a| !a.starts_with("--")).map(|s| s.to_lowercase());
    if delete && !apply_ {
        eprintln!("--delete-rars kræver --apply");
        return 2;
    }
    println!("{} i {}{}", if apply_ { "UDPAKKER" } else { "DRY-RUN" }, games.display(), if delete { " (sletter rar bagefter)" } else { "" });
    let (mut total_rar, mut total_out, mut n, mut failed) = (0u64, 0u64, 0, 0);
    for (d, portable) in scan::folders(games) {
        let name = d.file_name().unwrap_or_default().to_string_lossy().to_string();
        if portable || filter.as_ref().is_some_and(|f| !name.to_lowercase().contains(f)) {
            continue;
        }
        let sets = sets(&d);
        if sets.is_empty() {
            continue;
        }
        n += 1;
        let vols: Vec<&PathBuf> = sets.iter().flat_map(|s| &s.volumes).collect();
        let rar: u64 = vols.iter().map(|v| size(v)).sum();
        let linked = vols.iter().filter(|v| links(v) > 1).count();
        total_rar += rar;
        println!("\n{name}\n  {} sæt, {} volumener, {}{}", sets.len(), vols.len(), gb(rar),
            if linked > 0 { format!(" – {linked} volumener har også et hardlink et andet sted") } else { String::new() });
        let mut ok = true;
        let mut todo: Vec<&Set> = vec![];
        for s in &sets {
            match list(&s.first) {
                Ok(files) => {
                    let mut need = false;
                    for (f, sz) in &files {
                        let sz = known(*sz);
                        let rel = s.first.parent().unwrap_or(&d).strip_prefix(&d).unwrap_or(Path::new("")).join(f);
                        let shown = sz.map_or("ukendt størrelse".into(), gb);
                        if same(&d.join(&rel), sz) {
                            println!("  = {} ({shown}) findes allerede – springes over", rel.display());
                        } else if d.join(&rel).exists() {
                            println!("  ! {} ({shown}) FINDES ALLEREDE MED ANDEN STØRRELSE", rel.display());
                            ok = false;
                        } else {
                            println!("  → {} ({shown})", rel.display());
                            total_out += sz.unwrap_or(rar);
                            need = true;
                        }
                    }
                    if need {
                        todo.push(s);
                    }
                }
                Err(e) => {
                    println!("  FEJL: {e}");
                    ok = false;
                }
            }
        }
        if !ok {
            println!("  {}", if apply_ { "SPRINGER OVER (fejl eller navnekonflikt)" } else { "ville blive sprunget over (fejl eller navnekonflikt)" });
            failed += 1;
            continue;
        }
        if !apply_ {
            continue;
        }
        match apply(&d, &todo) {
            Ok(moved) => {
                println!("  udpakket: {moved} filer");
                if delete {
                    let sfv: Vec<PathBuf> = sets.iter().filter_map(|s| s.first.parent()).flat_map(|p| {
                        std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path())
                            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sfv")))
                    }).collect();
                    let mut freed = 0;
                    for v in vols.iter().map(|v| v.as_path()).chain(sfv.iter().map(|p| p.as_path())) {
                        let sz = size(v);
                        match std::fs::remove_file(v) {
                            Ok(()) => freed += sz,
                            Err(e) => println!("  kan ikke slette {}: {e}", v.display()),
                        }
                    }
                    println!("  slettet rar/sfv: {}", gb(freed));
                }
            }
            Err(e) => {
                println!("  FEJL: {e} – stopper");
                return 1;
            }
        }
    }
    println!("\n{n} mapper, rar i alt {}, nye udpakkede filer ca. {}{}", gb(total_rar), gb(total_out),
        if failed > 0 { format!(", {failed} sprunget over") } else { String::new() });
    if failed > 0 { 1 } else { 0 }
}
