//! Baustein-Übersicht (F14): eigener Prozess `tippkraft uebersicht`, liest stand.json, nur Anzeige und Filter.

use crate::format::{Ausloeser, Stand};

#[derive(Debug, Clone, PartialEq)]
pub struct Eintrag {
    pub kuerzel: String,
    pub ausloeser: &'static str,
    pub bezeichnung: Option<String>,
    pub vorschau: String,
    pub aktiv: bool,
    /// alles Durchsuchbare, kleingeschrieben
    suchtext: String,
}

fn ausloeser_name(a: Ausloeser) -> &'static str {
    match a {
        Ausloeser::Sofort => "Sofort",
        Ausloeser::Leertaste => "Leertaste",
        Ausloeser::Enter => "Enter",
        Ausloeser::Tab => "Tab",
    }
}

fn vorschau(inhalt: &str) -> String {
    let kurz = |z: &str| if z.chars().count() > 120 { format!("{}…", z.chars().take(120).collect::<String>()) } else { z.to_string() };
    let zeilen: Vec<&str> = inhalt.lines().collect();
    let mut v = zeilen.iter().take(2).map(|z| kurz(z)).collect::<Vec<_>>().join("\n");
    if zeilen.len() > 2 {
        v.push_str(" …");
    }
    v
}

/// Aktive zuerst, dann deaktivierte, jeweils alphabetisch
pub fn eintraege(stand: &Stand) -> Vec<Eintrag> {
    let mut e: Vec<Eintrag> = stand
        .bausteine
        .iter()
        .map(|b| Eintrag {
            kuerzel: b.kuerzel.clone(),
            ausloeser: ausloeser_name(b.ausloeser),
            bezeichnung: b.bezeichnung.clone(),
            vorschau: vorschau(&b.inhalt),
            aktiv: b.aktiv,
            suchtext: format!("{}\n{}\n{}", b.kuerzel, b.bezeichnung.as_deref().unwrap_or_default(), b.inhalt).to_lowercase(),
        })
        .collect();
    e.sort_by_key(|x| (!x.aktiv, x.kuerzel.to_lowercase()));
    e
}

pub fn filtern<'a>(e: &'a [Eintrag], suche: &str) -> Vec<&'a Eintrag> {
    let q = suche.trim().to_lowercase();
    e.iter().filter(|x| q.is_empty() || x.suchtext.contains(&q)).collect()
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
                ui.label(RichText::new("Bausteine").size(18.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let n = self.eintraege.len();
                    let text = if self.suche.trim().is_empty() { format!("{n} Bausteine") } else { format!("{} von {n}", treffer.len()) };
                    ui.label(RichText::new(text).color(f.muted).size(13.0));
                });
            });
            ui.add_space(8.0);
            // Suchfeld behält immer den Fokus: Es ist das einzige bedienbare Element
            let r = ui.add(egui::TextEdit::singleline(&mut self.suche).hint_text("Suchen …").desired_width(f32::INFINITY).margin(egui::vec2(10.0, 8.0)));
            if !r.has_focus() {
                r.request_focus();
            }
            ui.add_space(10.0);

            if self.eintraege.is_empty() {
                ui.label(RichText::new("Noch keine Bausteine. Verbinde tippkraft über „tippkraft verbinden“.").color(f.muted));
                return;
            }
            let aus = egui::ScrollArea::vertical().auto_shrink(false).vertical_scroll_offset(self.scroll).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for e in &treffer {
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
                                let tag = if e.aktiv { e.ausloeser.to_string() } else { "löst nicht aus".to_string() };
                                ui.label(RichText::new(tag).color(f.muted).size(12.5));
                            });
                        });
                        ui.label(RichText::new(&e.vorschau).monospace().color(f.muted).size(13.0));
                    });
                    let y = ui.cursor().top();
                    ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, f.border));
                }
                if treffer.is_empty() {
                    ui.label(RichText::new("Keine Treffer.").color(f.muted));
                }
            });
            self.scroll = aus.state.clamped_offset().y;
        });
    }
}

/// Öffnet die Übersicht. Läuft schon eine, passiert nichts (AC-8).
pub fn zeigen(stand: Stand, sperrdatei: &std::path::Path) -> Result<(), String> {
    let datei = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(sperrdatei).map_err(|e| e.to_string())?;
    if rustix::fs::flock(&datei, rustix::fs::FlockOperation::NonBlockingLockExclusive).is_err() {
        return Ok(());
    }
    let app = App { eintraege: eintraege(&stand), suche: String::new(), scroll: 0.0 };
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

    fn b(kuerzel: &str, inhalt: &str, bezeichnung: Option<&str>, aktiv: bool) -> Baustein {
        Baustein {
            kuerzel: kuerzel.into(),
            inhalt: inhalt.into(),
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
        let stand = Stand { version: 2, bausteine: vec![b("alt", "x", None, false), b("Mfg", "y", None, true), b("adr", "z", Some("Adresse"), true)], ausschluesse: vec![] };
        let e = eintraege(&stand);
        assert_eq!(e.iter().map(|x| (x.kuerzel.as_str(), x.aktiv)).collect::<Vec<_>>(), [("adr", true), ("Mfg", true), ("alt", false)]);
        assert_eq!(e[0].ausloeser, "Leertaste");
    }

    #[test]
    fn vorschau_hoechstens_zwei_zeilen() {
        let e = eintraege(&Stand { version: 2, bausteine: vec![b("a", "eins\nzwei\ndrei", None, true), b("l", &"x".repeat(200), None, true)], ausschluesse: vec![] });
        assert_eq!(e[0].vorschau, "eins\nzwei …");
        assert_eq!(e[1].vorschau.chars().count(), 121);
        assert!(e[1].vorschau.ends_with('…'));
    }

    #[test]
    fn filter_ueber_kuerzel_bezeichnung_inhalt_f14_ac5() {
        let e = eintraege(&Stand {
            version: 2,
            bausteine: vec![b("mfg", "Mit freundlichen Grüßen", None, true), b("adr", "Musterstraße", Some("Adresse"), true), b("sig", "Viele GRÜSSE", None, false)],
            ausschluesse: vec![],
        });
        let k = |q: &str| filtern(&e, q).iter().map(|x| x.kuerzel.clone()).collect::<Vec<_>>();
        assert_eq!(k("grüß"), ["mfg"]);
        assert_eq!(k("ADRESSE"), ["adr"]);
        assert_eq!(k("sig"), ["sig"]);
        assert_eq!(k("  "), ["adr", "mfg", "sig"], "leere Suche zeigt alles");
    }
}
