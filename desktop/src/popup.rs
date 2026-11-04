//! Abfrage-Popup (F8): eigener Prozess `tippkraft popup`, liest {titel, inhalt, datum} von stdin,
//! schreibt {"werte": {...}} auf stdout. Exit 0 = einfügen, Exit 1 = abgebrochen.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::engine::POPUP_APP_ID;
use crate::i18n::{T, tr};
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

/// Farben wie im Webtool (web/src/layouts/Layout.astro), hell und dunkel
pub(crate) struct Farben {
    pub bg: egui::Color32,
    pub fg: egui::Color32,
    pub muted: egui::Color32,
    pub border: egui::Color32,
    pub accent: egui::Color32,
    pub flaeche: egui::Color32,
}

pub(crate) fn farben(dunkel: bool) -> Farben {
    use egui::Color32 as C;
    if dunkel {
        Farben { bg: C::from_rgb(0x16, 0x16, 0x16), fg: C::from_rgb(0xee, 0xee, 0xee), muted: C::from_rgb(0x99, 0x99, 0x99), border: C::from_rgb(0x33, 0x33, 0x33), accent: C::from_rgb(0xff, 0x7a, 0x3d), flaeche: C::from_rgb(0x22, 0x22, 0x22) }
    } else {
        Farben { bg: C::WHITE, fg: C::from_rgb(0x1a, 0x1a, 0x1a), muted: C::from_rgb(0x66, 0x66, 0x66), border: C::from_rgb(0xdd, 0xdd, 0xdd), accent: C::from_rgb(0xd9, 0x48, 0x0f), flaeche: C::from_rgb(0xf6, 0xf4, 0xf2) }
    }
}

fn schrift_laden(fonts: &mut egui::FontDefinitions, name: &str, familie: egui::FontFamily, muster: &str) {
    // fontconfig kennt die Systemschriften, egui nicht
    let Ok(out) = std::process::Command::new("fc-match").args(["-f", "%{file}", muster]).output() else { return };
    let Ok(daten) = std::fs::read(String::from_utf8_lossy(&out.stdout).trim()) else { return };
    fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(daten)));
    fonts.families.entry(familie).or_default().insert(0, name.into());
}

