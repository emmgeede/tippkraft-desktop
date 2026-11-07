//! Baustein-Übersicht (F14): eigener Prozess `tippkraft uebersicht`, liest stand.json, nur Anzeige und Filter.

use crate::format::{Ausloeser, Stand};
use crate::i18n::{Sprache, T, text, tr};

#[derive(Debug, Clone, PartialEq)]
pub struct Eintrag {
    pub kuerzel: String,
    pub ausloeser: &'static str,
    pub bezeichnung: Option<String>,
    pub vorschau: String,
    pub aktiv: bool,
    /// Inhalt in einer Zeile, für die Fundstelle in der Vorschau
    zeile: String,
}

/// Suchergebnis: Eintrag und Vorschau (bei Treffer im Inhalt die Fundstelle)
pub struct Treffer<'a> {
    pub eintrag: &'a Eintrag,
    pub vorschau: String,
}

fn ausloeser_name(s: Sprache, a: Ausloeser) -> &'static str {
    text(
        s,
        match a {
            Ausloeser::Sofort => T::AuslSofort,
            Ausloeser::Leertaste => T::AuslLeertaste,
            Ausloeser::Enter => T::AuslEnter,
            Ausloeser::Tab => T::AuslTab,
        },
    )
}

/// Kurzer Hinweis wie im Webtool (`auszug` in web/src/lib/bausteine.ts): eine Zeile, höchstens 80 Zeichen
fn vorschau(inhalt: &str) -> String {
    ausschnitt(&einzeilig(inhalt), 0)
}

fn einzeilig(inhalt: &str) -> String {
    inhalt.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 80 Zeichen ab `start` (in Zeichen), mit … an abgeschnittenen Enden
fn ausschnitt(eine: &str, start: usize) -> String {
    let n = eine.chars().count();
    let teil: String = eine.chars().skip(start).take(80).collect();
    format!("{}{}{}", if start > 0 { "…" } else { "" }, teil, if start + 80 < n { "…" } else { "" })
}

/// Aktive zuerst, dann deaktivierte, jeweils alphabetisch
pub fn eintraege(stand: &Stand, s: Sprache) -> Vec<Eintrag> {
    let mut e: Vec<Eintrag> = stand
        .bausteine
        .iter()
        .map(|b| Eintrag {
            kuerzel: b.kuerzel.clone(),
            ausloeser: ausloeser_name(s, b.ausloeser),
            bezeichnung: b.bezeichnung.clone(),
            vorschau: vorschau(&b.inhalt),
            aktiv: b.aktiv,
            zeile: einzeilig(&b.inhalt),
        })
        .collect();
    e.sort_by_key(|x| (!x.aktiv, x.kuerzel.to_lowercase()));
    e
}

/// Treffer im Kürzel zuerst, dann in der Bezeichnung, dann im Inhalt (mit Fundstelle in der Vorschau).
/// Innerhalb einer Stufe bleibt die Reihenfolge der Liste (aktive zuerst, alphabetisch).
pub fn filtern<'a>(e: &'a [Eintrag], suche: &str) -> Vec<Treffer<'a>> {
    let q = suche.trim().to_lowercase();
    if q.is_empty() {
        return e.iter().map(|x| Treffer { eintrag: x, vorschau: x.vorschau.clone() }).collect();
    }
    let mut out: Vec<(u8, Treffer<'a>)> = e
        .iter()
        .filter_map(|x| {
            if x.kuerzel.to_lowercase().contains(&q) {
                return Some((0, Treffer { eintrag: x, vorschau: x.vorschau.clone() }));
            }
            if x.bezeichnung.as_deref().is_some_and(|b| b.to_lowercase().contains(&q)) {
                return Some((1, Treffer { eintrag: x, vorschau: x.vorschau.clone() }));
            }
            // Fundstelle in Zeichen; Kleinschreibung ändert die Zeichenzahl fast nie, sonst Anfang zeigen
            let klein = x.zeile.to_lowercase();
            let pos = klein.find(&q)?;
            let start = if klein.chars().count() == x.zeile.chars().count() { klein[..pos].chars().count().saturating_sub(25) } else { 0 };
            Some((2, Treffer { eintrag: x, vorschau: ausschnitt(&x.zeile, start) }))
        })
        .collect();
    out.sort_by_key(|(stufe, _)| *stufe);
    out.into_iter().map(|(_, t)| t).collect()
}

