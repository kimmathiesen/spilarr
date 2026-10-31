//! SQLite-lager. Skemaet er det samme som Python-prototypen brugte.
use std::collections::HashSet;
use std::path::Path;

use rusqlite::{params, Connection, Row};

use crate::steam::Meta;

pub fn open(p: &Path) -> rusqlite::Result<Connection> {
    let c = Connection::open(p)?;
    c.execute_batch(
        "pragma journal_mode=wal;
         create table if not exists games(
            id integer primary key, rel text unique, title text, year int, grp text, portable int,
            kind text, isos text, rars text, setups text, updates text, size int, mtime real,
            added real, seen real, appid int, meta text, locked int default 0, tried real);
         create table if not exists imports(
            hash text primary key, name text, src text, dest text, ok int, msg text, time real);
         create table if not exists crcs(path text primary key, size int, mtime int, crc int)",
    )?;
    Ok(c)
}

/// Gemt CRC, så længe filen er uændret.
pub fn crc_get(c: &Connection, path: &str, size: u64, mtime: u64) -> Option<u32> {
    c.query_row("select crc from crcs where path=? and size=? and mtime=?", params![path, size as i64, mtime as i64], |r| r.get(0)).ok()
}

pub fn crc_put(c: &Connection, path: &str, size: u64, mtime: u64, crc: u32) -> rusqlite::Result<()> {
    c.execute("insert or replace into crcs(path, size, mtime, crc) values(?,?,?,?)", params![path, size as i64, mtime as i64, crc])?;
    Ok(())
}

#[derive(Clone)]
pub struct Game {
    pub id: i64,
    pub rel: String,
    pub title: String,
    pub year: Option<i64>,
    pub grp: Option<String>,
    pub kind: String,
    pub isos: Vec<String>,
    pub rars: Vec<String>,
    pub setups: Vec<String>,
    pub updates: Vec<String>,
    pub size: i64,
    pub appid: Option<i64>,
    pub meta: Option<Meta>,
    pub locked: bool,
}

/// Resultatet af at scanne én spilmappe.
pub struct Scanned {
    pub title: String,
    pub year: Option<i64>,
    pub grp: Option<String>,
    pub portable: bool,
    pub kind: String,
    pub isos: Vec<String>,
    pub rars: Vec<String>,
    pub setups: Vec<String>,
    pub updates: Vec<String>,
    pub size: i64,
    pub mtime: f64,
}

