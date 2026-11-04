//! Kernlogik: Puffer, Kürzel-Erkennung, Auslöser, Schreibweise, Rückgängig.
//! Ohne Tastatur und ohne Wayland, damit alles per Unit-Test prüfbar ist.

use crate::format::{Aktion, Ausloeser, Baustein, Stand, aktionen};

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
}

impl Engine {
    pub fn new(stand: Stand) -> Self {
        let mut e = Engine { bausteine: vec![], ausschluesse: vec![], puffer: Puffer::default(), letzte: None, ausgeschlossen: false };
        e.stand(stand);
        e
    }

    pub fn stand(&mut self, stand: Stand) {
        let mut bausteine = stand.bausteine;
        // längstes Kürzel zuerst, damit es bei mehreren Treffern gewinnt
        bausteine.sort_by_key(|b| std::cmp::Reverse(b.kuerzel.chars().count()));
        self.bausteine = bausteine;
        self.ausschluesse = stand.ausschluesse.iter().map(|a| a.to_lowercase()).collect();
    }

    /// Fokus auf ein anderes Fenster: Puffer verwerfen, Ausschlussliste prüfen
    pub fn fokus(&mut self, app_id: Option<&str>) {
        self.reset();
        self.ausgeschlossen = app_id.is_some_and(|a| self.ausschluesse.contains(&a.to_lowercase()));
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
        let mut akt = aktionen(&b.inhalt);
        if !b.gross_klein_beachten
            && b.schreibweise_uebernehmen
            && gross_anfang(&getippt)
            && let Some(Aktion::Text(t)) = akt.iter_mut().find(|a| matches!(a, Aktion::Text(_)))
        {
            let mut cs = t.chars();
            if let Some(first) = cs.next() {
                *t = first.to_uppercase().chain(cs).collect();
            }
        }
        let nur_text = akt.iter().all(|a| matches!(a, Aktion::Text(_)));
        let eingefuegt = akt.iter().map(|a| if let Aktion::Text(t) = a { t.chars().count() } else { 0 }).sum();
        self.puffer.0.clear();
        self.letzte = nur_text.then(|| Rueckgaengig { eingefuegt, wiederherstellen: format!("{getippt}{nachsatz}") });
        Entscheidung::Ersetzen { loeschen: getippt.chars().count() - zurueckgehalten, aktionen: akt }
    }
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
            ausloeser,
            wortgrenze: true,
            gross_klein_beachten: false,
            schreibweise_uebernehmen: true,
        }
    }

    fn engine(bausteine: Vec<Baustein>) -> Engine {
        Engine::new(Stand { version: 1, bausteine, ausschluesse: vec!["org.keepassxc.KeePassXC".into()] })
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
        e.stand(Stand { version: 1, bausteine: vec![b("mfg", "x", Ausloeser::Leertaste)], ausschluesse: vec![] });
        assert_eq!(tippe(&mut e, "mfg ").last(), Some(&ersetzt(3, "x")));
    }
}
