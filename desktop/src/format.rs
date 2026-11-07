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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Abfrage {
    pub name: String,
    pub optionen: Option<Vec<String>>,
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
    for s in stuecke(inhalt) {
        if let Stueck::Klammer(k) = s
            && let Some(rest) = k.strip_prefix('?')
        {
            let (name, optionen) = match rest.split_once(':') {
                Some((n, o)) => (n, Some(o.split('|').map(String::from).collect())),
                None => (rest, None),
            };
            if !out.iter().any(|a| a.name == name) {
                out.push(Abfrage { name: name.into(), optionen });
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
    let muster: [(&str, String); 12] = [
        ("yyyy", format!("{:04}", t.year())),
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

/// Setzt Werte, Datum, Zwischenablage und Cursor-Markierung ein und macht aus "{{{{" ein "{{".
/// Werte werden maskiert, damit "[[" darin Text bleibt.
pub fn einsetzen(inhalt: &str, werte: &std::collections::HashMap<String, String>, auto: &Automatisch) -> String {
    stuecke(inhalt)
        .into_iter()
        .map(|s| match s {
            Stueck::Text(t) => t.to_string(),
            Stueck::Klammer("=date" | "=datum") => auto.datum.clone(),
            Stueck::Klammer(k) if k.starts_with("=date:") || k.starts_with("=datum:") => {
                let fmt = k.split_once(':').map(|(_, f)| f).unwrap_or_default();
                match chrono::NaiveDateTime::parse_from_str(&auto.jetzt, "%Y-%m-%dT%H:%M") {
                    Ok(t) if !fmt.is_empty() => datum_mit_format(t, fmt, auto.de),
                    _ => auto.datum.clone(),
                }
            }
            Stueck::Klammer("=clipboard") => auto.zwischenablage.as_deref().unwrap_or_default().replace("[[", "[[[["),
            Stueck::Klammer("=cursor") => CURSOR.to_string(),
            Stueck::Klammer(k) => match k.strip_prefix('?') {
                Some(rest) => werte.get(rest.split(':').next().unwrap_or_default()).map(|w| w.replace("[[", "[[[[")).unwrap_or_default(),
                None => format!("{{{{{k}}}}}"),
            },
        })
        .collect()
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
        let auto = Automatisch { datum: "06.03.2026".into(), jetzt: "2026-03-06T09:05".into(), de: true, zwischenablage: None };
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
                Abfrage { name: "Anrede".into(), optionen: Some(vec!["Herr".into(), "Frau".into()]) },
                Abfrage { name: "Name".into(), optionen: None }
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
        assert_eq!(a, vec![Abfrage { name: "Name".into(), optionen: None }, Abfrage { name: "Gruß".into(), optionen: Some(vec!["Viele Grüße".into(), "".into()]) }]);
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
}
