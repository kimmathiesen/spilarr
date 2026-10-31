//! Kan min PC spille det? Sammenligner PC'ens specs med Steams systemkrav. Tallene er omtrentlige PassMark-værdier
//! (CPU Mark / G3D Mark), så de kun skal bruges til en grov vurdering. Komponenter uden for tabellerne giver "ukendt".
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Specs {
    pub cpu: String,
    pub gpu: String,
    pub ram: u32,
}

impl Specs {
    pub fn is_empty(&self) -> bool {
        self.cpu.is_empty() && self.gpu.is_empty() && self.ram == 0
    }
}

pub const GPUS: &[(&str, u32)] = &[
    // Nvidia
    ("GT 730", 850), ("GT 1030", 2500), ("GTX 460", 1700), ("GTX 550 Ti", 1400), ("GTX 560", 2000), ("GTX 560 Ti", 2300),
    ("GTX 570", 3000), ("GTX 580", 3500), ("GTX 650", 1800), ("GTX 650 Ti", 2400), ("GTX 660", 4000), ("GTX 660 Ti", 4500),
    ("GTX 670", 5400), ("GTX 680", 5800), ("GTX 750", 3400), ("GTX 750 Ti", 3900), ("GTX 760", 4800), ("GTX 770", 6000),
    ("GTX 780", 7700), ("GTX 780 Ti", 9000), ("GTX 950", 5300), ("GTX 960", 6000), ("GTX 970", 9600), ("GTX 980", 11000),
    ("GTX 980 Ti", 13500), ("GTX 1050", 4900), ("GTX 1050 Ti", 6300), ("GTX 1060", 10000), ("GTX 1070", 13500),
    ("GTX 1070 Ti", 14500), ("GTX 1080", 15500), ("GTX 1080 Ti", 18500), ("GTX 1650", 7800), ("GTX 1650 Super", 12400),
    ("GTX 1660", 11500), ("GTX 1660 Super", 12700), ("GTX 1660 Ti", 13000), ("RTX 2060", 14000), ("RTX 2060 Super", 16000),
    ("RTX 2070", 15800), ("RTX 2070 Super", 18000), ("RTX 2080", 19000), ("RTX 2080 Super", 19800), ("RTX 2080 Ti", 21800),
    ("RTX 3050", 13000), ("RTX 3060", 17000), ("RTX 3060 Ti", 20000), ("RTX 3070", 22000), ("RTX 3070 Ti", 23500),
    ("RTX 3080", 25000), ("RTX 3080 Ti", 26500), ("RTX 3090", 26500), ("RTX 3090 Ti", 29000), ("RTX 4060", 19800),
    ("RTX 4060 Ti", 22500), ("RTX 4070", 26500), ("RTX 4070 Super", 30000), ("RTX 4070 Ti", 31500), ("RTX 4070 Ti Super", 32000),
    ("RTX 4080", 34500), ("RTX 4080 Super", 35000), ("RTX 4090", 38000), ("RTX 5060", 23000), ("RTX 5060 Ti", 26000),
    ("RTX 5070", 29000), ("RTX 5070 Ti", 33000), ("RTX 5080", 36000), ("RTX 5090", 41000),
    // AMD
    ("HD 5770", 1100), ("HD 5850", 1500), ("HD 5870", 1800), ("HD 6850", 1400), ("HD 6870", 1600), ("HD 6950", 2300), ("HD 6970", 2600),
    ("HD 7770", 1500), ("HD 7790", 2100), ("HD 7850", 3000), ("HD 7870", 3800), ("HD 7950", 4700), ("HD 7970", 5300),
    ("R7 260X", 1800), ("R7 265", 2800), ("R7 370", 2300), ("R9 270", 3200), ("R9 270X", 3700), ("R9 280", 4500), ("R9 280X", 5300),
    ("R9 285", 4700), ("R9 290", 7000), ("R9 290X", 7500), ("R9 380", 5800), ("R9 380X", 6300), ("R9 390", 8000), ("R9 390X", 8500),
    ("R9 Fury", 9000), ("RX Vega 56", 12500), ("RX Vega 64", 14500),("RX 460", 3700), ("RX 470", 7500), ("RX 480", 8300), ("RX 550", 2500), ("RX 560", 3500), ("RX 570", 7100),
    ("RX 580", 8700), ("RX 590", 8900), ("RX 5500 XT", 9500), ("RX 5600 XT", 13500), ("RX 5700", 15000), ("RX 5700 XT", 16500),
    ("RX 6400", 6000), ("RX 6500 XT", 7000), ("RX 6600", 15500), ("RX 6600 XT", 17000), ("RX 6650 XT", 17200), ("RX 6700", 18500),
    ("RX 6700 XT", 19500), ("RX 6750 XT", 20500), ("RX 6800", 21000), ("RX 6800 XT", 25000), ("RX 6900 XT", 27000),
    ("RX 7600", 17000), ("RX 7600 XT", 20000), ("RX 7700 XT", 24000), ("RX 7800 XT", 26000), ("RX 7900 GRE", 27000),
    ("RX 7900 XT", 29000), ("RX 7900 XTX", 30000), ("RX 9060 XT", 24000), ("RX 9070", 29000), ("RX 9070 XT", 31000),
    // Intel
    ("Arc A380", 5000), ("Arc A580", 13000), ("Arc A750", 13500), ("Arc A770", 14000), ("Arc B570", 14500), ("Arc B580", 16000),
    ("HD 4000", 550), ("HD 4600", 750), ("HD 530", 1000), ("HD 630", 1100), ("UHD 620", 1000), ("UHD 630", 1200), ("UHD 770", 1500), ("Iris Xe", 2500),
];

