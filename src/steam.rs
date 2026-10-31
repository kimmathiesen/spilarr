//! Metadata fra Steams åbne store-API (ingen nøgle). Blokerende – kaldes fra spawn_blocking.
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

const CDN: [&str; 2] = [
    "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/{a}/{f}",
    "https://cdn.cloudflare.steamstatic.com/steam/apps/{a}/{f}",
];
/// Ord i mappenavnene, som ikke står i Steams titler (alt fra første træf og frem skæres af).
const NOISE: &str = r"\b(incl(uding)?|bonus content|dlc|ps5|goty|gold|complete|black|definitive|deluxe|anniversary|directors cut|edition|remastered|remake|collection|the vetis wrath|overture|shadow of the erdtree|end of an era|knights of cross and rose)\b.*$";

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Meta {
    pub source: String,
    pub appid: i64,
    pub name: String,
    pub short: String,
    pub about: String,
    pub genres: Vec<String>,
    pub release: String,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    pub metacritic: Option<i64>,
    pub screens: Vec<String>,
    pub header: Option<String>,
    pub background: Option<String>,
    pub url: String,
    pub poster_wide: bool,
    pub req_min: Vec<String>,
    pub req_rec: Vec<String>,
}

pub fn http() -> &'static reqwest::blocking::Client {
    static C: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent("spilarr/0.1")
            .timeout(Duration::from_secs(20))
            .build()
            .expect("http-klient")
    })
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("regex")
}

pub fn norm(s: &str) -> String {
    let s: String = s.chars().filter(|c| !matches!(c, '®' | '™' | '©' | '\'' | '’' | '`' | ':')).collect();
    let s: String = s.nfkd().filter(|c| c.is_ascii()).collect::<String>().to_lowercase();
    re("[^a-z0-9]+").replace_all(&s, " ").trim().to_string()
}

