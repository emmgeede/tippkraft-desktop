//! Abfrage-Popup (F8): eigener Prozess `tippkraft popup`, liest {titel, inhalt, datum} von stdin,
//! schreibt {"werte": {...}} auf stdout. Exit 0 = einfügen, Exit 1 = abgebrochen.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::engine::POPUP_APP_ID;
use crate::i18n::{T, tr};
use crate::format::{Abfrage, Aktion, Automatisch, CURSOR, Taste, abfragen, aktionen, auswerten, einsetzen, gueltig};

#[derive(Serialize, Deserialize)]
pub struct Anfrage {
    pub titel: String,
    pub inhalt: String,
    /// Datum, Zeitpunkt, Sprache, Zwischenablage, Variablen (F42), Zufall (F42). Gleiche Felder wie früher, deshalb flach.
    #[serde(flatten)]
    pub auto: Automatisch,
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
    /// F42 AC-4: mit vorhandenen Variablen vorbelegt
    pub fn neu(inhalt: &str, auto: &Automatisch) -> Self {
        let a = abfragen(inhalt);
        let alt = |x: &Abfrage| auto.variablen.get(&x.name);
        let texte = a.iter().map(|x| alt(x).cloned().unwrap_or_default()).collect();
        let wahl = a.iter().map(|x| x.optionen.as_ref().and_then(|o| o.iter().position(|o| Some(o) == alt(x))).unwrap_or(0)).collect();
        Formular { texte, wahl, abfragen: a, aktiv: 0 }
    }

    /// F42 AC-7: Indizes der Felder in gewählten Zweigen
    pub fn sichtbar(&self, inhalt: &str, auto: &Automatisch) -> Vec<usize> {
        let namen = auswerten(inhalt, &self.werte(), auto).sichtbar;
        (0..self.abfragen.len()).filter(|&i| namen.contains(&self.abfragen[i].name)).collect()
    }