pub const CPUS: &[(&str, u32)] = &[
    // Ældre Intel
    ("Core 2 Duo E6600", 1000), ("Core 2 Duo E8400", 1300), ("Core 2 Quad Q6600", 2200), ("Core 2 Quad Q9550", 3200),
    ("i3-2100", 1800), ("i3-3220", 2200), ("i3-4130", 2500), ("i3-4160", 2700), ("i3-6100", 3100), ("i3-7100", 3400), ("i3-8100", 5900),
    ("i3-9100", 6300), ("i3-10100", 8500), ("i3-12100", 14000), ("i3-13100", 15000),
    ("i5-750", 2300), ("i5-760", 2400), ("i5-2300", 3700), ("i5-2400", 4200), ("i5-2500", 4600), ("i5-3330", 4500), ("i5-3470", 5000), ("i5-3570", 5400),
    ("i5-4430", 4900), ("i5-4460", 5100), ("i5-4570", 5300), ("i5-4590", 5400), ("i5-4670", 5500), ("i5-4690", 5600),
    ("i5-6400", 5200), ("i5-6500", 5700), ("i5-6600", 6000), ("i5-7400", 5900), ("i5-7500", 6400), ("i5-7600", 6800),
    ("i5-8400", 9200), ("i5-8600", 10000), ("i5-9400", 9400), ("i5-9600", 10500), ("i5-10400", 12200), ("i5-10600", 14000),
    ("i5-11400", 16800), ("i5-11600", 18000), ("i5-12400", 19500), ("i5-12600", 27000), ("i5-13400", 25000), ("i5-13600", 37000), ("i5-14600", 39000),
    ("i7-860", 2900), ("i7-920", 2600), ("i7-2600", 5300), ("i7-3770", 6400), ("i7-4770", 7000), ("i7-4790", 7900),
    ("i7-6700", 8000), ("i7-7700", 9000), ("i7-8700", 12800), ("i7-9700", 14000), ("i7-10700", 18500), ("i7-11700", 21500),
    ("i7-12700", 31000), ("i7-13700", 43000), ("i7-14700", 53000),
    ("i9-9900", 17500), ("i9-10900", 22000), ("i9-11900", 25000), ("i9-12900", 41000), ("i9-13900", 59000), ("i9-14900", 61000),
    ("Core Ultra 5 245", 42000), ("Core Ultra 7 265", 58000), ("Core Ultra 9 285", 67000),
    // AMD
    ("Phenom II X4 940", 2500), ("Phenom II X4 945", 2600), ("Phenom II X4 955", 2700), ("Phenom II X4 965", 2900), ("Phenom II X6 1055T", 4200), ("Phenom II X6 1100T", 5000),
    ("FX-4300", 3000), ("FX-6100", 3900), ("FX-6300", 4600), ("FX-6350", 5000), ("FX-8120", 5700), ("FX-8150", 5800), ("FX-8300", 6500), ("FX-8320", 6700), ("FX-8350", 7600), ("FX-9590", 8700),
    ("Ryzen 3 1200", 6100), ("Ryzen 3 1300X", 6900), ("Ryzen 3 2200G", 6800), ("Ryzen 3 3100", 11200), ("Ryzen 3 3300X", 12700),
    ("Ryzen 5 1400", 7900), ("Ryzen 5 1500X", 9000), ("Ryzen 5 1600", 12000), ("Ryzen 5 2400G", 8300), ("Ryzen 5 2600", 13000),
    ("Ryzen 5 3400G", 9300), ("Ryzen 5 3500", 13500), ("Ryzen 5 3600", 17800), ("Ryzen 5 4500", 16000), ("Ryzen 5 5500", 19500), ("Ryzen 5 5600", 21500), ("Ryzen 5 7500F", 27000),
    ("Ryzen 5 7600", 28000), ("Ryzen 5 8400F", 22000), ("Ryzen 5 9600", 30000),
    ("Ryzen 7 1700", 14500), ("Ryzen 7 1800X", 16500), ("Ryzen 7 2700", 17000), ("Ryzen 7 3700X", 22500), ("Ryzen 7 3800X", 23500), ("Ryzen 7 5700X", 26500),
    ("Ryzen 7 5800X", 28000), ("Ryzen 7 5800X3D", 28000), ("Ryzen 7 7700", 34500), ("Ryzen 7 7800X3D", 34500), ("Ryzen 7 9700X", 37000), ("Ryzen 7 9800X3D", 40000),
    ("Ryzen 9 3900X", 32000), ("Ryzen 9 3950X", 38500), ("Ryzen 9 5900X", 39000), ("Ryzen 9 5950X", 45500), ("Ryzen 9 7900", 50000), ("Ryzen 9 7950X", 62000), ("Ryzen 9 9900X", 54000), ("Ryzen 9 9950X", 66000),
];