/// Samme lighedsmål som Pythons difflib.SequenceMatcher.ratio() (Ratcliff/Obershelp).
pub fn ratio(a: &str, b: &str) -> f64 {
    fn matches(a: &[char], b: &[char]) -> usize {
        let (mut best, mut ai, mut bi) = (0, 0, 0);
        let mut prev = vec![0usize; b.len() + 1];
        for i in 0..a.len() {
            let mut cur = vec![0usize; b.len() + 1];
            for j in 0..b.len() {
                if a[i] == b[j] {
                    cur[j + 1] = prev[j] + 1;
                    if cur[j + 1] > best {
                        best = cur[j + 1];
                        ai = i + 1 - best;
                        bi = j + 1 - best;
                    }
                }
            }
            prev = cur;
        }
        if best == 0 {
            return 0;
        }
        best + matches(&a[..ai], &b[..bi]) + matches(&a[ai + best..], &b[bi + best..])
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    2.0 * matches(&a, &b) as f64 / (a.len() + b.len()) as f64
}

fn search(term: &str) -> Vec<(i64, String)> {
    let r = http()
        .get("https://store.steampowered.com/api/storesearch/")
        .query(&[("term", term), ("cc", "dk"), ("l", "danish")])
        .send()
        .and_then(|r| r.json::<Value>());
    let Ok(v) = r else { return vec![] };
    v["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|i| i["type"] == "app")
                .filter_map(|i| Some((i["id"].as_i64()?, i["name"].as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// (appid, navn, score) for bedste træf – prøver fuld titel og derefter uden udgave-ord.
pub fn best_match(title: &str) -> Option<(i64, String, f64)> {
    let full = norm(title);
    let mut terms = vec![title.to_string()];
    let short = re(NOISE).replace(&full, "").trim().to_string();
    if !short.is_empty() && short != full {
        terms.push(short);
    }
    let mut best: Option<(i64, String, f64)> = None;
    for t in &terms {
        let nt = norm(t);
        for (id, name) in search(t) {
            let n = norm(&name);
            let sc = ratio(&full, &n).max(ratio(&nt, &n));
            if best.as_ref().map_or(true, |b| sc > b.2) {
                best = Some((id, name, sc));
            }
        }
        if best.as_ref().is_some_and(|b| b.2 >= 0.9) {
            break;
        }
    }
    best.filter(|b| b.2 >= 0.72)
}

fn text(h: &str) -> String {
    let h = re(r"(?i)<br\s*/?>|</p>|</li>|</h\d>").replace_all(h, "\n");
    let h = re(r"(?i)<li>").replace_all(&h, "• ");
    let h = re(r"<[^>]+>").replace_all(&h, "");
    let h = html_escape::decode_html_entities(&h).to_string();
    re(r"\n{3,}").replace_all(&h, "\n\n").trim().to_string()
}

fn strs(v: &Value, key: &str) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x[key].as_str().or(x.as_str()).map(String::from)).collect())
        .unwrap_or_default()
}

/// Systemkrav fra Steams html som linjer, uden overskriften. Steam sender [] i stedet for et objekt, når der ingen er.
fn reqs(v: &Value) -> Vec<String> {
    text(v.as_str().unwrap_or_default())
        .lines()
        .map(|l| {
            let l = l.trim().trim_start_matches('•').trim();
            ["Minimum:", "Anbefalet:", "Recommended:"].iter().find_map(|h| l.strip_prefix(h)).unwrap_or(l).trim().to_string()
        })
        .filter(|l| !l.is_empty())
        .collect()
}

pub fn details(appid: i64) -> Option<Meta> {
    let v: Value = http()
        .get("https://store.steampowered.com/api/appdetails")
        .query(&[("appids", appid.to_string().as_str()), ("cc", "dk"), ("l", "danish")])
        .send()
        .ok()?
        .json()
        .ok()?;
    let d = &v[appid.to_string()];
    if d["success"] != true {
        return None;
    }
    let d = &d["data"];
    let s = |k: &str| d[k].as_str().unwrap_or_default().to_string();
    Some(Meta {
        source: "steam".into(),
        appid,
        name: s("name"),
        short: text(&s("short_description")),
        about: text(&s("about_the_game")),
        genres: strs(&d["genres"], "description"),
        release: d["release_date"]["date"].as_str().unwrap_or_default().to_string(),
        developers: strs(&d["developers"], ""),
        publishers: strs(&d["publishers"], ""),
        metacritic: d["metacritic"]["score"].as_i64(),
        screens: strs(&d["screenshots"], "path_full").into_iter().take(10).collect(),
        header: d["header_image"].as_str().map(String::from),
        background: d["background_raw"].as_str().or(d["background"].as_str()).map(String::from),
        url: format!("https://store.steampowered.com/app/{appid}"),
        poster_wide: false,
        req_min: reqs(&d["pc_requirements"]["minimum"]),
        req_rec: reqs(&d["pc_requirements"]["recommended"]),
    })
}

pub fn get_image(url: &str) -> Option<Vec<u8>> {
    let r = http().get(url).send().ok()?;
    let is_img = r.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("image"));
    if !r.status().is_success() || !is_img {
        return None;
    }
    let b = r.bytes().ok()?;
    (b.len() > 2000).then(|| b.to_vec())
}

/// Første billede, der findes på Steams CDN, fx library_600x900.jpg.
pub fn image(appid: i64, names: &[&str]) -> Option<Vec<u8>> {
    names.iter().find_map(|f| {
        CDN.iter().find_map(|c| get_image(&c.replace("{a}", &appid.to_string()).replace("{f}", f)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_som_difflib() {
        assert_eq!(ratio("abcd", "abcd"), 1.0);
        assert!((ratio("assassins creed", "assassin s creed") - 0.9677).abs() < 0.001);
    }

    #[test]
    fn systemkrav_som_linjer() {
        let h = "<strong>Minimum:</strong><br><ul class=\"bb_ul\"><li>Kræver 64-bit<br></li><li><strong>Hukommelse:</strong> 16 GB RAM<br></li></ul>";
        assert_eq!(reqs(&Value::from(h)), ["Kræver 64-bit", "Hukommelse: 16 GB RAM"]);
        assert_eq!(reqs(&Value::from("<strong>Minimum:</strong> 2 GHz")), ["2 GHz"]);
        assert!(reqs(&Value::from("<strong>Anbefalet:</strong><br><ul class=\"bb_ul\"></ul>")).is_empty());
        assert!(reqs(&serde_json::json!([])["minimum"]).is_empty());
    }

    #[test]
    fn norm_fjerner_tegn() {
        assert_eq!(norm("LEGO® Batman™: Legacy"), "lego batman legacy");
        assert_eq!(norm("Assassin's Creed"), "assassins creed");
    }
}
