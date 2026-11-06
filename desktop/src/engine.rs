//! Kernlogik: Puffer, Kürzel-Erkennung, Auslöser, Schreibweise, Rückgängig.
//! Ohne Tastatur und ohne Wayland, damit alles per Unit-Test prüfbar ist.

use crate::format::{Aktion, Ausloeser, Automatisch, Baustein, CURSOR, Stand, Taste, abfragen, aktionen, einsetzen};

/// Fenster des Abfrage-Popups. Darin löst tippkraft nie aus (F8).
pub const POPUP_APP_ID: &str = "tippkraft-popup";
/// Fenster der Baustein-Übersicht (F14), ebenfalls ausgeschlossen
pub const UEBERSICHT_APP_ID: &str = "tippkraft-uebersicht";

const PUFFER_MAX: usize = 64;

/// Was die Tastatur-Schicht aus einem Tastendruck macht
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eingabe {
    /// Druckbares Zeichen, ohne Strg/Alt/Super
    Zeichen(char),
    Leertaste,
    Enter,
    Tab,
    Backspace,
    /// Klick, Navigation, Tastenkürzel mit Modifier: Puffer verwerfen
    Zuruecksetzen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entscheidung {
    /// Taste normal weitergeben
    Durchlassen,
    /// Taste zurückhalten, `loeschen` Zeichen per Backspace entfernen, dann `aktionen` ausführen
    Ersetzen { loeschen: usize, aktionen: Vec<Aktion> },
    /// Taste zurückhalten und Popup öffnen. Danach `nach_popup` tippen oder bei Abbruch `abbruch`.
    Popup { titel: String, loeschen: usize, inhalt: String, gross: bool, abbruch: Vec<Aktion> },
}

/// Getippte Zeichen. Debug zeigt bewusst nur die Länge (Prinzip 2).
#[derive(Default)]
struct Puffer(Vec<char>);

impl std::fmt::Debug for Puffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Puffer({} Zeichen)", self.0.len())
    }
}

impl Puffer {
    fn push(&mut self, c: char) {
        if self.0.len() == PUFFER_MAX {
            self.0.remove(0);
        }
        self.0.push(c);
    }
}

#[derive(Debug)]
struct Rueckgaengig {
    eingefuegt: usize,
    wiederherstellen: String,
}

#[derive(Debug)]
pub struct Engine {
    bausteine: Vec<Baustein>,
    ausschluesse: Vec<String>,
    puffer: Puffer,
    letzte: Option<Rueckgaengig>,
    ausgeschlossen: bool,
    festes_datum: Option<String>,
    /// Nur für Tests: fester Zeitpunkt für {{=date:FORMAT}}
    festes_jetzt: Option<String>,
    feste_zwischenablage: Option<String>,
}

impl Engine {
    pub fn new(stand: Stand) -> Self {
        let mut e = Engine { bausteine: vec![], ausschluesse: vec![], puffer: Puffer::default(), letzte: None, ausgeschlossen: false, festes_datum: None, festes_jetzt: None, feste_zwischenablage: None };
        e.stand(stand);
        e
    }

    pub fn stand(&mut self, stand: Stand) {
        // Deaktivierte nur für die Übersicht (F14), sie lösen nie aus
        let mut bausteine: Vec<Baustein> = stand.bausteine.into_iter().filter(|b| b.aktiv).collect();
        // längstes Kürzel zuerst, damit es bei mehreren Treffern gewinnt
        bausteine.sort_by_key(|b| std::cmp::Reverse(b.kuerzel.chars().count()));
        self.bausteine = bausteine;
        self.ausschluesse = stand.ausschluesse.iter().map(|a| a.to_lowercase()).collect();
    }

    /// Fokus auf ein anderes Fenster: Puffer verwerfen, Ausschlussliste prüfen
    pub fn fokus(&mut self, app_id: Option<&str>) {
        self.reset();
        self.ausgeschlossen = app_id.is_some_and(|a| a == POPUP_APP_ID || a == UEBERSICHT_APP_ID || self.ausschluesse.contains(&a.to_lowercase()));
    }

    /// Nur für Tests: festes Datum statt heute
    pub fn datum_fest(&mut self, d: &str) {
        self.festes_datum = Some(d.into());
    }

    pub fn datum(&self) -> String {
        self.festes_datum.clone().unwrap_or_else(heute)
    }

