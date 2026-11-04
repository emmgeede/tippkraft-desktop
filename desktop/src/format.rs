//! Formatvertrag Server → Desktop, siehe shared/FORMAT.md

use serde::Deserialize;

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
    pub ausloeser: Ausloeser,
    pub wortgrenze: bool,
    pub gross_klein_beachten: bool,
    pub schreibweise_uebernehmen: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Stand {
    pub version: u32,
    pub bausteine: Vec<Baustein>,
    pub ausschluesse: Vec<String>,
}

pub fn parse(json: &str) -> Result<Stand, String> {
    let stand: Stand = serde_json::from_str(json).map_err(|e| format!("Stand nicht lesbar: {e}"))?;
    if stand.version != 1 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liest_den_vertrag_ac15() {
        let json = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../shared/snippets.example.json")).unwrap();
        let stand = parse(&json).unwrap();
        assert_eq!(stand.bausteine.len(), 3);
        assert_eq!(stand.ausschluesse, vec!["1password", "org.keepassxc.KeePassXC"]);
        let sig = &stand.bausteine[0];
        assert_eq!(sig.kuerzel, ";sig");
        assert_eq!(sig.ausloeser, Ausloeser::Sofort);
        assert!(!sig.wortgrenze && !sig.gross_klein_beachten && !sig.schreibweise_uebernehmen);
        assert_eq!(stand.bausteine[1].ausloeser, Ausloeser::Tab);
        assert!(stand.bausteine[1].gross_klein_beachten);
        assert_eq!(stand.bausteine[2].ausloeser, Ausloeser::Leertaste);
    }

    #[test]
    fn lehnt_unbekannte_version_ab_ac15() {
        let err = parse(r#"{"version":2,"bausteine":[],"ausschluesse":[]}"#).unwrap_err();
        assert!(err.contains("Version 2"), "{err}");
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
}
