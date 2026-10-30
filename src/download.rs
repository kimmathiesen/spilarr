//! Download gennem browseren: ISO'er som almindelige filer (med Range, så afbrudte downloads kan
//! genoptages) og hele spilmappen som zip, der streames uden komprimering og uden mellemfil.
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::services::ServeFile;
use walkdir::WalkDir;

use crate::{db, Shared};

/// `attachment` med både ASCII-navn og UTF-8-navn (RFC 6266).
fn disposition(name: &str) -> HeaderValue {
    let ascii: String = name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    HeaderValue::from_str(&format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", urlencoding::encode(name)))
        .unwrap_or(HeaderValue::from_static("attachment"))
}

#[derive(Deserialize)]
pub struct FileQ {
    f: String,
}

/// GET /spil/{id}/hent?f=<iso> – kun filer, scanneren har registreret som spillets ISO'er.
pub async fn file(State(app): State<Shared>, UrlPath(id): UrlPath<i64>, Query(q): Query<FileQ>, req: Request) -> Response {
    let Some(g) = db::get(&app.db.lock().unwrap(), id) else {
        return (StatusCode::NOT_FOUND, "Ukendt spil").into_response();
    };
    if !g.isos.contains(&q.f) {
        return (StatusCode::NOT_FOUND, "Ukendt fil").into_response();
    }
    let name = q.f.rsplit('/').next().unwrap_or(&q.f).to_string();
    match ServeFile::new_with_mime(app.cfg.games.join(&q.f), &mime_iso()).try_call(req).await {
        Ok(res) => {
            let mut res = res.map(Body::new);
            res.headers_mut().insert(header::CONTENT_DISPOSITION, disposition(&name));
            res
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn mime_iso() -> mime::Mime {
    "application/x-iso9660-image".parse().expect("mime")
}

/// Write-ende, der sender bidder videre til HTTP-svaret; fejler, når klienten har lukket.
struct ChanWriter(mpsc::Sender<io::Result<Bytes>>);

impl Write for ChanWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .blocking_send(Ok(Bytes::copy_from_slice(buf)))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "klienten afbrød"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// En fil i zip'en: sti på disken, navn i arkivet, størrelse og DOS-tid/-dato.
pub struct Entry {
    path: PathBuf,
    name: String,
    size: u64,
    dos: (u16, u16),
}

/// Alle filer under `dir`, sorteret; navnene i zip'en starter med mappenavnet.
pub fn entries(dir: &Path) -> io::Result<Vec<Entry>> {
    let top = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut out = vec![];
    for e in WalkDir::new(dir).min_depth(1).sort_by_file_name() {
        let e = e.map_err(io::Error::other)?;
        if !e.file_type().is_file() {
            continue;
        }
        let m = e.metadata().map_err(io::Error::other)?;
        let inner = e.path().strip_prefix(dir).unwrap_or(e.path());
        let secs = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        out.push(Entry {
            path: e.path().to_path_buf(),
            name: format!("{top}/{}", inner.to_string_lossy().replace('\\', "/")),
            size: m.len(),
            dos: dos_time(secs),
        });
    }
    Ok(out)
}

/// Unix-tid (UTC) → (DOS-tid, DOS-dato); før 1980 bliver til 1980-01-01.
fn dos_time(secs: u64) -> (u16, u16) {
    let (days, rem) = ((secs / 86400) as i64, (secs % 86400) as i64);
    // civil_from_days (H. Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    if y < 1980 {
        return (0, 0x21);
    }
    let time = (rem / 3600) << 11 | (rem % 3600 / 60) << 5 | (rem % 60) / 2;
    (time as u16, ((y - 1980).min(127) << 9 | m << 5 | d) as u16)
}

const LOCAL: u64 = 30 + 20; // lokal header + zip64-extra
const DESC: u64 = 24; // data descriptor (zip64)
const CENTRAL: u64 = 46 + 28; // central header + zip64-extra
const END: u64 = 56 + 20 + 22; // zip64 EOCD + locator + EOCD

/// Den præcise størrelse af zip'en, så browseren kan vise fremskridt.
pub fn zip_len(es: &[Entry]) -> u64 {
    es.iter().map(|e| LOCAL + DESC + CENTRAL + 2 * e.name.len() as u64 + e.size).sum::<u64>() + END
}

/// Tæller skrevne bytes (= offset i zip'en).
struct Count<W> {
    w: W,
    n: u64,
}

impl<W: Write> Count<W> {
    fn put(&mut self, b: &[u8]) -> io::Result<()> {
        self.w.write_all(b)?;
        self.n += b.len() as u64;
        Ok(())
    }
}

/// Skriver en zip64 uden komprimering (Stored) med data descriptors, så hver fil kun læses én gang
/// og CRC'en regnes undervejs. Flag 0x0808 = data descriptor + UTF-8-navne.
pub fn write_zip<W: Write>(es: &[Entry], w: W) -> io::Result<W> {
    let mut o = Count { w, n: 0 };
    let mut central = Vec::with_capacity(es.len());
    let mut buf = vec![0u8; 1 << 20];
    for e in es {
        let off = o.n;
        let mut h = Vec::with_capacity(LOCAL as usize + e.name.len());
        h.extend(0x04034b50u32.to_le_bytes());
        h.extend(45u16.to_le_bytes());
        h.extend(0x0808u16.to_le_bytes());
        h.extend(0u16.to_le_bytes()); // Stored
        h.extend(e.dos.0.to_le_bytes());
        h.extend(e.dos.1.to_le_bytes());
        h.extend(0u32.to_le_bytes()); // CRC står i data descriptor
        h.extend(u32::MAX.to_le_bytes());
        h.extend(u32::MAX.to_le_bytes());
        h.extend((e.name.len() as u16).to_le_bytes());
        h.extend(20u16.to_le_bytes());
        h.extend(e.name.as_bytes());
        h.extend(1u16.to_le_bytes());
        h.extend(16u16.to_le_bytes());
        h.extend(e.size.to_le_bytes());
        h.extend(e.size.to_le_bytes());
        o.put(&h)?;

        let mut f = std::fs::File::open(&e.path)?;
        let mut crc = crc32fast::Hasher::new();
        let mut n = 0u64;
        loop {
            let r = f.read(&mut buf)?;
            if r == 0 {
                break;
            }
            crc.update(&buf[..r]);
            o.put(&buf[..r])?;
            n += r as u64;
        }
        if n != e.size {
            return Err(io::Error::other(format!("{} ændrede størrelse under download", e.name)));
        }
        let crc = crc.finalize();
        let mut d = Vec::with_capacity(DESC as usize);
        d.extend(0x08074b50u32.to_le_bytes());
        d.extend(crc.to_le_bytes());
        d.extend(n.to_le_bytes());
        d.extend(n.to_le_bytes());
        o.put(&d)?;
        central.push((e, crc, off));
    }
    let cd_start = o.n;
    for (e, crc, off) in &central {
        let mut c = Vec::with_capacity(CENTRAL as usize + e.name.len());
        c.extend(0x02014b50u32.to_le_bytes());
        c.extend(45u16.to_le_bytes());
        c.extend(45u16.to_le_bytes());
        c.extend(0x0808u16.to_le_bytes());
        c.extend(0u16.to_le_bytes());
        c.extend(e.dos.0.to_le_bytes());
        c.extend(e.dos.1.to_le_bytes());
        c.extend(crc.to_le_bytes());
        c.extend(u32::MAX.to_le_bytes());
        c.extend(u32::MAX.to_le_bytes());
        c.extend((e.name.len() as u16).to_le_bytes());
        c.extend(28u16.to_le_bytes());
        c.extend([0u8; 6]); // kommentar-længde, disk, interne attributter
        c.extend(0u32.to_le_bytes()); // eksterne attributter
        c.extend(u32::MAX.to_le_bytes());
        c.extend(e.name.as_bytes());
        c.extend(1u16.to_le_bytes());
        c.extend(24u16.to_le_bytes());
        c.extend(e.size.to_le_bytes());
        c.extend(e.size.to_le_bytes());
        c.extend(off.to_le_bytes());
        o.put(&c)?;
    }
    let (cd_end, count) = (o.n, central.len() as u64);
    let mut z = Vec::with_capacity(END as usize);
    z.extend(0x06064b50u32.to_le_bytes());
    z.extend(44u64.to_le_bytes());
    z.extend(45u16.to_le_bytes());
    z.extend(45u16.to_le_bytes());
    z.extend([0u8; 8]); // disk-numre
    z.extend(count.to_le_bytes());
    z.extend(count.to_le_bytes());
    z.extend((cd_end - cd_start).to_le_bytes());
    z.extend(cd_start.to_le_bytes());
    z.extend(0x07064b50u32.to_le_bytes());
    z.extend(0u32.to_le_bytes());
    z.extend(cd_end.to_le_bytes());
    z.extend(1u32.to_le_bytes());
    z.extend(0x06054b50u32.to_le_bytes());
    z.extend([0u8; 4]);
    z.extend(u16::MAX.to_le_bytes());
    z.extend(u16::MAX.to_le_bytes());
    z.extend(u32::MAX.to_le_bytes());
    z.extend(u32::MAX.to_le_bytes());
    z.extend(0u16.to_le_bytes());
    o.put(&z)?;
    Ok(o.w)
}

/// GET /spil/{id}/zip – hele spilmappen (inkl. updates/) som én zip.
pub async fn folder(State(app): State<Shared>, UrlPath(id): UrlPath<i64>) -> Response {
    let Some(g) = db::get(&app.db.lock().unwrap(), id) else {
        return (StatusCode::NOT_FOUND, "Ukendt spil").into_response();
    };
    let dir = app.cfg.games.join(&g.rel);
    if !dir.is_dir() {
        return (StatusCode::NOT_FOUND, "Mappen findes ikke").into_response();
    }
    let d = dir.clone();
    let es = match tokio::task::spawn_blocking(move || entries(&d)).await {
        Ok(Ok(es)) => es,
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let len = zip_len(&es);
    let (tx, rx) = mpsc::channel::<io::Result<Bytes>>(16);
    tokio::task::spawn_blocking(move || {
        let w = BufWriter::with_capacity(1 << 20, ChanWriter(tx.clone()));
        if let Err(e) = write_zip(&es, w).and_then(|mut w| w.flush()) {
            eprintln!("zip af {}: {e}", dir.display());
            let _ = tx.blocking_send(Err(e));
        }
    });
    let name = format!("{}.zip", g.rel.rsplit('/').next().unwrap_or(&g.rel));
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("application/zip")),
            (header::CONTENT_DISPOSITION, disposition(&name)),
            (header::CONTENT_LENGTH, HeaderValue::from(len)),
        ],
        Body::from_stream(ReceiverStream::new(rx)),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("spilarr-zip-{}", std::process::id()));
        let d = tmp.join("Spil (2020) - GRP");
        std::fs::create_dir_all(d.join("updates/fix")).unwrap();
        std::fs::write(d.join("spil.iso"), vec![7u8; 300_000]).unwrap();
        std::fs::write(d.join("updates/fix/a.txt"), "hej").unwrap();
        let es = entries(&d).unwrap();
        let buf = write_zip(&es, Vec::new()).unwrap();
        assert_eq!(buf.len() as u64, zip_len(&es));
        let mut z = zip::ZipArchive::new(io::Cursor::new(buf)).unwrap();
        let mut names: Vec<_> = z.file_names().map(String::from).collect();
        names.sort();
        assert_eq!(names, ["Spil (2020) - GRP/spil.iso", "Spil (2020) - GRP/updates/fix/a.txt"]);
        let mut iso = vec![];
        z.by_name("Spil (2020) - GRP/spil.iso").unwrap().read_to_end(&mut iso).unwrap();
        assert_eq!(iso, vec![7u8; 300_000]);
        std::fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn dos_dates() {
        assert_eq!(dos_time(0), (0, 0x21));
        // 2026-10-29 13:45:10 UTC
        assert_eq!(dos_time(1793281510), (13 << 11 | 45 << 5 | 5, 46 << 9 | 10 << 5 | 29));
    }

    #[test]
    fn dispositions() {
        assert_eq!(disposition("rune-split.fiction.iso"), "attachment; filename=\"rune-split.fiction.iso\"; filename*=UTF-8''rune-split.fiction.iso");
        assert!(disposition("Æble \"x\".zip").to_str().unwrap().starts_with("attachment; filename=\"_ble _x_.zip\""));
    }
}
