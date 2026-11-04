//! Abfrage-Popup (F8): eigener Prozess `tippkraft popup`, liest {titel, inhalt, datum} von stdin,
//! schreibt {"werte": {...}} auf stdout. Exit 0 = einfügen, Exit 1 = abgebrochen.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::engine::POPUP_APP_ID;
use crate::format::{Abfrage, Aktion, Taste, abfragen, aktionen, einsetzen};

#[derive(Serialize, Deserialize)]
pub struct Anfrage {
    pub titel: String,
    pub inhalt: String,
    pub datum: String,
}

#[derive(Serialize, Deserialize)]
pub struct Antwort {
    pub werte: HashMap<String, String>,
}

/// Zustand des Formulars, ohne Fenster testbar
pub struct Formular {
    pub abfragen: Vec<Abfrage>,
    /// Text je Abfrage. Bei einer Auswahl unbenutzt.
    pub texte: Vec<String>,
    /// gewählte Option je Abfrage. Bei Textfeldern unbenutzt.
    pub wahl: Vec<usize>,
    pub aktiv: usize,
}

impl Formular {
    pub fn neu(inhalt: &str) -> Self {
        let a = abfragen(inhalt);
        Formular { texte: vec![String::new(); a.len()], wahl: vec![0; a.len()], abfragen: a, aktiv: 0 }
    }

    pub fn weiter(&mut self, rueckwaerts: bool) {
        let n = self.abfragen.len().max(1);
        self.aktiv = if rueckwaerts { (self.aktiv + n - 1) % n } else { (self.aktiv + 1) % n };
    }

    /// Pfeiltaste im aktiven Auswahlfeld
    pub fn waehlen(&mut self, schritt: i32) {
        if let Some(Some(opt)) = self.abfragen.get(self.aktiv).map(|a| a.optionen.as_ref()) {
            let n = opt.len() as i32;
            self.wahl[self.aktiv] = (self.wahl[self.aktiv] as i32 + schritt).rem_euclid(n) as usize;
        }
    }

    pub fn werte(&self) -> HashMap<String, String> {
        self.abfragen
            .iter()
            .enumerate()
            .map(|(i, a)| (a.name.clone(), a.optionen.as_ref().map_or_else(|| self.texte[i].clone(), |o| o[self.wahl[i]].clone())))
            .collect()
    }

    /// Fertiger Text, Tasten-Tokens als Symbole
    pub fn vorschau(&self, inhalt: &str, datum: &str) -> String {
        aktionen(&einsetzen(inhalt, &self.werte(), datum))
            .into_iter()
            .map(|a| match a {
                Aktion::Text(t) => t,
                Aktion::Taste(t, n) => {
                    let s = match t {
                        Taste::Tab => "⇥",
                        Taste::Enter => "⏎",
                        Taste::Space => "␣",
                        _ => "⌨",
                    };
                    s.repeat(n as usize)
                }
            })
            .collect()
    }
}