    /// Nur für Tests: feste Zwischenablage statt wl-paste
    pub fn zwischenablage_fest(&mut self, z: &str) {
        self.feste_zwischenablage = Some(z.into());
    }

    /// Automatische Werte für diesen Inhalt. Die Zwischenablage wird nur gelesen, wenn sie gebraucht wird.
    pub fn automatisch(&self, inhalt: &str) -> Automatisch {
        let zwischenablage = inhalt.contains("{{=clipboard}}").then(|| self.feste_zwischenablage.clone().unwrap_or_else(zwischenablage_lesen));
        let jetzt = self.festes_jetzt.clone().unwrap_or_else(|| chrono::Local::now().naive_local().format("%Y-%m-%dT%H:%M").to_string());
        Automatisch { datum: self.datum(), jetzt, de: crate::i18n::aktuell() == crate::i18n::Sprache::De, zwischenablage }
    }

    pub fn anzahl(&self) -> usize {
        self.bausteine.len()
    }

    pub fn puffer_laenge(&self) -> usize {
        self.puffer.0.len()
    }

    fn reset(&mut self) {
        self.puffer.0.clear();
        self.letzte = None;
    }

    pub fn eingabe(&mut self, e: Eingabe) -> Entscheidung {
        if self.ausgeschlossen {
            return Entscheidung::Durchlassen;
        }
        if let Some(r) = self.letzte.take()
            && e == Eingabe::Backspace
        {
            self.puffer.0.clear();
            return Entscheidung::Ersetzen { loeschen: r.eingefuegt, aktionen: vec![Aktion::Text(r.wiederherstellen)] };
        }
        match e {
            Eingabe::Zeichen(c) => {
                self.puffer.push(c);
                match self.treffer(Ausloeser::Sofort) {
                    // das vervollständigende Zeichen ist nie beim Programm angekommen
                    Some((b, getippt)) => self.ersetzen(b, getippt, 1, ""),
                    None => Entscheidung::Durchlassen,
                }
            }
            Eingabe::Leertaste => match self.treffer(Ausloeser::Leertaste) {
                Some((b, getippt)) => self.ersetzen(b, getippt, 0, " "),
                None => {
                    self.puffer.push(' ');
                    Entscheidung::Durchlassen
                }
            },
            Eingabe::Enter | Eingabe::Tab => {
                let ausloeser = if e == Eingabe::Enter { Ausloeser::Enter } else { Ausloeser::Tab };
                match self.treffer(ausloeser) {
                    Some((b, getippt)) => self.ersetzen(b, getippt, 0, ""),
                    None => {
                        self.reset();
                        Entscheidung::Durchlassen
                    }
                }
            }
            Eingabe::Backspace => {
                self.puffer.0.pop();
                Entscheidung::Durchlassen
            }
            Eingabe::Zuruecksetzen => {
                self.reset();
                Entscheidung::Durchlassen
            }
        }
    }

    /// Passender Baustein für diesen Auslöser und das tatsächlich getippte Kürzel
    fn treffer(&self, ausloeser: Ausloeser) -> Option<(Baustein, String)> {
        let p = &self.puffer.0;
        self.bausteine.iter().filter(|b| b.ausloeser == ausloeser).find_map(|b| {
            let k: Vec<char> = b.kuerzel.chars().collect();
            if k.len() > p.len() {
                return None;
            }
            let start = p.len() - k.len();
            let getippt: String = p[start..].iter().collect();
            let gleich = if b.gross_klein_beachten { getippt == b.kuerzel } else { getippt.to_lowercase() == b.kuerzel.to_lowercase() };
            let grenze = !b.wortgrenze || start == 0 || !p[start - 1].is_alphanumeric();
            (gleich && grenze).then(|| (b.clone(), getippt))
        })
    }

