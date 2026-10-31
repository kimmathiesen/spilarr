//! Spilarr – Radarr-agtigt bibliotek over en spil-share med plakater, infoside og spil://-knapper.
mod db;
mod download;
mod import;
mod klient;
mod krav;
mod scan;
mod steam;
mod unpack;
mod web;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct Config {
    pub games: PathBuf,
    pub config: PathBuf,
    pub unc_root: String,
    /// Spilmappen på Linux-klienter, hvis install.sh ikke selv finder den.
    pub linux_root: String,
    pub scan_hours: f64,
    /// Tom = import fra qBittorrent er slået fra.
    pub qbit_url: String,
    pub qbit_category: String,
    pub import_minutes: f64,
    /// Mappe med programmer (ikke spil); tom = slået fra
    pub software: Option<PathBuf>,
    pub software_unc: String,
}

/// Software gemmes i samme tabel som spil, med rel = "@software/<mappe>".
pub const SOFTWARE: &str = "@software/";

impl Config {
    pub fn covers(&self) -> PathBuf {
        self.config.join("covers")
    }

    /// Sti på disken for en rel-sti fra databasen (spil eller software).
    pub fn path(&self, rel: &str) -> PathBuf {
        match (rel.strip_prefix(SOFTWARE), &self.software) {
            (Some(r), Some(s)) => s.join(r),
            _ => self.games.join(rel),
        }
    }

    /// Windows-sti til en rel-sti.
    pub fn unc(&self, rel: &str) -> String {
        match rel.strip_prefix(SOFTWARE) {
            Some(r) => format!("{}\\{}", self.software_unc, r.replace('/', "\\")),
            None => format!("{}\\{}", self.unc_root, rel.replace('/', "\\")),
        }
    }
}

#[derive(Default, Clone, serde::Serialize)]
pub struct Status {
    pub scanning: bool,
    pub last: Option<f64>,
    pub msg: String,
}

pub struct App {
    pub cfg: Config,
    pub db: Mutex<rusqlite::Connection>,
    pub status: Mutex<Status>,
    pub scan_lock: Mutex<()>,
    pub import_lock: Mutex<()>,
}

pub type Shared = Arc<App>;

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

#[tokio::main]
async fn main() {
    let cfg = Config {
        games: env("GAMES_DIR", "/games").into(),
        config: env("CONFIG_DIR", "/config").into(),
        unc_root: env("UNC_ROOT", r"\\192.168.68.58\data\games"),
        linux_root: env("LINUX_ROOT", "/mnt/spil"),
        scan_hours: env("SCAN_HOURS", "6").parse().unwrap_or(6.0),
        qbit_url: env("QBIT_URL", ""),
        qbit_category: env("QBIT_CATEGORY", "games"),
        import_minutes: env("IMPORT_MINUTES", "5").parse().unwrap_or(5.0),
        software: Some(env("SOFTWARE_DIR", "")).filter(|s| !s.is_empty()).map(PathBuf::from),
        software_unc: env("SOFTWARE_UNC", r"\\192.168.68.58\data\software"),
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "klient") {
        return klient::run(klient::Cfg::from_env()).await;
    }
    if args.first().is_some_and(|a| a == "unpack") {
        std::process::exit(unpack::cli(&cfg.games, &args[1..]));
    }
    std::fs::create_dir_all(cfg.covers()).expect("kan ikke oprette config/covers");
    let conn = db::open(&cfg.config.join("spilarr.db")).expect("kan ikke åbne databasen");
    let app: Shared = Arc::new(App {
        cfg,
        db: Mutex::new(conn),
        status: Mutex::new(Status::default()),
        scan_lock: Mutex::new(()),
        import_lock: Mutex::new(()),
    });

    let bg = app.clone();
    tokio::spawn(async move {
        loop {
            let a = bg.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || scan::scan(&a, false)).await {
                eprintln!("scan fejlede: {e}");
            }
            tokio::time::sleep(Duration::from_secs_f64(bg.cfg.scan_hours * 3600.0)).await;
        }
    });

    if !app.cfg.qbit_url.is_empty() {
        let bg = app.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs_f64(bg.cfg.import_minutes * 60.0)).await;
                let a = bg.clone();
                if let Err(e) = tokio::task::spawn_blocking(move || import::run(&a)).await {
                    eprintln!("import fejlede: {e}");
                }
            }
        });
    }

    let port = env("PORT", "8790");
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await.expect("kan ikke binde port");
    eprintln!("Spilarr lytter på :{port}");
    axum::serve(listener, web::router(app)).await.expect("server fejlede");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stier() {
        let c = Config {
            games: "/data/games".into(), config: "/config".into(), unc_root: r"\t\data\games".into(), linux_root: String::new(),
            scan_hours: 6.0, qbit_url: String::new(), qbit_category: String::new(), import_minutes: 5.0,
            software: Some("/data/software".into()), software_unc: r"\t\data\software".into(),
        };
        assert_eq!(c.path("Spil/a.iso"), PathBuf::from("/data/games/Spil/a.iso"));
        assert_eq!(c.path("@software/petka/x.iso"), PathBuf::from("/data/software/petka/x.iso"));
        assert_eq!(c.unc("portable/Cyberpunk"), r"\t\data\games\portable\Cyberpunk");
        assert_eq!(c.unc("@software/petka"), r"\t\data\software\petka");
    }
}