struct App {
    anfrage: Anfrage,
    f: Formular,
    ergebnis: Arc<Mutex<Option<HashMap<String, String>>>>,
    fokus_gesetzt: Option<usize>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        use egui::{Key, Modifiers};
        let (tab, shift_tab, enter, esc, links, rechts) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Tab),
                i.consume_key(Modifiers::SHIFT, Key::Tab),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::NONE, Key::ArrowLeft) || i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowRight) || i.consume_key(Modifiers::NONE, Key::ArrowDown),
            )
        });
        let ist_auswahl = self.f.abfragen.get(self.f.aktiv).is_some_and(|a| a.optionen.is_some());
        if tab || shift_tab {
            self.f.weiter(shift_tab);
        }
        if ist_auswahl && (links || rechts) {
            self.f.waehlen(if links { -1 } else { 1 });
        }
        if esc {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if enter {
            *self.ergebnis.lock().unwrap() = Some(self.f.werte());
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(format!("tippkraft · {}", self.anfrage.titel));
            ui.add_space(8.0);
            for i in 0..self.f.abfragen.len() {
                let aktiv = i == self.f.aktiv;
                let rahmen = egui::Frame::group(ui.style()).stroke(if aktiv { egui::Stroke::new(2.0, egui::Color32::from_rgb(0xd9, 0x48, 0x0f)) } else { ui.visuals().widgets.noninteractive.bg_stroke });
                rahmen.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(&self.f.abfragen[i].name);
                    match self.f.abfragen[i].optionen.clone() {
                        None => {
                            let r = ui.add(egui::TextEdit::singleline(&mut self.f.texte[i]).desired_width(f32::INFINITY));
                            if aktiv && self.fokus_gesetzt != Some(i) {
                                r.request_focus();
                                self.fokus_gesetzt = Some(i);
                            }
                            if r.clicked() {
                                self.f.aktiv = i;
                            }
                        }
                        Some(optionen) => {
                            if aktiv && self.fokus_gesetzt != Some(i) {
                                ui.ctx().memory_mut(|m| m.surrender_focus(egui::Id::NULL));
                                self.fokus_gesetzt = Some(i);
                            }
                            ui.horizontal_wrapped(|ui| {
                                for (j, o) in optionen.iter().enumerate() {
                                    let text = if o.is_empty() { "(nichts)" } else { o.as_str() };
                                    if ui.selectable_label(self.f.wahl[i] == j, text).clicked() {
                                        self.f.wahl[i] = j;
                                        self.f.aktiv = i;
                                    }
                                }
                            });
                        }
                    }
                });
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Vorschau").weak());
            ui.label(egui::RichText::new(self.f.vorschau(&self.anfrage.inhalt, &self.anfrage.datum)).monospace());
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Enter einfügen · Tab weiter · ←/→ Auswahl · Esc abbrechen").weak().small());
        });
    }
}

/// Zeigt das Popup und liefert die Werte, `None` bei Abbruch
pub fn zeigen(anfrage: Anfrage) -> Result<Option<HashMap<String, String>>, String> {
    let f = Formular::neu(&anfrage.inhalt);
    let hoehe = 150.0 + 70.0 * f.abfragen.len() as f32;
    let ergebnis = Arc::new(Mutex::new(None));
    let app = App { anfrage, f, ergebnis: ergebnis.clone(), fokus_gesetzt: None };
    let optionen = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_app_id(POPUP_APP_ID).with_title("tippkraft").with_inner_size([480.0, hoehe]),
        ..Default::default()
    };
    eframe::run_native("tippkraft", optionen, Box::new(|_| Ok(Box::new(app)))).map_err(|e| e.to_string())?;
    let r = ergebnis.lock().unwrap().take();
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formular_tab_auswahl_werte_vorschau_ac11_ac12() {
        let inhalt = "{{?Anrede:Herr|Frau|}} {{?Name}}, {{=datum}}[[Tab]] {{?Name}}";
        let mut f = Formular::neu(inhalt);
        assert_eq!(f.abfragen.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Anrede", "Name"]);
        assert_eq!(f.werte()["Anrede"], "Herr", "erste Option vorgewählt");
        f.waehlen(1);
        f.waehlen(1);
        assert_eq!(f.werte()["Anrede"], "", "leere Option");
        f.waehlen(-1);
        f.weiter(false);
        f.waehlen(1); // im Textfeld wirkungslos
        f.texte[1] = "Muster".into();
        assert_eq!(f.werte()["Anrede"], "Frau");
        assert_eq!(f.vorschau(inhalt, "04.11.2026"), "Frau Muster, 04.11.2026⇥ Muster");
        f.weiter(false);
        assert_eq!(f.aktiv, 0, "Tab springt vom letzten zum ersten Feld");
        f.weiter(true);
        assert_eq!(f.aktiv, 1);
    }
}