fn list_col(s: Option<String>) -> Vec<String> {
    s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn js(v: &[String]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

fn from_row(r: &Row) -> rusqlite::Result<Game> {
    Ok(Game {
        id: r.get("id")?,
        rel: r.get("rel")?,
        title: r.get("title")?,
        year: r.get("year")?,
        grp: r.get("grp")?,
        kind: r.get("kind")?,
        isos: list_col(r.get("isos")?),
        rars: list_col(r.get("rars")?),
        setups: list_col(r.get("setups")?),
        updates: list_col(r.get("updates")?),
        size: r.get::<_, Option<i64>>("size")?.unwrap_or(0),
        appid: r.get("appid")?,
        meta: r.get::<_, Option<String>>("meta")?.and_then(|s| serde_json::from_str(&s).ok()),
        locked: r.get::<_, Option<i64>>("locked")?.unwrap_or(0) != 0,
    })
}

pub fn list(c: &Connection, order: &str) -> rusqlite::Result<Vec<Game>> {
    let mut s = c.prepare(&format!("select * from games order by {order}"))?;
    let rows = s.query_map([], from_row)?;
    rows.collect()
}

pub fn get(c: &Connection, id: i64) -> Option<Game> {
    c.query_row("select * from games where id=?1", [id], from_row).ok()
}

pub fn find(c: &Connection, rel: &str) -> Option<(i64, f64)> {
    c.query_row("select id, mtime from games where rel=?1", [rel], |r| Ok((r.get(0)?, r.get(1)?)))
        .ok()
}

pub fn touch(c: &Connection, id: i64, now: f64) -> rusqlite::Result<()> {
    c.execute("update games set seen=?1 where id=?2", params![now, id]).map(|_| ())
}

pub fn save_scanned(c: &Connection, id: Option<i64>, rel: &str, s: &Scanned, now: f64) -> rusqlite::Result<()> {
    let (isos, rars, setups, updates) = (js(&s.isos), js(&s.rars), js(&s.setups), js(&s.updates));
    match id {
        Some(id) => c.execute(
            "update games set title=?1,year=?2,grp=?3,portable=?4,kind=?5,isos=?6,rars=?7,setups=?8,updates=?9,
             size=?10,mtime=?11,seen=?12 where id=?13",
            params![s.title, s.year, s.grp, s.portable as i64, s.kind, isos, rars, setups, updates, s.size, s.mtime, now, id],
        ),
        None => c.execute(
            "insert into games(title,year,grp,portable,kind,isos,rars,setups,updates,size,mtime,seen,rel,added)
             values(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![s.title, s.year, s.grp, s.portable as i64, s.kind, isos, rars, setups, updates, s.size, s.mtime, now, rel, s.mtime],
        ),
    }
    .map(|_| ())
}

pub fn delete_missing(c: &Connection, seen: &HashSet<String>) -> rusqlite::Result<usize> {
    let rels: Vec<String> = c
        .prepare("select rel from games")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut n = 0;
    for rel in rels.iter().filter(|r| !seen.contains(*r)) {
        n += c.execute("delete from games where rel=?1", [rel])?;
    }
    Ok(n)
}

/// Spil uden metadata, som ikke er låst og ikke er forsøgt den seneste uge.
pub fn todo(c: &Connection, before: f64) -> rusqlite::Result<Vec<i64>> {
    c.prepare("select id from games where locked=0 and meta is null and kind!='software' and (tried is null or tried<?1)")?
        .query_map([before], |r| r.get(0))?
        .collect()
}

/// Spil, hvis metadata er hentet, før systemkrav kom med.
pub fn todo_reqs(c: &Connection) -> rusqlite::Result<Vec<i64>> {
    c.prepare("select id from games where appid is not null and meta is not null and meta not like '%\"req_min\":%'")?
        .query_map([], |r| r.get(0))?
        .collect()
}

pub fn set_meta(c: &Connection, id: i64, appid: Option<i64>, meta: Option<&Meta>, locked: bool, now: f64) -> rusqlite::Result<()> {
    let m = meta.and_then(|m| serde_json::to_string(m).ok());
    c.execute(
        "update games set appid=?1, meta=?2, tried=?3, locked=?4 where id=?5",
        params![appid, m, now, locked as i64, id],
    )
    .map(|_| ())
}

pub struct Import {
    pub hash: String,
    pub name: String,
    pub dest: String,
    pub ok: bool,
    pub msg: String,
    pub time: f64,
}

pub fn imported(c: &Connection, hash: &str) -> bool {
    c.query_row("select 1 from imports where hash=?1", [hash], |_| Ok(())).is_ok()
}

#[allow(clippy::too_many_arguments)]
pub fn save_import(c: &Connection, hash: &str, name: &str, src: &str, dest: &str, ok: bool, msg: &str, now: f64) -> rusqlite::Result<()> {
    c.execute(
        "insert or replace into imports(hash,name,src,dest,ok,msg,time) values(?1,?2,?3,?4,?5,?6,?7)",
        params![hash, name, src, dest, ok as i64, msg, now],
    )
    .map(|_| ())
}

pub fn imports(c: &Connection) -> rusqlite::Result<Vec<Import>> {
    c.prepare("select hash,name,dest,ok,msg,time from imports order by time desc limit 100")?
        .query_map([], |r| {
            Ok(Import {
                hash: r.get(0)?,
                name: r.get(1)?,
                dest: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ok: r.get::<_, i64>(3)? != 0,
                msg: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                time: r.get(5)?,
            })
        })?
        .collect()
}

pub fn forget_import(c: &Connection, hash: &str) -> rusqlite::Result<()> {
    c.execute("delete from imports where hash=?1", [hash]).map(|_| ())
}
