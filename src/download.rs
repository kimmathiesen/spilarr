//! Download gennem browseren: ISO'er som almindelige filer (med Range, så afbrudte downloads kan
//! genoptages) og hele spilmappen som zip, der streames uden komprimering og uden mellemfil.
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
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
    match ServeFile::new_with_mime(app.cfg.path(&q.f), &mime_iso()).try_call(req).await {
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
    mtime: u64,
    dos: (u16, u16),
}

/// CRC'er fra tidligere læsninger, så en genoptaget zip ikke skal læse de færdige filer igen.
pub trait Crcs {
    fn known(&self, path: &Path, size: u64, mtime: u64) -> Option<u32>;
    fn save(&self, path: &Path, size: u64, mtime: u64, crc: u32);
}

/// Uden gemte CRC'er (fx i tests)
pub struct NoCrcs;

impl Crcs for NoCrcs {
    fn known(&self, _: &Path, _: u64, _: u64) -> Option<u32> {
        None
    }
    fn save(&self, _: &Path, _: u64, _: u64, _: u32) {}
}

impl Crcs for Shared {
    fn known(&self, path: &Path, size: u64, mtime: u64) -> Option<u32> {
        db::crc_get(&self.db.lock().unwrap(), &path.to_string_lossy(), size, mtime)
    }
    fn save(&self, path: &Path, size: u64, mtime: u64, crc: u32) {
        let _ = db::crc_put(&self.db.lock().unwrap(), &path.to_string_lossy(), size, mtime, crc);
    }
}