    pub fn weiter(&mut self, rueckwaerts: bool, sichtbar: &[usize]) {
        let n = sichtbar.len();
        if n == 0 {
            return;
        }
        let pos = sichtbar.iter().position(|&i| i == self.aktiv).unwrap_or(0);
        self.aktiv = sichtbar[if rueckwaerts { (pos + n - 1) % n } else { (pos + 1) % n }];
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

    /// F42 AC-14 bis AC-16: erstes sichtbares Feld mit Pflicht- oder Prüffehler
    pub fn fehler(&self, sichtbar: &[usize]) -> Option<(usize, T)> {
        let w = self.werte();
        sichtbar.iter().copied().find_map(|i| {
            let a = &self.abfragen[i];
            let wert = &w[&a.name];
            (!gueltig(a, wert)).then(|| (i, if wert.trim().is_empty() { T::PopupPflicht } else if a.pruefung.as_deref() == Some("email") { T::PopupEmail } else { T::PopupZahl }))
        })
    }

    /// Fertiger Text, Tasten-Tokens als Symbole, Cursorposition als ▏
    pub fn vorschau(&self, inhalt: &str, auto: &Automatisch) -> String {
        aktionen(&einsetzen(inhalt, &self.werte(), auto))
            .into_iter()
            .map(|a| match a {
                Aktion::Text(t) | Aktion::Html { text: t, .. } => t.replace(CURSOR, "▏"),
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
    /// F42: Feld mit Pflicht- oder Prüffehler nach Enter
    fehler: Option<(usize, T)>,
}

/// Farben wie im Webtool (web/src/layouts/Layout.astro), hell und dunkel
pub(crate) struct Farben {
    pub bg: egui::Color32,
    pub fg: egui::Color32,
    pub muted: egui::Color32,
    pub border: egui::Color32,
    pub accent: egui::Color32,
    /// Füllung für Knöpfe mit weißer Schrift (accent-600, Kontrast über 4,5:1)
    pub knopf: egui::Color32,
    pub flaeche: egui::Color32,
}

// Palette wie im Webtool (F16): Grautöne stone, Akzentskala aus #d9480f
pub(crate) fn farben(dunkel: bool) -> Farben {
    use egui::Color32 as C;
    const KNOPF: C = C::from_rgb(0xc0, 0x3d, 0x0c);
    if dunkel {
        Farben { bg: C::from_rgb(0x1c, 0x19, 0x17), fg: C::from_rgb(0xf5, 0xf5, 0xf4), muted: C::from_rgb(0xa8, 0xa2, 0x9e), border: C::from_rgb(0x44, 0x40, 0x3c), accent: C::from_rgb(0xff, 0x7a, 0x3d), knopf: KNOPF, flaeche: C::from_rgb(0x29, 0x25, 0x24) }
    } else {
        Farben { bg: C::WHITE, fg: C::from_rgb(0x1c, 0x19, 0x17), muted: C::from_rgb(0x78, 0x71, 0x6c), border: C::from_rgb(0xe7, 0xe5, 0xe4), accent: KNOPF, knopf: KNOPF, flaeche: C::from_rgb(0xf5, 0xf5, 0xf4) }
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
        let sichtbar = self.f.sichtbar(&self.anfrage.inhalt, &self.anfrage.auto);
        let ist_auswahl = self.f.abfragen.get(self.f.aktiv).is_some_and(|a| a.optionen.is_some());
        if tab || shift_tab {
            self.f.weiter(shift_tab, &sichtbar);
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

            for &i in &sichtbar {
                let aktiv = i == self.f.aktiv;
                let a = &self.f.abfragen[i];
                let name = if a.pflicht { format!("{} *", a.name) } else { a.name.clone() };
                ui.label(RichText::new(name).color(if aktiv { f.accent } else { f.fg }));
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
                                    .fill(if gewaehlt { f.knopf } else { f.bg })
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
                if let Some((j, t)) = self.fehler
                    && j == i
                {
                    ui.label(RichText::new(tr(t)).color(egui::Color32::from_rgb(0xdc, 0x26, 0x26)).size(13.0));
                }
                ui.add_space(6.0);
            }

            ui.add_space(4.0);
            ui.label(RichText::new(tr(T::PopupVorschau)).color(f.muted).size(13.0));
            egui::Frame::new().fill(f.flaeche).corner_radius(6).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(self.f.vorschau(&self.anfrage.inhalt, &self.anfrage.auto)).monospace());
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let primaer = egui::Button::new(RichText::new(tr(T::PopupEinfuegen)).color(egui::Color32::WHITE)).fill(f.knopf).stroke(egui::Stroke::NONE);
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

        if einfuegen && let Some(fehler) = self.f.fehler(&sichtbar) {
            // F42: Popup bleibt offen, Fokus aufs Feld mit dem Fehler
            self.fehler = Some(fehler);
            self.f.aktiv = fehler.0;
            self.fokus_gesetzt = None;
        } else if einfuegen {
            *self.ergebnis.lock().unwrap() = Some(self.f.werte());
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else if abbrechen {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Zeigt das Popup und liefert die Werte, `None` bei Abbruch
pub fn zeigen(anfrage: Anfrage) -> Result<Option<HashMap<String, String>>, String> {
    let f = Formular::neu(&anfrage.inhalt, &anfrage.auto);
    let hoehe = 236.0 + 72.0 * f.abfragen.len() as f32 + 18.0 * anfrage.inhalt.lines().count().saturating_sub(1) as f32;
    let ergebnis = Arc::new(Mutex::new(None));
    let app = App { anfrage, f, ergebnis: ergebnis.clone(), fokus_gesetzt: None, fehler: None };
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
        let mut f = Formular::neu(inhalt, &Automatisch::default());
        let alle = [0, 1];
        assert_eq!(f.abfragen.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Anrede", "Name"]);
        assert_eq!(f.werte()["Anrede"], "Herr", "erste Option vorgewählt");
        f.waehlen(1);
        f.waehlen(1);
        assert_eq!(f.werte()["Anrede"], "", "leere Option");
        f.waehlen(-1);
        f.weiter(false, &alle);
        f.waehlen(1); // im Textfeld wirkungslos
        f.texte[1] = "Muster".into();
        assert_eq!(f.werte()["Anrede"], "Frau");
        assert_eq!(f.vorschau(inhalt, &Automatisch { datum: "04.11.2026".into(), zwischenablage: None, ..Default::default() }), "Frau Muster, 04.11.2026⇥ Muster");
        assert_eq!(f.vorschau("a{{=cursor}}b {{=clipboard}}", &Automatisch { datum: String::new(), zwischenablage: Some("Z".into()), ..Default::default() }), "a▏b Z");
        f.weiter(false, &alle);
        assert_eq!(f.aktiv, 0, "Tab springt vom letzten zum ersten Feld");
        f.weiter(true, &alle);
        assert_eq!(f.aktiv, 1);
    }

    #[test]
    fn felder_ausblenden_vorbelegen_pruefen_f42_ac4_ac7_ac14_ac16() {
        let inhalt = "{{?Firma}}{{#if Firma}} z. Hd. {{?Ansprechpartner*}}{{#end}} {{?Betrag:number}} {{?Anrede:Herr|Frau}}";
        let mut auto = Automatisch::default();
        auto.variablen.insert("Anrede".into(), "Frau".into());
        auto.variablen.insert("Betrag".into(), "12".into());
        let mut f = Formular::neu(inhalt, &auto);
        assert_eq!((f.texte[2].as_str(), f.werte()["Anrede"].as_str()), ("12", "Frau"), "vorbelegt");
        assert_eq!(f.sichtbar(inhalt, &auto), [0, 2, 3], "Ansprechpartner ausgeblendet");
        assert_eq!(f.fehler(&f.sichtbar(inhalt, &auto)), None, "ausgeblendetes Pflichtfeld zählt nicht");
        f.weiter(false, &f.sichtbar(inhalt, &auto));
        assert_eq!(f.aktiv, 2, "Tab überspringt ausgeblendete Felder");
        f.texte[0] = "ACME".into();
        let s = f.sichtbar(inhalt, &auto);
        assert_eq!(s, [0, 1, 2, 3]);
        assert_eq!(f.fehler(&s), Some((1, T::PopupPflicht)));
        f.texte[1] = "Udo".into();
        f.texte[2] = "12a".into();
        assert_eq!(f.fehler(&s), Some((2, T::PopupZahl)));
        f.texte[2] = "12,5".into();
        assert_eq!(f.fehler(&s), None);
    }
}