struct App {
    eintraege: Vec<Eintrag>,
    suche: String,
    scroll: f32,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        use egui::{Key, Modifiers, RichText};
        let (esc, hoch, runter, bild_hoch, bild_runter) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::PageUp),
                i.consume_key(Modifiers::NONE, Key::PageDown),
            )
        });
        if esc {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.scroll = (self.scroll + 48.0 * (runter as i32 - hoch as i32) as f32 + 400.0 * (bild_runter as i32 - bild_hoch as i32) as f32).max(0.0);
        let f = crate::popup::farben(ui.visuals().dark_mode);
        let treffer = filtern(&self.eintraege, &self.suche);

        let rand = egui::Frame::new().fill(f.bg).inner_margin(egui::Margin::same(20));
        egui::CentralPanel::default().frame(rand).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr(T::UebTitel)).size(18.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let n = self.eintraege.len();
                    let sp = crate::i18n::aktuell();
                    let text = if self.suche.trim().is_empty() { crate::i18n::f(sp, T::UebAnzahl, &[&n]) } else { crate::i18n::f(sp, T::UebVonN, &[&treffer.len(), &n]) };
                    ui.label(RichText::new(text).color(f.muted).size(13.0));
                });
            });
            ui.add_space(8.0);
            // Suchfeld behält immer den Fokus: Es ist das einzige bedienbare Element
            let r = ui.add(egui::TextEdit::singleline(&mut self.suche).hint_text(tr(T::UebSuchen)).desired_width(f32::INFINITY).margin(egui::vec2(10.0, 8.0)));
            if !r.has_focus() {
                r.request_focus();
            }
            ui.add_space(10.0);

            if self.eintraege.is_empty() {
                ui.label(RichText::new(tr(T::UebLeer)).color(f.muted));
                return;
            }
            let aus = egui::ScrollArea::vertical().auto_shrink(false).vertical_scroll_offset(self.scroll).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for t in &treffer {
                    let e = t.eintrag;
                    let kuerzel_farbe = if e.aktiv { f.accent } else { f.muted };
                    let text_farbe = if e.aktiv { f.fg } else { f.muted };
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(4, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&e.kuerzel).monospace().color(kuerzel_farbe).size(15.0));
                            if let Some(bz) = &e.bezeichnung {
                                ui.label(RichText::new(bz).color(text_farbe));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let tag = if e.aktiv { e.ausloeser.to_string() } else { tr(T::UebLoestNicht).to_string() };
                                ui.label(RichText::new(tag).color(f.muted).size(12.5));
                            });
                        });
                        // immer genau eine Zeile, auch wenn das Fenster schmal ist
                        ui.add(egui::Label::new(RichText::new(&t.vorschau).monospace().color(f.muted).size(13.0)).truncate());
                    });
                    let y = ui.cursor().top();
                    ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, f.border));
                }
                if treffer.is_empty() {
                    ui.label(RichText::new(tr(T::UebKeineTreffer)).color(f.muted));
                }
            });
            self.scroll = aus.state.clamped_offset().y;
        });
    }
}