fn mtime(m: &std::fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

/// CRC af en hel fil; gemmes, så den kun regnes én gang.
fn file_crc(c: &impl Crcs, path: &Path) -> io::Result<u32> {
    let m = std::fs::metadata(path)?;
    if let Some(v) = c.known(path, m.len(), mtime(&m)) {
        return Ok(v);
    }
    let (mut f, mut crc, mut buf) = (std::fs::File::open(path)?, crc32fast::Hasher::new(), vec![0u8; 1 << 20]);
    loop {
        let r = f.read(&mut buf)?;
        if r == 0 {
            break;
        }
        crc.update(&buf[..r]);
    }
    let v = crc.finalize();
    c.save(path, m.len(), mtime(&m), v);
    Ok(v)
}

/// GET /spil/{id}/crc?f=<iso> – så klienten kan tjekke en hentet fil.
pub async fn crc(State(app): State<Shared>, UrlPath(id): UrlPath<i64>, Query(q): Query<FileQ>) -> Response {
    let Some(g) = db::get(&app.db.lock().unwrap(), id) else {
        return (StatusCode::NOT_FOUND, "Ukendt spil").into_response();
    };
    if !g.isos.contains(&q.f) {
        return (StatusCode::NOT_FOUND, "Ukendt fil").into_response();
    }
    let path = app.cfg.path(&q.f);
    match tokio::task::spawn_blocking(move || file_crc(&app, &path)).await {
        Ok(Ok(v)) => format!("{v:08x}").into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
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
        let secs = mtime(&m);
        out.push(Entry {
            path: e.path().to_path_buf(),
            name: format!("{top}/{}", inner.to_string_lossy().replace('\\', "/")),
            size: m.len(),
            mtime: secs,
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

/// Tæller bytes (= offset i zip'en) og sender kun det, der ligger efter `skip`.
struct Count<W> {
    w: W,
    n: u64,
    skip: u64,
}

impl<W: Write> Count<W> {
    fn put(&mut self, b: &[u8]) -> io::Result<()> {
        let end = self.n + b.len() as u64;
        if end > self.skip {
            self.w.write_all(&b[self.skip.saturating_sub(self.n) as usize..])?;
        }
        self.n = end;
        Ok(())
    }
}

/// Skriver en zip64 uden komprimering (Stored) med data descriptors, så hver fil kun læses én gang
/// og CRC'en regnes undervejs. Flag 0x0808 = data descriptor + UTF-8-navne.
/// Fra offset `skip`: filer før det læses ikke, hvis deres CRC er kendt. Indholdet er det samme hver gang,
/// så længe mappen er uændret (filerne er sorteret, og tiderne kommer fra disken).
pub fn write_zip<W: Write>(es: &[Entry], skip: u64, crcs: &impl Crcs, w: W) -> io::Result<W> {
    let mut o = Count { w, n: 0, skip };
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

        // Spring det over, der allerede er sendt, hvis CRC'en er kendt; ellers skal filen læses for at regne den
        let known = crcs.known(&e.path, e.size, e.mtime);
        let from = if known.is_some() { o.skip.saturating_sub(o.n).min(e.size) } else { 0 };
        o.n += from;
        let (crc, n) = if from == e.size && e.size > 0 {
            (known.unwrap_or(0), e.size)
        } else {
            let mut f = std::fs::File::open(&e.path)?;
            f.seek(SeekFrom::Start(from))?;
            let mut h = crc32fast::Hasher::new();
            let mut n = from;
            loop {
                let r = f.read(&mut buf)?;
                if r == 0 {
                    break;
                }
                h.update(&buf[..r]);
                o.put(&buf[..r])?;
                n += r as u64;
            }
            let crc = known.unwrap_or_else(|| h.finalize());
            if n == e.size && known.is_none() {
                crcs.save(&e.path, e.size, e.mtime, crc);
            }
            (crc, n)
        };
        if n != e.size {
            return Err(io::Error::other(format!("{} ændrede størrelse under download", e.name)));
        }
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

/// `Range: bytes=N-` – den eneste form, en genoptagelse bruger.
fn range_start(h: &HeaderMap) -> Option<u64> {
    h.get(header::RANGE)?.to_str().ok()?.strip_prefix("bytes=")?.strip_suffix('-')?.parse().ok()
}

/// GET /spil/{id}/zip – hele spilmappen (inkl. updates/) som én zip. Kan genoptages med Range.
pub async fn folder(State(app): State<Shared>, UrlPath(id): UrlPath<i64>, headers: HeaderMap) -> Response {
    let Some(g) = db::get(&app.db.lock().unwrap(), id) else {
        return (StatusCode::NOT_FOUND, "Ukendt spil").into_response();
    };
    let dir = app.cfg.path(&g.rel);
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
    let skip = range_start(&headers).unwrap_or(0);
    if skip > 0 && skip >= len {
        return (StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{len}"))]).into_response();
    }
    let (tx, rx) = mpsc::channel::<io::Result<Bytes>>(16);
    let crcs = app.clone();
    tokio::task::spawn_blocking(move || {
        let w = BufWriter::with_capacity(1 << 20, ChanWriter(tx.clone()));
        if let Err(e) = write_zip(&es, skip, &crcs, w).and_then(|mut w| w.flush()) {
            eprintln!("zip af {}: {e}", dir.display());
            let _ = tx.blocking_send(Err(e));
        }
    });
    let name = format!("{}.zip", g.rel.rsplit('/').next().unwrap_or(&g.rel));
    let mut res = (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("application/zip")),
            (header::CONTENT_DISPOSITION, disposition(&name)),
            (header::CONTENT_LENGTH, HeaderValue::from(len - skip)),
            (header::ACCEPT_RANGES, HeaderValue::from_static("bytes")),
        ],
        Body::from_stream(ReceiverStream::new(rx)),
    )
        .into_response();
    if skip > 0 {
        *res.status_mut() = StatusCode::PARTIAL_CONTENT;
        if let Ok(v) = HeaderValue::from_str(&format!("bytes {skip}-{}/{len}", len - 1)) {
            res.headers_mut().insert(header::CONTENT_RANGE, v);
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Mem(std::sync::Mutex<std::collections::HashMap<PathBuf, u32>>);

    impl Crcs for Mem {
        fn known(&self, path: &Path, _: u64, _: u64) -> Option<u32> {
            self.0.lock().unwrap().get(path).copied()
        }
        fn save(&self, path: &Path, _: u64, _: u64, crc: u32) {
            self.0.lock().unwrap().insert(path.to_path_buf(), crc);
        }
    }

    #[test]
    fn zip_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("spilarr-zip-{}", std::process::id()));
        let d = tmp.join("Spil (2020) - GRP");
        std::fs::create_dir_all(d.join("updates/fix")).unwrap();
        std::fs::write(d.join("spil.iso"), vec![7u8; 300_000]).unwrap();
        std::fs::write(d.join("updates/fix/a.txt"), "hej").unwrap();
        let es = entries(&d).unwrap();
        let buf = write_zip(&es, 0, &NoCrcs, Vec::new()).unwrap();
        assert_eq!(buf.len() as u64, zip_len(&es));
        // Genoptagelse giver samme bytes, med og uden kendte CRC'er, og læser ikke de færdige filer
        let saved = Mem::default();
        write_zip(&es, 0, &saved, Vec::new()).unwrap();
        for skip in [1, 60, 300_000, 300_100, buf.len() as u64 - 3] {
            assert_eq!(write_zip(&es, skip, &NoCrcs, Vec::new()).unwrap(), buf[skip as usize..]);
            assert_eq!(write_zip(&es, skip, &saved, Vec::new()).unwrap(), buf[skip as usize..]);
        }
        let iso = es.iter().find(|e| e.name.ends_with(".iso")).unwrap();
        assert_eq!(file_crc(&NoCrcs, &iso.path).unwrap(), saved.known(&iso.path, iso.size, iso.mtime).unwrap());
        assert!(crate::klient::verify_zip_bytes(&buf).is_ok());
        let mut bad = buf.clone();
        bad[1000] ^= 1;
        assert!(crate::klient::verify_zip_bytes(&bad).is_err());
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