    /// `zurueckgehalten`: Zeichen des Kürzels, die nie ans Programm gingen. `nachsatz`: verbrauchter Auslöser für Rückgängig.
    fn ersetzen(&mut self, b: Baustein, getippt: String, zurueckgehalten: usize, nachsatz: &str) -> Entscheidung {
        let gross = !b.gross_klein_beachten && b.schreibweise_uebernehmen && gross_anfang(&getippt);
        let loeschen = getippt.chars().count() - zurueckgehalten;
        self.puffer.0.clear();
        if !abfragen(&b.inhalt).is_empty() {
            self.letzte = None;
            // Bei Abbruch den zurückgehaltenen Auslöser nachholen, als wäre nichts passiert
            let abbruch = match b.ausloeser {
                Ausloeser::Sofort => vec![Aktion::Text(getippt.chars().last().map(String::from).unwrap_or_default())],
                Ausloeser::Leertaste => vec![Aktion::Text(" ".into())],
                Ausloeser::Enter => vec![Aktion::Taste(Taste::Enter, 1)],
                Ausloeser::Tab => vec![Aktion::Taste(Taste::Tab, 1)],
            };
            return Entscheidung::Popup { titel: getippt, loeschen, inhalt: b.inhalt, gross, abbruch };
        }
        let akt = nach_popup(&b.inhalt, &Default::default(), &self.automatisch(&b.inhalt), gross);
        let nur_text = akt.iter().all(|a| matches!(a, Aktion::Text(_)));
        let eingefuegt = akt.iter().map(|a| if let Aktion::Text(t) = a { t.chars().count() } else { 0 }).sum();
        self.letzte = nur_text.then(|| Rueckgaengig { eingefuegt, wiederherstellen: format!("{getippt}{nachsatz}") });
        Entscheidung::Ersetzen { loeschen, aktionen: akt }
    }
}

/// Fertige Aktionen: Werte, Datum, Zwischenablage einsetzen, Tokens auswerten, Cursor setzen, ggf. ersten Buchstaben großschreiben
pub fn nach_popup(inhalt: &str, werte: &std::collections::HashMap<String, String>, auto: &Automatisch, gross: bool) -> Vec<Aktion> {
    let mut akt = cursor_anwenden(aktionen(&einsetzen(inhalt, werte, auto)));
    if gross && let Some(Aktion::Text(t)) = akt.iter_mut().find(|a| matches!(a, Aktion::Text(_))) {
        let mut cs = t.chars();
        if let Some(first) = cs.next() {
            *t = first.to_uppercase().chain(cs).collect();
        }
    }
    akt
}

/// Heutiges Datum im Format der aktuellen Sprache (F15): DE TT.MM.JJJJ, EN JJJJ-MM-TT
/// Entfernt die Cursor-Markierung und hängt so viele "Pfeil links" an, wie Zeichen dahinter stehen.
/// Folgen dahinter Tasten-Tokens (etwa aus einem Verweis), ist die Position unklar: Cursor ignorieren.
fn cursor_anwenden(akt: Vec<Aktion>) -> Vec<Aktion> {
    let Some(i) = akt.iter().position(|a| matches!(a, Aktion::Text(t) if t.contains(CURSOR))) else { return akt };
    let tasten_danach = akt[i + 1..].iter().any(|a| matches!(a, Aktion::Taste(..)));
    let mut zeichen_danach: usize = akt[i + 1..].iter().map(|a| if let Aktion::Text(t) = a { t.chars().count() } else { 0 }).sum();
    if let Aktion::Text(t) = &akt[i] {
        zeichen_danach += t.split(CURSOR).skip(1).collect::<String>().chars().count();
    }
    let mut out: Vec<Aktion> = akt
        .into_iter()
        .map(|a| if let Aktion::Text(t) = a { Aktion::Text(t.replace(CURSOR, "")) } else { a })
        .filter(|a| !matches!(a, Aktion::Text(t) if t.is_empty()))
        .collect();
    if !tasten_danach {
        while zeichen_danach > 0 {
            let n = zeichen_danach.min(255);
            out.push(Aktion::Taste(Taste::Left, n as u8));
            zeichen_danach -= n;
        }
    }
    out
}