fn sep(c: char) -> bool {
    !c.is_alphanumeric()
}

/// Laveste score blandt de modeller, teksten nævner (krav som "GTX 970/RX 590" opfyldes af begge). Længste navne
/// prøves først, så "RTX 3060 Ti" ikke læses som "RTX 3060", og mellemrum og bindestreger ignoreres.
fn score_in(text: &str, table: &[(&str, u32)]) -> Option<u32> {
    let mut t: Vec<char> = text.to_lowercase().chars().collect();
    let mut models: Vec<(Vec<char>, u32)> = table
        .iter()
        .map(|(n, s)| (n.to_lowercase().chars().filter(|c| !sep(*c)).collect(), *s))
        .collect();
    models.sort_by_key(|m| std::cmp::Reverse(m.0.len()));
    let mut best = None;
    for (key, s) in &models {
        let mut i = 0;
        while i < t.len() {
            let mut j = i;
            let mut k = 0;
            while j < t.len() && k < key.len() {
                if t[j] == key[k] {
                    k += 1;
                    j += 1;
                } else if k > 0 && sep(t[j]) {
                    j += 1;
                } else {
                    break;
                }
            }
            let start_ok = i == 0 || !t[i - 1].is_alphanumeric();
            let end_ok = j == t.len() || !t[j].is_ascii_digit();
            if k == key.len() && start_ok && end_ok {
                best = Some(best.map_or(*s, |b: u32| b.min(*s)));
                t[i..j].fill(' ');
                i = j;
            } else {
                i += 1;
            }
        }
    }
    best
}

/// RAM i GB fra fx "16 GB RAM" eller "512 MB RAM".
fn ram_in(text: &str) -> Option<f64> {
    let t = text.to_lowercase();
    let cs: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        if cs[i].is_ascii_digit() {
            let st = i;
            while i < cs.len() && (cs[i].is_ascii_digit() || matches!(cs[i], '.' | ',')) {
                i += 1;
            }
            let n: f64 = cs[st..i].iter().collect::<String>().replace(',', ".").trim_end_matches('.').parse().ok()?;
            let unit: String = cs[i..].iter().skip_while(|c| c.is_whitespace()).take(2).collect();
            match unit.as_str() {
                "gb" => return Some(n),
                "mb" => return Some(n / 1024.0),
                _ => {}
            }
        } else {
            i += 1;
        }
    }
    None
}

fn line<'a>(lines: &'a [String], labels: &[&str]) -> Option<&'a str> {
    lines.iter().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        let k = k.trim().to_lowercase();
        labels.iter().any(|x| k.starts_with(x)).then_some(v)
    })
}

#[derive(Default)]
struct Tier {
    /// Komponenter, der er for svage
    under: Vec<&'static str>,
    /// Komponenter, der ikke kunne sammenlignes
    uklar: Vec<&'static str>,
}