/// Stil wie im Webtool: Noto Sans, Akzentfarbe, 6 px Radius, viel Luft, hell/dunkel nach System
pub(crate) fn stil(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    schrift_laden(&mut fonts, "noto", egui::FontFamily::Proportional, "Noto Sans");
    schrift_laden(&mut fonts, "noto-mono", egui::FontFamily::Monospace, "Noto Sans Mono");
    ctx.set_fonts(fonts);

    for (theme, dunkel) in [(egui::Theme::Light, false), (egui::Theme::Dark, true)] {
        let f = farben(dunkel);
        let mut v = if dunkel { egui::Visuals::dark() } else { egui::Visuals::light() };
        let radius = egui::CornerRadius::same(6);
        v.panel_fill = f.bg;
        v.window_fill = f.bg;
        v.extreme_bg_color = f.bg; // Hintergrund der Textfelder
        v.override_text_color = Some(f.fg);
        v.selection.bg_fill = f.accent.gamma_multiply(0.35);
        v.selection.stroke = egui::Stroke::new(2.0, f.accent); // Fokusrahmen
        v.text_cursor.stroke = egui::Stroke::new(2.0, f.accent);
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = radius;
            w.bg_stroke = egui::Stroke::new(1.0, f.border);
            w.fg_stroke = egui::Stroke::new(1.0, f.fg);
        }
        v.widgets.inactive.weak_bg_fill = f.bg;
        v.widgets.inactive.bg_fill = f.bg;
        v.widgets.hovered.weak_bg_fill = f.flaeche;
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, f.muted);
        v.widgets.active.weak_bg_fill = f.flaeche;
        ctx.set_visuals_of(theme, v);
    }
    ctx.all_styles_mut(|s| {
        use egui::{FontFamily::*, FontId, TextStyle};
        s.text_styles = [
            (TextStyle::Heading, FontId::new(20.0, Proportional)),
            (TextStyle::Body, FontId::new(15.0, Proportional)),
            (TextStyle::Button, FontId::new(15.0, Proportional)),
            (TextStyle::Small, FontId::new(12.5, Proportional)),
            (TextStyle::Monospace, FontId::new(14.0, Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(14.0, 7.0);
        s.spacing.interact_size.y = 34.0;
    });
    ctx.set_theme(egui::ThemePreference::System);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        use egui::{Key, Modifiers, RichText};
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
        let mut abbrechen = esc;
        let mut einfuegen = enter;
        let f = farben(ui.visuals().dark_mode);

        let rand = egui::Frame::new().fill(f.bg).inner_margin(egui::Margin::same(20));
        egui::CentralPanel::default().frame(rand).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr(T::PopupBaustein)).color(f.muted).size(13.0));
                ui.label(RichText::new(&self.anfrage.titel).monospace().color(f.accent).size(15.0));
            });
            ui.add_space(10.0);

            for i in 0..self.f.abfragen.len() {
                let aktiv = i == self.f.aktiv;
                ui.label(RichText::new(&self.f.abfragen[i].name).color(if aktiv { f.accent } else { f.fg }));
                match self.f.abfragen[i].optionen.clone() {
                    None => {
                        let r = ui.add(egui::TextEdit::singleline(&mut self.f.texte[i]).desired_width(f32::INFINITY).margin(egui::vec2(10.0, 8.0)));
                        if aktiv && self.fokus_gesetzt != Some(i) {
                            r.request_focus();
                            self.fokus_gesetzt = Some(i);
                        }
                        if r.gained_focus() {
                            self.f.aktiv = i;
                            self.fokus_gesetzt = Some(i);
                        }
                    }
                    Some(optionen) => {
                        if aktiv && self.fokus_gesetzt != Some(i) {
                            ui.ctx().memory_mut(|m| m.surrender_focus(egui::Id::NULL));
                            self.fokus_gesetzt = Some(i);
                        }
                        ui.horizontal_wrapped(|ui| {
                            for (j, o) in optionen.iter().enumerate() {
                                let gewaehlt = self.f.wahl[i] == j;
                                let text = if o.is_empty() { tr(T::PopupNichts) } else { o.as_str() };
                                // Pill-Buttons wie die Gruppen im Webtool: gewählt = Akzentfläche
                                let knopf = egui::Button::new(RichText::new(text).color(if gewaehlt { egui::Color32::WHITE } else { f.fg }))
                                    .fill(if gewaehlt { f.accent } else { f.bg })
                                    .stroke(egui::Stroke::new(if aktiv && gewaehlt { 2.0 } else { 1.0 }, if gewaehlt { f.accent } else { f.border }))
                                    .corner_radius(egui::CornerRadius::same(255));
                                if ui.add(knopf).clicked() {
                                    self.f.wahl[i] = j;
                                    self.f.aktiv = i;
                                }
                            }
                        });
                    }
                }
                ui.add_space(6.0);
            }

            ui.add_space(4.0);
            ui.label(RichText::new(tr(T::PopupVorschau)).color(f.muted).size(13.0));
            egui::Frame::new().fill(f.flaeche).corner_radius(6).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(self.f.vorschau(&self.anfrage.inhalt, &self.anfrage.datum)).monospace());
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let primaer = egui::Button::new(RichText::new(tr(T::PopupEinfuegen)).color(egui::Color32::WHITE)).fill(f.accent).stroke(egui::Stroke::NONE);
                if ui.add(primaer).clicked() {
                    einfuegen = true;
                }
                if ui.add(egui::Button::new(tr(T::PopupAbbrechen)).fill(f.bg)).clicked() {
                    abbrechen = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(tr(T::PopupTasten)).color(f.muted).size(12.5));
                });
            });
        });

        if einfuegen {
            *self.ergebnis.lock().unwrap() = Some(self.f.werte());
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else if abbrechen {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Zeigt das Popup und liefert die Werte, `None` bei Abbruch
pub fn zeigen(anfrage: Anfrage) -> Result<Option<HashMap<String, String>>, String> {
    let f = Formular::neu(&anfrage.inhalt);
    let hoehe = 236.0 + 72.0 * f.abfragen.len() as f32 + 18.0 * anfrage.inhalt.lines().count().saturating_sub(1) as f32;
    let ergebnis = Arc::new(Mutex::new(None));
    let app = App { anfrage, f, ergebnis: ergebnis.clone(), fokus_gesetzt: None };
    let optionen = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_app_id(POPUP_APP_ID).with_title("tippkraft").with_inner_size([520.0, hoehe]),
        ..Default::default()
    };
    eframe::run_native(
        "tippkraft",
        optionen,
        Box::new(|cc| {
            stil(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())?;
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