/// Text aus der Wayland-Zwischenablage, leer wenn keiner da ist (höchstens 20 000 Zeichen)
fn zwischenablage_lesen() -> String {
    std::process::Command::new("wl-paste")
        .args(["--no-newline", "--type", "text/plain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).chars().take(20_000).collect())
        .unwrap_or_default()
}

fn heute() -> String {
    datum_formatieren(chrono::Local::now().date_naive(), crate::i18n::aktuell())
}

pub fn datum_formatieren(d: chrono::NaiveDate, s: crate::i18n::Sprache) -> String {
    d.format(if s == crate::i18n::Sprache::De { "%d.%m.%Y" } else { "%Y-%m-%d" }).to_string()
}

/// Großer Anfangsbuchstabe, aber nicht komplett groß (bei mehr als einem Buchstaben)
fn gross_anfang(s: &str) -> bool {
    let buchstaben: Vec<char> = s.chars().filter(|c| c.is_alphabetic()).collect();
    let Some(erster) = buchstaben.first() else { return false };
    erster.is_uppercase() && !(buchstaben.len() > 1 && buchstaben.iter().all(|c| c.is_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{Aktion, Ausloeser, Baustein, Stand, Taste};

    fn b(kuerzel: &str, inhalt: &str, ausloeser: Ausloeser) -> Baustein {
        Baustein {
            kuerzel: kuerzel.into(),
            inhalt: inhalt.into(),
            bezeichnung: None,
            aktiv: true,
            ausloeser,
            wortgrenze: true,
            gross_klein_beachten: false,
            schreibweise_uebernehmen: true,
        }
    }

    fn engine(bausteine: Vec<Baustein>) -> Engine {
        Engine::new(Stand { sprache: None, desktop_version: None, version: 1, bausteine, ausschluesse: vec!["org.keepassxc.KeePassXC".into()] })
    }

    /// Tippt einen Text Zeichen für Zeichen (' ' als Leertaste) und liefert die Entscheidungen
    fn tippe(e: &mut Engine, text: &str) -> Vec<Entscheidung> {
        text.chars().map(|c| e.eingabe(if c == ' ' { Eingabe::Leertaste } else { Eingabe::Zeichen(c) })).collect()
    }

    fn ersetzt(loeschen: usize, text: &str) -> Entscheidung {
        Entscheidung::Ersetzen { loeschen, aktionen: vec![Aktion::Text(text.into())] }
    }

    use Entscheidung::Durchlassen;

    #[test]
    fn leertaste_ersetzt_und_wird_verbraucht_ac1() {
        let mut e = engine(vec![b("mfg", "Mit freundlichen Grüßen", Ausloeser::Leertaste)]);
        assert_eq!(tippe(&mut e, "mfg "), vec![Durchlassen, Durchlassen, Durchlassen, ersetzt(3, "Mit freundlichen Grüßen")]);
    }

    #[test]
    fn sofort_haelt_das_letzte_zeichen_zurueck_ac2() {
        let mut e = engine(vec![b(";s", "Signatur", Ausloeser::Sofort)]);
        assert_eq!(tippe(&mut e, ";s"), vec![Durchlassen, ersetzt(1, "Signatur")]);
    }

    #[test]
    fn enter_und_tab_werden_verbraucht_ac3() {
        let mut e = engine(vec![b("mfg", "x", Ausloeser::Enter), b("adr", "y", Ausloeser::Tab)]);
        tippe(&mut e, "mfg");
        assert_eq!(e.eingabe(Eingabe::Enter), ersetzt(3, "x"));
        tippe(&mut e, "adr");
        assert_eq!(e.eingabe(Eingabe::Tab), ersetzt(3, "y"));
        // ohne Treffer: durchlassen und Puffer leeren
        tippe(&mut e, "ad");
        assert_eq!(e.eingabe(Eingabe::Tab), Durchlassen);
        tippe(&mut e, "r");
        assert_eq!(e.eingabe(Eingabe::Tab), Durchlassen);
    }

    #[test]
    fn falscher_ausloeser_tut_nichts_ac4() {
        let mut e = engine(vec![b("mfg", "x", Ausloeser::Leertaste)]);
        tippe(&mut e, "mfg");
        assert_eq!(e.eingabe(Eingabe::Enter), Durchlassen);
    }

    #[test]
    fn wortgrenze_ac5() {
        let mut e = engine(vec![b("mfg", "x", Ausloeser::Leertaste)]);
        assert_eq!(tippe(&mut e, "xmfg ").last(), Some(&Durchlassen));
        assert_eq!(tippe(&mut e, "(mfg ").last(), Some(&ersetzt(3, "x")));
        assert_eq!(tippe(&mut e, "mfg ").last(), Some(&ersetzt(3, "x")), "Pufferanfang zählt als Grenze");
        assert_eq!(tippe(&mut e, "ä").last(), Some(&Durchlassen));
        assert_eq!(tippe(&mut e, "mfg ").last(), Some(&Durchlassen), "Umlaut ist ein Buchstabe");

        let mut ohne = engine(vec![Baustein { wortgrenze: false, ..b("mfg", "x", Ausloeser::Leertaste) }]);
        assert_eq!(tippe(&mut ohne, "xmfg ").last(), Some(&ersetzt(3, "x")));
    }

    #[test]
    fn gross_klein_ac6() {
        let mut e = engine(vec![b("mfg", "x", Ausloeser::Leertaste)]);
        assert_eq!(tippe(&mut e, "MFG ").last(), Some(&ersetzt(3, "x")));
        assert_eq!(tippe(&mut e, "Mfg ").last(), Some(&ersetzt(3, "X")));

        let mut genau = engine(vec![Baustein { gross_klein_beachten: true, ..b("mfg", "x", Ausloeser::Leertaste) }]);
        assert_eq!(tippe(&mut genau, "Mfg ").last(), Some(&Durchlassen));
        assert_eq!(tippe(&mut genau, "mfg ").last(), Some(&ersetzt(3, "x")));
    }

    #[test]
    fn schreibweise_ac7() {
        let inhalt = "mit freundlichen Grüßen";
        let mut e = engine(vec![b("mfg", inhalt, Ausloeser::Leertaste)]);
        assert_eq!(tippe(&mut e, "Mfg ").last(), Some(&ersetzt(3, "Mit freundlichen Grüßen")));
        assert_eq!(tippe(&mut e, "MFG ").last(), Some(&ersetzt(3, inhalt)));
        assert_eq!(tippe(&mut e, "mfg ").last(), Some(&ersetzt(3, inhalt)));

        let mut aus = engine(vec![Baustein { schreibweise_uebernehmen: false, ..b("mfg", inhalt, Ausloeser::Leertaste) }]);
        assert_eq!(tippe(&mut aus, "Mfg ").last(), Some(&ersetzt(3, inhalt)));

        // einbuchstabiges Kürzel: großgeschrieben heißt großer Anfang
        let mut kurz = engine(vec![b("m", "mit", Ausloeser::Leertaste)]);
        assert_eq!(tippe(&mut kurz, "M ").last(), Some(&ersetzt(1, "Mit")));
        // beginnt der Inhalt mit einem Token, betrifft es den ersten Text
        let mut tok = engine(vec![b("t", "[[Tab]]hallo", Ausloeser::Leertaste)]);
        assert_eq!(
            tippe(&mut tok, "T ").last(),
            Some(&Entscheidung::Ersetzen { loeschen: 1, aktionen: vec![Aktion::Taste(Taste::Tab, 1), Aktion::Text("Hallo".into())] })
        );
    }

    #[test]
    fn rueckgaengig_ac8() {
        let mut e = engine(vec![b("mfg", "Grüße 🙂", Ausloeser::Leertaste), b(";s", "Sig", Ausloeser::Sofort)]);
        tippe(&mut e, "mfg ");
        assert_eq!(e.eingabe(Eingabe::Backspace), ersetzt(7, "mfg "), "7 Zeichen inkl. Emoji");
        assert_eq!(e.eingabe(Eingabe::Backspace), Durchlassen, "nur einmal");

        tippe(&mut e, ";s");
        assert_eq!(e.eingabe(Eingabe::Backspace), ersetzt(3, ";s"), "sofort: ohne Leerzeichen");

        tippe(&mut e, "mfg x");
        assert_eq!(e.eingabe(Eingabe::Backspace), Durchlassen, "nach anderer Taste normal");

        let mut tok = engine(vec![b("adr", "a[[Tab]]b", Ausloeser::Leertaste)]);
        tippe(&mut tok, "adr ");
        assert_eq!(tok.eingabe(Eingabe::Backspace), Durchlassen, "mit Tasten-Token kein Rückgängig");
    }

    #[test]
    fn zuruecksetzen_leert_den_puffer_ac9() {
        let mut e = engine(vec![b("mfg", "x", Ausloeser::Leertaste)]);
        tippe(&mut e, "mf");
        assert_eq!(e.eingabe(Eingabe::Zuruecksetzen), Durchlassen);
        assert_eq!(tippe(&mut e, "g ").last(), Some(&Durchlassen));
        tippe(&mut e, "mf");
        e.fokus(Some("firefox"));
        assert_eq!(tippe(&mut e, "g ").last(), Some(&Durchlassen));
    }

    #[test]
    fn backspace_korrigiert_den_puffer_ac10() {
        let mut e = engine(vec![b("mfg", "x", Ausloeser::Leertaste)]);
        tippe(&mut e, "mfx");
        assert_eq!(e.eingabe(Eingabe::Backspace), Durchlassen);
        assert_eq!(tippe(&mut e, "g ").last(), Some(&ersetzt(3, "x")));
    }

    #[test]
    fn puffer_hat_hoechstens_64_zeichen_ac11() {
        let mut e = engine(vec![]);
        tippe(&mut e, &"x".repeat(100));
        assert_eq!(e.puffer_laenge(), 64);
    }

    #[test]
    fn ausgeschlossene_app_ac12() {
        let mut e = engine(vec![b("mfg", "x", Ausloeser::Leertaste)]);
        e.fokus(Some("org.keepassxc.keepassxc"));
        assert_eq!(tippe(&mut e, "mfg ").last(), Some(&Durchlassen));
        assert_eq!(e.puffer_laenge(), 0);
        e.fokus(Some("foot"));
        assert_eq!(tippe(&mut e, "mfg ").last(), Some(&ersetzt(3, "x")));
    }

    #[test]
    fn laengstes_kuerzel_gewinnt_ac13() {
        let mut e = engine(vec![Baustein { wortgrenze: false, ..b("fg", "kurz", Ausloeser::Leertaste) }, b("mfg", "lang", Ausloeser::Leertaste)]);
        assert_eq!(tippe(&mut e, "mfg ").last(), Some(&ersetzt(3, "lang")));
    }

    #[test]
    fn puffer_erscheint_nicht_in_debug_ausgaben() {
        let mut e = engine(vec![]);
        tippe(&mut e, "geheim");
        assert!(!format!("{e:?}").contains("geheim"));
    }

    #[test]
    fn neuer_stand_ersetzt_bausteine() {
        let mut e = engine(vec![]);
        e.stand(Stand { sprache: None, desktop_version: None, version: 1, bausteine: vec![b("mfg", "x", Ausloeser::Leertaste)], ausschluesse: vec![] });
        assert_eq!(tippe(&mut e, "mfg ").last(), Some(&ersetzt(3, "x")));
    }

    // ---------- F8 ----------

    #[test]
    fn datum_ohne_popup_ac5() {
        let mut e = engine(vec![b("dat", "Stand: {{=datum}}", Ausloeser::Leertaste)]);
        e.datum_fest("04.11.2026");
        assert_eq!(tippe(&mut e, "dat ").last(), Some(&ersetzt(3, "Stand: 04.11.2026")));
    }

    #[test]
    fn abfragen_oeffnen_das_popup_ac6_ac9() {
        let inhalt = "{{?Anrede:Herr|Frau}} {{?Name}}";
        let mut e = engine(vec![b("brief", inhalt, Ausloeser::Leertaste), b(";p", inhalt, Ausloeser::Sofort), b("pe", inhalt, Ausloeser::Enter)]);
        let erwartet = |loeschen, gross, abbruch| Entscheidung::Popup { titel: "brief".into(), loeschen, inhalt: inhalt.into(), gross, abbruch };
        assert_eq!(tippe(&mut e, "brief ").last(), Some(&erwartet(5, false, vec![Aktion::Text(" ".into())])));
        assert!(matches!(tippe(&mut e, "Brief ").last(), Some(Entscheidung::Popup { titel, gross: true, .. }) if titel == "Brief"));
        assert!(matches!(tippe(&mut e, ";p").last(), Some(Entscheidung::Popup { loeschen: 1, abbruch, .. }) if *abbruch == vec![Aktion::Text("p".into())]));
        tippe(&mut e, "pe");
        assert!(matches!(e.eingabe(Eingabe::Enter), Entscheidung::Popup { abbruch, .. } if abbruch == vec![Aktion::Taste(Taste::Enter, 1)]));
        // kein Rückgängig nach Popup-Bausteinen
        assert_eq!(e.eingabe(Eingabe::Backspace), Entscheidung::Durchlassen);
    }

    #[test]
    fn nach_dem_popup_ausfuellen_ac7() {
        let werte = std::collections::HashMap::from([("Anrede".to_string(), "frau".to_string()), ("Name".to_string(), "Muster".to_string())]);
        let auto = Automatisch { datum: "04.11.2026".into(), zwischenablage: None, ..Default::default() };
        assert_eq!(nach_popup("{{?Anrede:Herr|frau}} {{?Name}}, {{=datum}}", &werte, &auto, true), vec![Aktion::Text("Frau Muster, 04.11.2026".into())]);
        assert_eq!(nach_popup("{{?Name}}[[Tab]]", &werte, &auto, false), vec![Aktion::Text("Muster".into()), Aktion::Taste(Taste::Tab, 1)]);
    }

    #[test]
    fn im_popup_loest_nichts_aus_ac10() {
        let mut e = engine(vec![b("mfg", "x", Ausloeser::Leertaste)]);
        e.fokus(Some(POPUP_APP_ID));
        assert_eq!(tippe(&mut e, "mfg ").last(), Some(&Durchlassen));
    }

    #[test]
    fn deaktivierte_loesen_nicht_aus_f14_ac2() {
        let mut e = engine(vec![Baustein { aktiv: false, ..b("sig", "x", Ausloeser::Leertaste) }, b("mfg", "y", Ausloeser::Leertaste)]);
        assert_eq!(tippe(&mut e, "sig ").last(), Some(&Durchlassen));
        assert_eq!(tippe(&mut e, "mfg ").last(), Some(&ersetzt(3, "y")));
        assert_eq!(e.anzahl(), 1, "Status zählt nur auslösende");
    }

    #[test]
    fn in_der_uebersicht_loest_nichts_aus_f14_ac7() {
        let mut e = engine(vec![b("mfg", "x", Ausloeser::Leertaste)]);
        e.fokus(Some(UEBERSICHT_APP_ID));
        assert_eq!(tippe(&mut e, "mfg ").last(), Some(&Durchlassen));
    }

    #[test]
    fn datumsformat_nach_sprache_f15_ac9() {
        let d = chrono::NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
        assert_eq!(datum_formatieren(d, crate::i18n::Sprache::De), "04.11.2026");
        assert_eq!(datum_formatieren(d, crate::i18n::Sprache::En), "2026-11-04");
        let mut e = engine(vec![b("dat", "{{=date}} / {{=datum}}", Ausloeser::Leertaste)]);
        e.datum_fest("2026-11-04");
        assert_eq!(tippe(&mut e, "dat ").last(), Some(&ersetzt(3, "2026-11-04 / 2026-11-04")));
    }

    // ---------- F9 ----------

    #[test]
    fn zwischenablage_als_text_f9_ac1() {
        let mut e = engine(vec![b("zw", "Von: {{=clipboard}}", Ausloeser::Leertaste)]);
        e.zwischenablage_fest("Muster [[Tab]]");
        assert_eq!(tippe(&mut e, "zw ").last(), Some(&ersetzt(2, "Von: Muster [[Tab]]")));
    }

    #[test]
    fn cursor_mit_pfeil_links_f9_ac2() {
        let mut e = engine(vec![b("hi", "Hallo {{=cursor}}, viele Grüße", Ausloeser::Leertaste)]);
        assert_eq!(
            tippe(&mut e, "hi ").last(),
            Some(&Entscheidung::Ersetzen { loeschen: 2, aktionen: vec![Aktion::Text("Hallo , viele Grüße".into()), Aktion::Taste(Taste::Left, 13)] })
        );
        // über 255 Zeichen hinweg in mehreren Schritten, Zeilenumbrüche zählen als ein Zeichen
        let lang = format!("a{{{{=cursor}}}}{}\nb", "x".repeat(300));
        let akt = nach_popup(&lang, &Default::default(), &Default::default(), false);
        let links: usize = akt.iter().map(|a| if let Aktion::Taste(Taste::Left, n) = a { *n as usize } else { 0 }).sum();
        assert_eq!(links, 302);
        // hinter dem Cursor ein Tasten-Token (z. B. aus einem Verweis): Cursor wird ignoriert
        let akt = nach_popup("a{{=cursor}}b[[Tab]]", &Default::default(), &Default::default(), false);
        assert_eq!(akt, vec![Aktion::Text("ab".into()), Aktion::Taste(Taste::Tab, 1)]);
    }
}