fn tier(lines: &[String], pc: &Specs) -> Tier {
    let mut r = Tier::default();
    let cmp = |have: Option<f64>, need: Option<f64>, name: &'static str, r: &mut Tier| match (have, need) {
        (Some(h), Some(n)) if h < n => r.under.push(name),
        (Some(_), Some(_)) => {}
        _ => r.uklar.push(name),
    };
    let known = |table: &[(&str, u32)], name: &str| table.iter().find(|m| m.0 == name).map(|m| f64::from(m.1));
    let need = |labels: &[&str], table: &[(&str, u32)]| line(lines, labels).and_then(|l| score_in(l, table)).map(f64::from);
    cmp(known(CPUS, &pc.cpu), need(&["processor", "cpu"], CPUS), "processor", &mut r);
    cmp(known(GPUS, &pc.gpu), need(&["grafik", "graphics", "video"], GPUS), "grafikkort", &mut r);
    let ram = (pc.ram > 0).then_some(f64::from(pc.ram));
    cmp(ram, line(lines, &["hukommelse", "memory", "ram"]).and_then(ram_in), "RAM", &mut r);
    r
}

/// ("ja" | "min" | "nej" | "ukendt", forklaring). "ja" = opfylder anbefalet, "min" = opfylder minimum.
pub fn vurder(pc: &Specs, req_min: &[String], req_rec: &[String]) -> (&'static str, String) {
    let (a, b) = (tier(req_min, pc), tier(req_rec, pc));
    if !a.under.is_empty() {
        return ("nej", format!("Under minimum: {}", a.under.join(", ")));
    }
    if b.under.is_empty() && b.uklar.is_empty() {
        return ("ja", "Opfylder de anbefalede krav".into());
    }
    if a.uklar.is_empty() {
        let why = if b.under.is_empty() { "kan ikke sammenligne anbefalet".to_string() } else { format!("under anbefalet: {}", b.under.join(", ")) };
        return ("min", format!("Opfylder minimum, {why}"));
    }
    ("ukendt", format!("Kan ikke sammenligne: {}", a.uklar.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pc() -> Specs {
        Specs { cpu: "Ryzen 5 3600".into(), gpu: "RTX 3060".into(), ram: 16 }
    }

    fn v(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn finder_modeller() {
        assert_eq!(score_in("4 GB, GeForce GTX 970/Radeon RX 590", GPUS), Some(8900));
        assert_eq!(score_in("NVIDIA GeForce RTX 3060 Ti", GPUS), Some(20000));
        assert_eq!(score_in("GTX 1060 3GB", GPUS), Some(10000));
        assert_eq!(score_in("GTX1060", GPUS), Some(10000));
        assert_eq!(score_in("RX 5700", GPUS), Some(15000));
        assert_eq!(score_in("Intel Core i5-4690K (3.5 GHz)/AMD FX-8300 (3.3 GHz)", CPUS), Some(5600));
        assert_eq!(score_in("AMD Ryzen 5 3600X", CPUS), Some(17800));
        assert_eq!(score_in("Intel Core2 Duo E8400", CPUS), Some(1300));
        assert_eq!(score_in("Dual Core 2.4 GHz", CPUS), None);
    }

    #[test]
    fn ram() {
        assert_eq!(ram_in(" 16 GB RAM"), Some(16.0));
        assert_eq!(ram_in("512 MB RAM"), Some(0.5));
        assert_eq!(ram_in("1,5 GB"), Some(1.5));
        assert_eq!(ram_in("nok"), None);
    }

    #[test]
    fn vurderinger() {
        let min = v(&["Processor: i5-4690K", "Hukommelse: 8 GB RAM", "Grafik: GTX 970"]);
        let rec = v(&["Processor: i7-8700K", "Hukommelse: 16 GB RAM", "Grafik: RTX 3070"]);
        assert_eq!(vurder(&pc(), &min, &rec).0, "min");
        assert!(vurder(&pc(), &min, &rec).1.contains("grafikkort"));
        assert_eq!(vurder(&pc(), &min, &min).0, "ja");
        assert_eq!(vurder(&pc(), &rec, &rec).0, "nej");
        assert_eq!(vurder(&pc(), &[], &[]).0, "ukendt");
        assert_eq!(vurder(&pc(), &v(&["Processor: i5-4690", "Hukommelse: 8 GB", "Grafik: Ukendt kort"]), &[]).0, "ukendt");
        // Ét klart for svagt krav vejer tungere end et, der ikke kan læses
        assert_eq!(vurder(&pc(), &v(&["Hukommelse: 32 GB", "Grafik: Ukendt kort"]), &[]).0, "nej");
    }

    #[test]
    fn komponenter_er_entydige() {
        for t in [GPUS, CPUS] {
            for (i, a) in t.iter().enumerate() {
                assert!(!t[i + 1..].iter().any(|b| a.0 == b.0), "{} står to gange", a.0);
            }
        }
    }
}