/// Öffnet die Übersicht. Läuft schon eine, passiert nichts (AC-8).
pub fn zeigen(stand: Stand, sperrdatei: &std::path::Path) -> Result<(), String> {
    use std::io::{Read, Seek, Write};
    let mut datei = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(sperrdatei).map_err(|e| e.to_string())?;
    if rustix::fs::flock(&datei, rustix::fs::FlockOperation::NonBlockingLockExclusive).is_err() {
        // Schon offen: das Kürzel schaltet um, also das offene Fenster schließen
        let mut pid = String::new();
        let _ = datei.read_to_string(&mut pid);
        if let Some(p) = pid.trim().parse::<i32>().ok().and_then(rustix::process::Pid::from_raw) {
            let _ = rustix::process::kill_process(p, rustix::process::Signal::TERM);
        }
        return Ok(());
    }
    let _ = datei.set_len(0).and_then(|_| datei.rewind()).and_then(|_| write!(datei, "{}", std::process::id()));
    let app = App { eintraege: eintraege(&stand, crate::i18n::aktuell()), suche: String::new(), scroll: 0.0 };
    let optionen = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_app_id(crate::engine::UEBERSICHT_APP_ID).with_title("tippkraft · Bausteine").with_inner_size([640.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "tippkraft",
        optionen,
        Box::new(|cc| {
            crate::popup::stil(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Baustein;

    fn eintraege_de(s: &Stand) -> Vec<Eintrag> {
        eintraege(s, Sprache::De)
    }

    fn b(kuerzel: &str, inhalt: &str, bezeichnung: Option<&str>, aktiv: bool) -> Baustein {
        Baustein {
            kuerzel: kuerzel.into(),
            inhalt: inhalt.into(),
            html: None,
            bezeichnung: bezeichnung.map(String::from),
            aktiv,
            ausloeser: Ausloeser::Leertaste,
            wortgrenze: true,
            gross_klein_beachten: false,
            schreibweise_uebernehmen: true,
        }
    }

    #[test]
    fn aktive_zuerst_dann_deaktivierte_f14_ac4() {
        let stand = Stand { sprache: None, desktop_version: None, version: 2, bausteine: vec![b("alt", "x", None, false), b("Mfg", "y", None, true), b("adr", "z", Some("Adresse"), true)], ausschluesse: vec![] };
        let e = eintraege(&stand, Sprache::De);
        assert_eq!(e.iter().map(|x| (x.kuerzel.as_str(), x.aktiv)).collect::<Vec<_>>(), [("adr", true), ("Mfg", true), ("alt", false)]);
        assert_eq!(e[0].ausloeser, "Leertaste");
    }

    #[test]
    fn nur_einzeiliger_auszug_wie_im_webtool() {
        let e = eintraege_de(&Stand { sprache: None, desktop_version: None, version: 2, bausteine: vec![b("a", "eins\n\n  zwei\ndrei", None, true), b("l", &"x".repeat(20_000), None, true)], ausschluesse: vec![] });
        assert_eq!(e[0].vorschau, "eins zwei drei");
        assert_eq!(e[1].vorschau, format!("{}…", "x".repeat(80)));
    }

    #[test]
    fn filter_ueber_kuerzel_bezeichnung_inhalt_f14_ac5() {
        let e = eintraege_de(&Stand {
            sprache: None,
            desktop_version: None,
            version: 2,
            bausteine: vec![b("mfg", "Mit freundlichen Grüßen", None, true), b("adr", "Musterstraße", Some("Adresse"), true), b("sig", "Viele GRÜSSE", None, false)],
            ausschluesse: vec![],
        });
        let k = |q: &str| filtern(&e, q).iter().map(|x| x.eintrag.kuerzel.clone()).collect::<Vec<_>>();
        assert_eq!(k("grüß"), ["mfg"]);
        assert_eq!(k("ADRESSE"), ["adr"]);
        assert_eq!(k("sig"), ["sig"]);
        assert_eq!(k("  "), ["adr", "mfg", "sig"], "leere Suche zeigt alles");
    }

    #[test]
    fn treffer_im_kuerzel_zuerst_und_vorschau_zeigt_die_fundstelle() {
        let lang = "Liebe Anna, ich würde dich gerne näher kennen lernen. Eventuell hast du Lust, am Montag einen Test mitzumachen, der nur kurz dauert.";
        let e = eintraege_de(&Stand {
            sprache: None,
            desktop_version: None,
            version: 2,
            bausteine: vec![b("brief", lang, Some("Anschreiben"), true), b("xtest", "kurz", None, true), b("ok", "passt", Some("Testantwort"), true)],
            ausschluesse: vec![],
        });
        let t = filtern(&e, "test");
        // Kürzel vor Bezeichnung vor Inhalt
        assert_eq!(t.iter().map(|x| x.eintrag.kuerzel.as_str()).collect::<Vec<_>>(), ["xtest", "ok", "brief"]);
        // bei einem Treffer im Inhalt steht die Fundstelle in der Vorschau
        assert!(t[2].vorschau.starts_with('…') && t[2].vorschau.contains("einen Test mitzumachen"), "{}", t[2].vorschau);
        // ohne Suche und bei Treffer im Kürzel: Anfang wie bisher
        assert_eq!(t[0].vorschau, "kurz");
        assert!(filtern(&e, "").iter().find(|x| x.eintrag.kuerzel == "brief").unwrap().vorschau.starts_with("Liebe Anna"));
    }
}
