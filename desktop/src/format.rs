//! Formatvertrag Server → Desktop, siehe shared/FORMAT.md

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ausloeser {
    Sofort,
    Leertaste,
    Enter,
    Tab,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Baustein {
    pub kuerzel: String,
    /// Reiner Text. Beim formatierten Baustein (F10) aus dem HTML abgeleitet, für Programme ohne HTML
    pub inhalt: String,
    /// F10: formatierter Baustein, bereinigtes HTML (nur im Tarif mit Formatierung)
    #[serde(default)]
    pub html: Option<String>,
    /// ab Version 2
    #[serde(default)]
    pub bezeichnung: Option<String>,
    /// ab Version 2, bei Version 1 immer true
    #[serde(default = "wahr")]
    pub aktiv: bool,
    pub ausloeser: Ausloeser,
    pub wortgrenze: bool,
    pub gross_klein_beachten: bool,
    pub schreibweise_uebernehmen: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Stand {
    pub version: u32,
    /// Sprache des Accounts (F15), fehlt bei älteren Servern
    #[serde(default)]
    pub sprache: Option<String>,
    /// Neueste Desktop-Version laut Server (F21), fehlt, wenn unbekannt
    #[serde(default, rename = "desktopVersion")]
    pub desktop_version: Option<String>,
    pub bausteine: Vec<Baustein>,
    pub ausschluesse: Vec<String>,
}

fn wahr() -> bool {
    true
}

/// true, wenn `verfuegbar` (z. B. "v0.2.0" oder "0.2.0") neuer ist als `laufend` (F21 AC-8)
pub fn neuer(verfuegbar: &str, laufend: &str) -> bool {
    let teile = |v: &str| -> Option<Vec<u64>> { v.trim().trim_start_matches('v').split('.').map(|x| x.parse().ok()).collect() };
    match (teile(verfuegbar), teile(laufend)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

pub fn parse(json: &str) -> Result<Stand, String> {
    let stand: Stand = serde_json::from_str(json).map_err(|e| format!("Stand nicht lesbar: {e}"))?;
    if !(1..=2).contains(&stand.version) {
        return Err(format!("Version {} wird nicht unterstützt, bitte tippkraft aktualisieren", stand.version));
    }
    Ok(stand)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taste {
    Tab,
    Enter,
    Space,
    Backspace,
    Delete,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

impl Taste {
    fn von_name(name: &str) -> Option<Taste> {
        use Taste::*;
        Some(match name {
            "Tab" => Tab,
            "Enter" => Enter,
            "Space" => Space,
            "Backspace" => Backspace,
            "Delete" => Delete,
            "Escape" => Escape,
            "Left" => Left,
            "Right" => Right,
            "Up" => Up,
            "Down" => Down,
            "Home" => Home,
            "End" => End,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aktion {
    Text(String),
    Taste(Taste, u8),
    /// F10: über die Zwischenablage einfügen, als HTML und als reiner Text
    Html { html: String, text: String },
}

/// Zerlegt einen Inhalt in Text und Tasten-Tokens. "[[[[" steht für ein wörtliches "[[".
pub fn aktionen(inhalt: &str) -> Vec<Aktion> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = inhalt;
    while let Some(i) = rest.find("[[") {
        text.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix("[[[[") {
            text.push_str("[[");
            rest = r;
            continue;
        }
        let token = rest[2..].find("]]").and_then(|end| {
            let inner = &rest[2..2 + end];
            let (name, n) = match inner.split_once(':') {
                Some((name, n)) => (name, n.parse::<u8>().ok().filter(|n| (1..=20).contains(n))?),
                None => (inner, 1),
            };
            Some((Taste::von_name(name)?, n, 2 + end + 2))
        });
        match token {
            Some((taste, n, len)) => {
                if !text.is_empty() {
                    out.push(Aktion::Text(std::mem::take(&mut text)));
                }
                out.push(Aktion::Taste(taste, n));
                rest = &rest[len..];
            }
            None => {
                text.push_str("[[");
                rest = &rest[2..];
            }
        }
    }
    text.push_str(rest);
    if !text.is_empty() {
        out.push(Aktion::Text(text));
    }
    out
}

/// Abfrage im Popup: Textfeld (`optionen: None`) oder Auswahl
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Abfrage {
    pub name: String,
    pub optionen: Option<Vec<String>>,
    /// F42: {{?Name*}}
    #[serde(default)]
    pub pflicht: bool,
    /// F42: "number" oder "email" bei {{?Name:number}}
    #[serde(default)]
    pub pruefung: Option<String>,
}

/// F42: "Name*:email" → Name, Pflicht, Rest hinter dem Doppelpunkt
fn feld(rest: &str) -> (&str, bool, Option<&str>) {
    let (roh, nach) = match rest.split_once(':') {
        Some((n, o)) => (n, Some(o)),
        None => (rest, None),
    };
    match roh.strip_suffix('*') {
        Some(n) => (n, true, nach),
        None => (roh, false, nach),
    }
}

/// F42: Pflicht und Prüfung. Leere optionale Felder sind immer gültig.
pub fn gueltig(a: &Abfrage, wert: &str) -> bool {
    let w = wert.trim();
    if w.is_empty() {
        return !a.pflicht;
    }
    match a.pruefung.as_deref() {
        Some("number") => {
            let z = w.strip_prefix('-').unwrap_or(w);
            let (ganz, rest) = z.split_once([',', '.']).unwrap_or((z, "0"));
            [ganz, rest].iter().all(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()))
        }
        Some("email") => w.split_once('@').is_some_and(|(l, d)| !l.is_empty() && !w.contains(char::is_whitespace) && !d.contains('@') && d.split('.').count() > 1 && d.split('.').all(|t| !t.is_empty())),
        _ => true,
    }
}

enum Stueck<'a> {
    Text(&'a str),
    /// Inhalt zwischen {{ und }}
    Klammer(&'a str),
}

/// Zerlegt nach {{…}}. "{{{{" ergibt Text "{{".
fn stuecke(inhalt: &str) -> Vec<Stueck<'_>> {
    let mut out = vec![];
    let mut rest = inhalt;
    while let Some(i) = rest.find("{{") {
        out.push(Stueck::Text(&rest[..i]));
        rest = &rest[i..];
        if rest.starts_with("{{{{") {
            out.push(Stueck::Text("{{"));
            rest = &rest[4..];
        } else if let Some(ende) = rest[2..].find("}}") {
            out.push(Stueck::Klammer(&rest[2..2 + ende]));
            rest = &rest[2 + ende + 2..];
        } else {
            break;
        }
    }
    out.push(Stueck::Text(rest));
    out
}

/// Abfragen in Reihenfolge des ersten Vorkommens, jeder Name einmal
pub fn abfragen(inhalt: &str) -> Vec<Abfrage> {
    let mut out: Vec<Abfrage> = vec![];
    // F55: offene Blöcke, bei {{#choice}} der Index der Abfrage, die die Möglichkeiten sammelt
    let mut bloecke: Vec<Option<usize>> = vec![];
    for s in stuecke(inhalt) {
        let Stueck::Klammer(k) = s else { continue };
        if let Some(name) = k.strip_prefix("#choice ") {
            let name = name.trim();
            let neu = !out.iter().any(|a| a.name == name);
            if neu {
                out.push(Abfrage { name: name.into(), optionen: Some(vec![]), ..Default::default() });
            }
            bloecke.push(neu.then_some(out.len() - 1));
        } else if let Some(o) = k.strip_prefix("#option ") {
            if let Some(Some(i)) = bloecke.last()
                && let Some(liste) = out[*i].optionen.as_mut()
            {
                liste.push(o.to_string());
            }
        } else if k.starts_with("#if ") || k.starts_with("#set ") {
            bloecke.push(None);
        } else if k == "#end" {
            bloecke.pop();
        } else if let Some(rest) = k.strip_prefix('?') {
            let (name, pflicht, nach) = feld(rest);
            let pruefung = nach.filter(|o| matches!(*o, "number" | "email")).map(String::from);
            let optionen = nach.filter(|_| pruefung.is_none()).map(|o| o.split('|').map(String::from).collect());
            if !out.iter().any(|a| a.name == name) {
                out.push(Abfrage { name: name.into(), optionen, pflicht, pruefung });
            }
        }
    }
    out
}

/// Werte, die beim Auslösen automatisch eingesetzt werden
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Automatisch {
    pub datum: String,
    /// F40: Zeitpunkt für {{=date:FORMAT}} als "%Y-%m-%dT%H:%M", Namen auf Deutsch bei `de`
    #[serde(default)]
    pub jetzt: String,
    #[serde(default)]
    pub de: bool,
    /// nur gelesen, wenn der Inhalt {{=clipboard}} enthält
    pub zwischenablage: Option<String>,
    /// F42: Variablen aus früheren Bausteinen, nur im Speicher
    #[serde(default)]
    pub variablen: std::collections::HashMap<String, String>,
    /// F42: Startwert für {{=random:…}}, damit HTML und Text dieselbe Option bekommen
    #[serde(default)]
    pub zufall: u64,
}

const MONATE_DE: [&str; 12] = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
const MONATE_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const TAGE_DE: [&str; 7] = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"];
const TAGE_EN: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// F40: Datum nach Muster wie dd.MM.yyyy, MMMM, EEEE, HH:mm. Unbekannte Zeichen bleiben wörtlich.
pub fn datum_mit_format(t: chrono::NaiveDateTime, fmt: &str, de: bool) -> String {
    use chrono::{Datelike, Timelike};
    let monat = (if de { MONATE_DE } else { MONATE_EN })[t.month0() as usize];
    let tag = (if de { TAGE_DE } else { TAGE_EN })[t.weekday().num_days_from_monday() as usize];
    let kurz = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    // längste Muster zuerst
    let muster: [(&str, String); 16] = [
        ("yyyy", format!("{:04}", t.year())),
        // F42: Tag des Jahres, ISO-Kalenderwoche
        ("DDD", format!("{:03}", t.ordinal())),
        ("ww", format!("{:02}", t.iso_week().week())),
        ("MMMM", monat.to_string()),
        ("EEEE", tag.to_string()),
        ("MMM", kurz(monat, 3)),
        ("EEE", kurz(tag, if de { 2 } else { 3 })),
        ("yy", format!("{:02}", t.year() % 100)),
        ("MM", format!("{:02}", t.month())),
        ("dd", format!("{:02}", t.day())),
        ("HH", format!("{:02}", t.hour())),
        ("mm", format!("{:02}", t.minute())),
        ("M", t.month().to_string()),
        ("d", t.day().to_string()),
        ("D", t.ordinal().to_string()),
        ("w", t.iso_week().week().to_string()),
    ];
    let mut out = String::new();
    let mut rest = fmt;
    'aussen: while !rest.is_empty() {
        // 'text' bleibt wörtlich, '' ist ein einzelnes Anführungszeichen
        if let Some(r) = rest.strip_prefix('\'') {
            let ende = r.find('\'').unwrap_or(r.len());
            out.push_str(if ende == 0 { "'" } else { &r[..ende] });
            rest = r.get(ende + 1..).unwrap_or_default();
            continue;
        }
        for (m, w) in &muster {
            if let Some(r) = rest.strip_prefix(m) {
                out.push_str(w);
                rest = r;
                continue 'aussen;
            }
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Markiert {{=cursor}} bis zur Auswertung in `engine::nach_popup` (Zeichen aus dem Private-Use-Bereich)
pub const CURSOR: char = '\u{E000}';

/// Ergebnis von `auswerten`
pub struct Auswertung {
    pub text: String,
    /// Variablen nach dem Baustein: frühere, Werte aus dem Popup, gesetzte (F42)
    pub variablen: std::collections::HashMap<String, String>,
    /// Abfragen in gewählten Zweigen, in Reihenfolge (F42 AC-7)
    pub sichtbar: Vec<String>,
}

/// Setzt Werte, Variablen, Datum, Zwischenablage, Zufall und Cursor-Markierung ein, wertet Bedingungen aus
/// und macht aus "{{{{" ein "{{". Werte werden maskiert, damit "[[" darin Text bleibt.
pub fn einsetzen(inhalt: &str, werte: &std::collections::HashMap<String, String>, auto: &Automatisch) -> String {
    auswerten(inhalt, werte, auto).text
}

pub fn auswerten(inhalt: &str, werte: &std::collections::HashMap<String, String>, auto: &Automatisch) -> Auswertung {
    let mut vars = auto.variablen.clone();
    vars.extend(werte.iter().map(|(k, v)| (k.clone(), v.clone())));
    let (mut text, mut sichtbar) = (String::new(), vec![]);
    let mut stapel: Vec<Rahmen> = vec![];
    let mut aktiv = true;
    let mut zufall = auto.zufall;
    for s in stuecke(inhalt) {
        match s {
            Stueck::Klammer(k) if k.starts_with("#if ") => {
                let b = bedingung(&k[4..], &vars);
                stapel.push(Rahmen::Wenn { davor: aktiv, erfuellt: b });
                aktiv &= b;
            }
            Stueck::Klammer("#else") => {
                if let Some(&Rahmen::Wenn { davor, erfuellt }) = stapel.last() {
                    aktiv = davor && !erfuellt;
                }
            }
            // F55: Auswahl mit eigenem Inhalt je Möglichkeit. Gewählt ist, was im Popup (oder als Variable) steht.
            Stueck::Klammer(k) if k.starts_with("#choice ") => {
                let name = k[8..].trim();
                if aktiv && !sichtbar.iter().any(|s| s == name) {
                    sichtbar.push(name.to_string());
                }
                stapel.push(Rahmen::Auswahl { davor: aktiv, gewaehlt: vars.get(name).cloned().unwrap_or_default() });
                aktiv = false;
            }
            Stueck::Klammer(k) if k.starts_with("#option ") => {
                if let Some(Rahmen::Auswahl { davor, gewaehlt }) = stapel.last() {
                    aktiv = *davor && k[8..] == **gewaehlt;
                }
            }
            // F55: Inhalt auswerten und in die Variable schreiben statt einfügen
            Stueck::Klammer(k) if k.starts_with("#set ") => stapel.push(Rahmen::Setzen { davor: aktiv, name: k[5..].trim().to_string(), start: text.len() }),
            Stueck::Klammer("#end") => match stapel.pop() {
                Some(Rahmen::Setzen { davor, name, start }) => {
                    if davor {
                        // Variablen halten Text: Maskierung zurück, Cursor-Markierung weg
                        vars.insert(name, text[start..].replace("[[[[", "[[").replace(CURSOR, ""));
                    }
                    text.truncate(start);
                    aktiv = davor;
                }
                Some(Rahmen::Wenn { davor, .. } | Rahmen::Auswahl { davor, .. }) => aktiv = davor,
                None => {}
            },
            _ if !aktiv => {}
            Stueck::Text(t) => text.push_str(t),
            Stueck::Klammer(k) => text.push_str(&klammer(k, &mut vars, &mut sichtbar, auto, &mut zufall)),
        }
    }
    Auswertung { text, variablen: vars, sichtbar }
}

/// Offener Block beim Auswerten
enum Rahmen {
    Wenn { davor: bool, erfuellt: bool },
    Auswahl { davor: bool, gewaehlt: String },
    Setzen { davor: bool, name: String, start: usize },
}

fn bedingung(b: &str, vars: &std::collections::HashMap<String, String>) -> bool {
    // F57: {{#if Name}}, {{#if !Name}}, {{#if Name OP Wert}} mit OP = != >= <= !~ = > < ~, Wert fest oder $Variable
    let wert = |n: &str| vars.get(n.trim().trim_start_matches('$').trim()).map(|w| w.trim().to_string()).unwrap_or_default();
    let Some(i) = b.find(['=', '!', '<', '>', '~']) else { return !wert(b).is_empty() };
    if i == 0 && b.starts_with('!') && !b[1..].contains(['=', '!', '<', '>', '~']) {
        return wert(&b[1..]).is_empty();
    }
    let (links, rest) = b.split_at(i);
    let op = ["!=", ">=", "<=", "!~", "=", ">", "<", "~"].into_iter().find(|o| rest.starts_with(o)).unwrap_or("=");
    let rechts = rest[op.len()..].trim();
    let (l, r) = (wert(links), if rechts.starts_with('$') { wert(rechts) } else { rechts.to_string() });
    let (lk, rk) = (l.to_lowercase(), r.to_lowercase());
    let zahlen = zahl(&l).zip(zahl(&r));
    match op {
        "!=" => zahlen.map_or(lk != rk, |(a, b)| a != b),
        "=" => zahlen.map_or(lk == rk, |(a, b)| a == b),
        "~" => lk.contains(&rk),
        "!~" => !lk.contains(&rk),
        // größer/kleiner nur für Zahlen, sonst falsch
        o => zahlen.is_some_and(|(a, b)| match o {
            ">" => a > b,
            "<" => a < b,
            ">=" => a >= b,
            _ => a <= b,
        }),
    }
}

/// "12,50", "1.234,5", "1234.5", " 7 " als Zahl
fn zahl(s: &str) -> Option<f64> {
    let s = s.trim().replace(' ', "");
    let s = if s.contains(',') { s.replace('.', "").replace(',', ".") } else { s };
    s.parse::<f64>().ok().filter(|z| z.is_finite())
}

/// splitmix64: genügt für die Wahl einer Formulierung
fn zufallszahl(z: &mut u64) -> u64 {
    *z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut x = *z;
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn klammer(k: &str, vars: &mut std::collections::HashMap<String, String>, sichtbar: &mut Vec<String>, auto: &Automatisch, zufall: &mut u64) -> String {
    let maskiert = |w: &str| w.replace("[[", "[[[[");
    if let Some(d) = k.strip_prefix("=date").or_else(|| k.strip_prefix("=datum"))
        && let Some(t) = datum(d, auto)
    {
        return t;
    }
    match k {
        "=clipboard" => return auto.zwischenablage.as_deref().map(maskiert).unwrap_or_default(),
        "=cursor" => return CURSOR.to_string(),
        _ => {}
    }
    if let Some(o) = k.strip_prefix("=random:") {
        let o: Vec<&str> = o.split('|').collect();
        return o[(zufallszahl(zufall) % o.len() as u64) as usize].to_string();
    }
    if let Some(v) = k.strip_prefix('$') {
        return match v.split_once('=') {
            Some((n, w)) => {
                vars.insert(n.to_string(), w.to_string());
                String::new()
            }
            None => vars.get(v).map(|w| maskiert(w)).unwrap_or_default(),
        };
    }
    if let Some(rest) = k.strip_prefix('?') {
        let (name, ..) = feld(rest);
        if !sichtbar.iter().any(|s| s == name) {
            sichtbar.push(name.to_string());
        }
        return vars.get(name).map(|w| maskiert(w)).unwrap_or_default();
    }
    format!("{{{{{k}}}}}")
}

/// "", "+14", "-1m:dd.MM.yyyy", ":FORMAT" → Datum. None, wenn es kein Datums-Platzhalter ist.
fn datum(d: &str, auto: &Automatisch) -> Option<String> {
    let (versatz, fmt) = d.split_once(':').unwrap_or((d, ""));
    if versatz.is_empty() && fmt.is_empty() {
        return Some(auto.datum.clone());
    }
    let Ok(jetzt) = chrono::NaiveDateTime::parse_from_str(&auto.jetzt, "%Y-%m-%dT%H:%M") else { return Some(auto.datum.clone()) };
    let t = if versatz.is_empty() {
        jetzt
    } else {
        let (zahl, einheit) = match versatz.char_indices().last() {
            Some((i, c)) if c.is_ascii_alphabetic() => (&versatz[..i], c),
            _ => (versatz, 'd'),
        };
        let n: i32 = zahl.parse().ok().filter(|_| zahl.starts_with(['+', '-']))?;
        let monate = |m: i32| if m >= 0 { jetzt.checked_add_months(chrono::Months::new(m as u32)) } else { jetzt.checked_sub_months(chrono::Months::new(m.unsigned_abs())) };
        match einheit {
            'd' => jetzt.checked_add_signed(chrono::Duration::days(n as i64)),
            'w' => jetzt.checked_add_signed(chrono::Duration::weeks(n as i64)),
            'm' => monate(n),
            'y' => monate(n * 12),
            _ => None,
        }?
    };
    Some(if fmt.is_empty() { t.format(if auto.de { "%d.%m.%Y" } else { "%Y-%m-%d" }).to_string() } else { datum_mit_format(t, fmt, auto.de) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zeit(s: &str) -> chrono::NaiveDateTime {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn datum_mit_format_f40_ac1() {
        let t = zeit("2026-03-06 09:05");
        assert_eq!(datum_mit_format(t, "d. MMMM yyyy", true), "6. März 2026");
        assert_eq!(datum_mit_format(t, "dd.MM.yy", true), "06.03.26");
        assert_eq!(datum_mit_format(t, "EEEE, d. MMM", true), "Freitag, 6. Mär");
        assert_eq!(datum_mit_format(t, "EEE d MMM yyyy, HH:mm", false), "Fri 6 Mar 2026, 09:05");
        assert_eq!(datum_mit_format(t, "MMMM d, yyyy", false), "March 6, 2026");
        assert_eq!(datum_mit_format(t, "yyyy-MM-dd (KW)", false), "2026-03-06 (KW)");
        assert_eq!(datum_mit_format(t, "EEEE, 'den' d. MMMM", true), "Freitag, den 6. März");
        assert_eq!(datum_mit_format(t, "d. MMMM ''yy", true), "6. März '26");
    }

    #[test]
    fn date_platzhalter_mit_und_ohne_format_f40_ac2() {
        let auto = Automatisch { datum: "06.03.2026".into(), jetzt: "2026-03-06T09:05".into(), de: true, zwischenablage: None, ..Default::default() };
        let w = std::collections::HashMap::new();
        assert_eq!(einsetzen("{{=date}} / {{=date:d. MMMM}} / {{=datum:yyyy}} / {{=date:}}", &w, &auto), "06.03.2026 / 6. März / 2026 / 06.03.2026");
    }

    #[test]
    fn liest_den_vertrag_ac15() {
        let json = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../shared/snippets.example.json")).unwrap();
        let stand = parse(&json).unwrap();
        assert_eq!(stand.version, 2);
        assert_eq!(stand.bausteine.len(), 5);
        let alt = &stand.bausteine[2];
        assert_eq!((alt.kuerzel.as_str(), alt.aktiv, alt.bezeichnung.as_deref()), ("alt", false, Some("Nur zum Nachschlagen")));
        assert_eq!(stand.bausteine[1].bezeichnung.as_deref(), Some("Adresse"));
        assert_eq!(stand.ausschluesse, vec!["1password", "org.keepassxc.KeePassXC"]);
        let sig = &stand.bausteine[0];
        assert_eq!(sig.kuerzel, ";sig");
        assert_eq!(sig.ausloeser, Ausloeser::Sofort);
        assert!(!sig.wortgrenze && !sig.gross_klein_beachten && !sig.schreibweise_uebernehmen);
        assert_eq!(stand.bausteine[1].ausloeser, Ausloeser::Tab);
        assert!(stand.bausteine[1].gross_klein_beachten);
        assert_eq!(stand.bausteine[4].ausloeser, Ausloeser::Leertaste);
        let brief = &stand.bausteine[3];
        assert_eq!(brief.kuerzel, "brief");
        assert_eq!(
            abfragen(&brief.inhalt),
            vec![
                Abfrage { name: "Anrede".into(), optionen: Some(vec!["Herr".into(), "Frau".into()]), ..Default::default() },
                Abfrage { name: "Name".into(), optionen: None, ..Default::default() }
            ]
        );
    }

    #[test]
    fn liest_version_1_mit_standardwerten_f14_ac2() {
        let s = parse(r#"{"version":1,"bausteine":[{"kuerzel":"a","inhalt":"b","ausloeser":"tab","wortgrenze":true,"grossKleinBeachten":false,"schreibweiseUebernehmen":true}],"ausschluesse":[]}"#).unwrap();
        assert!(s.bausteine[0].aktiv);
        assert_eq!(s.bausteine[0].bezeichnung, None);
    }

    #[test]
    fn vergleicht_versionen_f21_ac8() {
        assert!(neuer("v0.2.0", "0.1.0"));
        assert!(neuer("0.10.0", "0.9.3"));
        assert!(!neuer("0.1.0", "0.1.0"));
        assert!(!neuer("v0.1.0", "0.2.0"));
        assert!(!neuer("kaputt", "0.1.0"));
        let s = parse(r#"{"version":2,"desktopVersion":"v0.3.1","bausteine":[],"ausschluesse":[]}"#).unwrap();
        assert_eq!(s.desktop_version.as_deref(), Some("v0.3.1"));
    }

    #[test]
    fn lehnt_unbekannte_version_ab_ac15() {
        let err = parse(r#"{"version":3,"bausteine":[],"ausschluesse":[]}"#).unwrap_err();
        assert!(err.contains("Version 3"), "{err}");
        assert!(parse("kein json").is_err());
    }

    #[test]
    fn zerlegt_tokens_ac14() {
        assert_eq!(
            aktionen("a[[Tab:2]]b[[[[c"),
            vec![Aktion::Text("a".into()), Aktion::Taste(Taste::Tab, 2), Aktion::Text("b[[c".into())]
        );
        assert_eq!(aktionen("[[Enter]]"), vec![Aktion::Taste(Taste::Enter, 1)]);
        assert_eq!(aktionen("x [[[[kein Token]]"), vec![Aktion::Text("x [[kein Token]]".into())]);
        assert_eq!(aktionen(""), vec![]);
    }

    #[test]
    fn unbekannte_tokens_bleiben_text() {
        // Der Server lässt sie nicht durch. Falls doch, lieber wörtlich tippen als verschlucken.
        assert_eq!(aktionen("[[Foo]]"), vec![Aktion::Text("[[Foo]]".into())]);
    }

    #[test]
    fn abfragen_einmal_pro_name_in_reihenfolge_ac6() {
        let a = abfragen("{{?Name}} {{?Gruß:Viele Grüße|}} {{=datum}} {{{{?nicht}} {{?Name}}");
        assert_eq!(a, vec![Abfrage { name: "Name".into(), optionen: None, ..Default::default() }, Abfrage { name: "Gruß".into(), optionen: Some(vec!["Viele Grüße".into(), "".into()]), ..Default::default() }]);
        assert!(abfragen("nur {{=datum}} und [[Tab]]").is_empty());
    }

    #[test]
    fn einsetzen_ac5_ac7_ac8() {
        let werte = std::collections::HashMap::from([("Name".to_string(), "Muster [[Tab]] {{=datum}}".to_string()), ("Anrede".to_string(), "Frau".to_string())]);
        assert_eq!(
            einsetzen("{{?Anrede:Herr|Frau}} {{?Name}}, {{=datum}} {{{{?x}} {{?Name}} {{unbekannt}}", &werte, &Automatisch { datum: "04.11.2026".into(), zwischenablage: None, ..Default::default() }),
            // Werte werden maskiert: [[ → [[[[, damit sie Text bleiben. {{=datum}} im Wert bleibt Text.
            "Frau Muster [[[[Tab]] {{=datum}}, 04.11.2026 {{?x}} Muster [[[[Tab]] {{=datum}} {{unbekannt}}"
        );
        assert_eq!(aktionen(&einsetzen("{{?Name}}", &werte, &Automatisch::default())), vec![Aktion::Text("Muster [[Tab]] {{=datum}}".into())]);
    }

    // ---------- F42 ----------

    fn auto_f42() -> Automatisch {
        Automatisch { datum: "31.01.2026".into(), jetzt: "2026-01-31T09:05".into(), de: true, ..Default::default() }
    }
    fn w(paare: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        paare.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn variablen_setzen_und_einsetzen_f42_ac1_ac2_ac3() {
        let mut auto = auto_f42();
        auto.variablen = w(&[("Kunde", "Meier")]);
        let a = auswerten("{{$Kunde}} / {{$Ort=Köln}}Ort: {{$Ort}} / [{{$X}}]", &w(&[]), &auto);
        assert_eq!(a.text, "Meier / Ort: Köln / []");
        assert_eq!(a.variablen["Ort"], "Köln");
        assert_eq!(a.variablen["Kunde"], "Meier");
        // Abfrage wird zur Variable, spätere Zuweisung überschreibt
        let a = auswerten("{{?Name*}} {{$Name}} {{$Name=B}}{{$Name}}", &w(&[("Name", "A")]), &auto);
        assert_eq!(a.text, "A A B");
        assert_eq!(a.variablen["Name"], "B");
        // Werte bleiben Text
        assert_eq!(einsetzen("{{$T}}", &w(&[("T", "[[Tab]]")]), &auto), "[[[[Tab]]");
    }

    #[test]
    fn vergleiche_f57() {
        let auto = auto_f42();
        let v = |p: &[(&str, &str)], b: &str| bedingung(b, &w(p));
        // Zahlen werden als Zahlen verglichen, auch mit Komma
        assert!(v(&[("Preis", "9")], "Preis<10"));
        assert!(!v(&[("Preis", "9")], "Preis>10"));
        assert!(v(&[("Preis", "12,50")], "Preis > 12"));
        assert!(v(&[("Preis", "1.234,5")], "Preis>=1234.5"));
        assert!(v(&[("Preis", "10")], "Preis<=10"));
        assert!(v(&[("Preis", "10.0")], "Preis=10"));
        // größer/kleiner mit Text ist falsch
        assert!(!v(&[("A", "abc")], "A>1"));
        // Vergleichswert aus einer Variable
        assert!(v(&[("Preis", "120"), ("Limit", "100")], "Preis>$Limit"));
        assert!(!v(&[("Preis", "80"), ("Limit", "100")], "$Preis > $Limit"));
        assert!(v(&[("A", "Frau"), ("B", "frau")], "A=$B"));
        // enthält, enthält nicht, leer
        assert!(v(&[("Mail", "max@ACME.de")], "Mail~acme"));
        assert!(v(&[("Mail", "max@web.de")], "Mail!~acme"));
        assert!(v(&[], "!Firma"));
        assert!(!v(&[("Firma", "ACME")], "!Firma"));
        // als Block
        assert_eq!(einsetzen("{{?Betrag}}{{#if Betrag>=100}} Rabatt{{#else}} kein Rabatt{{#end}}", &w(&[("Betrag", "150")]), &auto), "150 Rabatt");
        assert_eq!(einsetzen("{{#set Art}}{{?Art:Privat|Firma}}{{#end}}{{#if Art=Firma}}Sehr geehrte Damen und Herren{{#else}}Hallo{{#end}}", &w(&[("Art", "Firma")]), &auto), "Sehr geehrte Damen und Herren");
    }

    #[test]
    fn bedingungen_f42_ac6_ac7_ac8_ac10() {
        let auto = auto_f42();
        let inhalt = "{{?Anrede:Herr|Frau}} {{#if Anrede=frau }}Liebe{{#else}}Lieber{{#end}}";
        assert_eq!(einsetzen(inhalt, &w(&[("Anrede", "Frau")]), &auto), "Frau Liebe");
        assert_eq!(einsetzen(inhalt, &w(&[("Anrede", "Herr")]), &auto), "Herr Lieber");
        let firma = "{{?Firma}}{{#if Firma}}, z. Hd. {{?Ansprechpartner}}{{#end}}";
        let a = auswerten(firma, &w(&[]), &auto);
        assert_eq!((a.text.as_str(), a.sichtbar.clone()), ("", vec!["Firma".to_string()]));
        let a = auswerten(firma, &w(&[("Firma", "ACME"), ("Ansprechpartner", "Udo")]), &auto);
        assert_eq!((a.text.as_str(), a.sichtbar), ("ACME, z. Hd. Udo", vec!["Firma".to_string(), "Ansprechpartner".to_string()]));
        // verschachtelt, != und nicht gewählte Zweige wirken nicht
        let t = "{{#if a!=x}}1{{#if b}}2{{#else}}3{{$v=nein}}{{#end}}{{#else}}4{{#end}}|{{$v}}";
        assert_eq!(einsetzen(t, &w(&[("a", "y"), ("b", "1")]), &auto), "12|");
        assert_eq!(einsetzen(t, &w(&[("a", "y")]), &auto), "13|nein");
        assert_eq!(einsetzen(t, &w(&[("a", " X ")]), &auto), "4|");
        assert_eq!(einsetzen("{{#if a}}{{#if b}}{{#if c}}{{#if d}}{{#if e}}tief{{#end}}{{#end}}{{#end}}{{#end}}{{#end}}", &w(&[("a", "1"), ("b", "1"), ("c", "1"), ("d", "1"), ("e", "1")]), &auto), "tief");
        assert_eq!(einsetzen("{{{{#if a}}", &w(&[]), &auto), "{{#if a}}");
    }

    #[test]
    fn datum_verschoben_f42_ac11() {
        let auto = auto_f42();
        let t = |s: &str| einsetzen(s, &w(&[]), &auto);
        assert_eq!(t("{{=date+14}}"), "14.02.2026");
        assert_eq!(t("{{=date+1m:d. MMMM}}"), "28. Februar");
        assert_eq!(t("{{=datum-1y}}"), "31.01.2025");
        assert_eq!(t("{{=date+2w:EEEE}}"), "Samstag");
        assert_eq!(t("{{=date-31d:yyyy-MM-dd}}"), "2025-12-31");
        let en = Automatisch { de: false, ..auto_f42() };
        assert_eq!(einsetzen("{{=date+1}}", &w(&[]), &en), "2026-02-01");
    }

    #[test]
    fn kalenderwoche_und_tag_des_jahres_f42_ac12() {
        assert_eq!(datum_mit_format(zeit("2027-01-01 00:00"), "ww w DDD D", true), "53 53 001 1");
        assert_eq!(datum_mit_format(zeit("2026-03-06 09:05"), "'KW' ww, 'Tag' D", true), "KW 10, Tag 65");
    }

    #[test]
    fn zufall_f42_ac17() {
        let mut gesehen = std::collections::HashSet::new();
        for zufall in 0..200 {
            let auto = Automatisch { zufall, ..auto_f42() };
            let t = einsetzen("{{=random:a|b|c}}", &w(&[]), &auto);
            assert!(["a", "b", "c"].contains(&t.as_str()), "{t}");
            // gleiche Zahl, gleiches Ergebnis (HTML und Text, F10)
            assert_eq!(t, einsetzen("{{=random:a|b|c}}", &w(&[]), &auto));
            gesehen.insert(t);
        }
        assert_eq!(gesehen.len(), 3);
    }

    #[test]
    fn pflicht_und_pruefung_in_abfragen_f42_ac14_ac15() {
        let a = abfragen("{{?Name*}} {{?Betrag:number}} {{?Mail*:email}} {{?Anrede*:Herr|Frau}}");
        assert_eq!(a.iter().map(|a| (a.name.as_str(), a.pflicht, a.pruefung.as_deref())).collect::<Vec<_>>(), [("Name", true, None), ("Betrag", false, Some("number")), ("Mail", true, Some("email")), ("Anrede", true, None)]);
        assert_eq!(a[1].optionen, None);
        assert!(gueltig(&a[1], "12,50") && gueltig(&a[1], "-3") && gueltig(&a[1], "") && !gueltig(&a[1], "12a"));
        assert!(gueltig(&a[2], "a@b.de") && !gueltig(&a[2], "a@b") && !gueltig(&a[2], ""));
        assert!(!gueltig(&a[0], " ") && gueltig(&a[0], "x"));
        assert_eq!(einsetzen("{{?Name*}}/{{?Betrag:number}}", &w(&[("Name", "A"), ("Betrag", "3")]), &auto_f42()), "A/3");
    }

    // ---------- F55 ----------

    #[test]
    fn auswahl_mit_eigenem_inhalt_f55() {
        let auto = auto_f42();
        let inhalt = "{{#choice Anrede}}\n{{#option Herr}}Lieber Herr {{?Name}}{{#option Frau}}Liebe Frau {{?Name}}{{#option Firma}}Hallo {{#choice Team}}{{#option A}}Team A{{#option B}}Team B{{#end}}{{#end}}!";
        let a = abfragen(inhalt);
        assert_eq!(a.iter().map(|a| (a.name.as_str(), a.optionen.clone())).collect::<Vec<_>>(), [
            ("Anrede", Some(vec!["Herr".to_string(), "Frau".into(), "Firma".into()])),
            ("Name", None),
            ("Team", Some(vec!["A".to_string(), "B".into()])),
        ]);
        let r = auswerten(inhalt, &w(&[("Anrede", "Frau"), ("Name", "Muster"), ("Team", "A")]), &auto);
        assert_eq!(r.text, "Liebe Frau Muster!");
        assert_eq!(r.sichtbar, ["Anrede", "Name"], "Team nur bei Firma");
        let r = auswerten(inhalt, &w(&[("Anrede", "Firma"), ("Team", "B")]), &auto);
        assert_eq!((r.text.as_str(), r.sichtbar), ("Hallo Team B!", vec!["Anrede".to_string(), "Team".into()]));
        assert_eq!(r.variablen["Anrede"], "Firma", "Auswahl ist Variable mit dem Namen der Möglichkeit");
        // nichts gewählt (z. B. ohne Popup): nichts eingefügt
        assert_eq!(einsetzen(inhalt, &w(&[]), &auto), "!");
    }

    #[test]
    fn variable_aus_inhalt_setzen_f55() {
        let auto = auto_f42();
        let inhalt = "{{#set Kunde}}{{?Vorname}} {{?Nachname}}{{#end}}Hallo {{$Kunde}}{{#choice Tarif}}{{#option Pro}}{{#set Preis}}19 {{$W}} bis {{=date+14}}{{#end}}{{#option Free}}{{$Preis=0}}{{#end}} – {{$Preis}}";
        let r = auswerten(inhalt, &w(&[("Vorname", "Udo"), ("Nachname", "[[Tab]]"), ("Tarif", "Pro"), ("W", "€")]), &auto);
        assert_eq!(r.text, "Hallo Udo [[[[Tab]] – 19 € bis 14.02.2026");
        assert_eq!(r.variablen["Kunde"], "Udo [[Tab]]", "Variablen halten Text, keine Tasten");
        assert_eq!(r.variablen["Preis"], "19 € bis 14.02.2026");
        assert_eq!(einsetzen(inhalt, &w(&[("Tarif", "Free")]), &auto), "Hallo   – 0");
        // in einem nicht gewählten Zweig wirkt #set nicht
        let mut a2 = auto_f42();
        a2.variablen.insert("X".into(), "alt".into());
        assert_eq!(auswerten("{{#if n}}{{#set X}}neu{{#end}}{{#end}}{{$X}}", &w(&[]), &a2).text, "alt");
    }
}
