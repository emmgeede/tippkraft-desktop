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
    pub inhalt: String,
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
    pub bausteine: Vec<Baustein>,
    pub ausschluesse: Vec<String>,
}

fn wahr() -> bool {
    true
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
    /// nur gelesen, wenn der Inhalt {{=clipboard}} enthält
    pub zwischenablage: Option<String>,
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
            einsetzen("{{?Anrede:Herr|Frau}} {{?Name}}, {{=datum}} {{{{?x}} {{?Name}} {{unbekannt}}", &werte, &Automatisch { datum: "04.11.2026".into(), zwischenablage: None }),
            // Werte werden maskiert: [[ → [[[[, damit sie Text bleiben. {{=datum}} im Wert bleibt Text.
            "Frau Muster [[[[Tab]] {{=datum}}, 04.11.2026 {{?x}} Muster [[[[Tab]] {{=datum}} {{unbekannt}}"
        );
        assert_eq!(aktionen(&einsetzen("{{?Name}}", &werte, &Automatisch::default())), vec![Aktion::Text("Muster [[Tab]] {{=datum}}".into())]);
    }
}
