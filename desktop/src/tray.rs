//! Tray-Icon (F13): StatusNotifierItem über ksni, Icon wird zur Laufzeit gezeichnet.

use crate::sync::Verbindung;

#[derive(Debug, Clone, PartialEq)]
pub struct TrayZustand {
    pub pausiert: bool,
    pub verbindung: Verbindung,
    pub anzahl: usize,
    pub server: String,
}

fn verbindung_kurz(v: Verbindung) -> &'static str {
    match v {
        Verbindung::Verbunden => "verbunden",
        Verbindung::Offline => "offline",
        Verbindung::TokenUngueltig => "Token ungültig",
        Verbindung::NichtAngemeldet => "nicht verbunden",
        Verbindung::ServerNeuer => "Server neuer, bitte aktualisieren",
    }
}

pub fn statuszeile(z: &TrayZustand) -> String {
    format!("{} · {} · {} Bausteine", if z.pausiert { "Pausiert" } else { "Aktiv" }, verbindung_kurz(z.verbindung), z.anzahl)
}

fn warnung(z: &TrayZustand) -> bool {
    matches!(z.verbindung, Verbindung::Offline | Verbindung::TokenUngueltig | Verbindung::ServerNeuer)
}

// Logo (docs/brand/logo.svg, viewBox 32): Rahmen mit runden Ecken, Dreieck als gedrückte Taste
fn im_rund_rechteck(x: f32, y: f32, a: f32, b: f32, r: f32) -> bool {
    if x < a || y < a || x > b || y > b {
        return false;
    }
    let dx = (a + r - x).max(x - (b - r)).max(0.0);
    let dy = (a + r - y).max(y - (b - r)).max(0.0);
    dx * dx + dy * dy <= r * r
}

fn im_dreieck(x: f32, y: f32) -> bool {
    // (10,12), (22,12), (16,21)
    let kante = |ax: f32, ay: f32, bx: f32, by: f32| (bx - ax) * (y - ay) - (by - ay) * (x - ax);
    let (d1, d2, d3) = (kante(10.0, 12.0, 22.0, 12.0), kante(22.0, 12.0, 16.0, 21.0), kante(16.0, 21.0, 10.0, 12.0));
    (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
}

/// Icon als ARGB32, mit 4×4-Supersampling für glatte Kanten
pub fn icon(groesse: i32, z: &TrayZustand) -> ksni::Icon {
    let logo: [u8; 3] = if z.pausiert { [0x8a, 0x8a, 0x8a] } else { [0xd9, 0x48, 0x0f] };
    let punkt = warnung(z);
    let skala = 32.0 / groesse as f32;
    let mut data = Vec::with_capacity((groesse * groesse * 4) as usize);
    for py in 0..groesse {
        for px in 0..groesse {
            let (mut deckung, mut r, mut g, mut b) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = (px as f32 + (sx as f32 + 0.5) / 4.0) * skala;
                    let y = (py as f32 + (sy as f32 + 0.5) / 4.0) * skala;
                    let abstand = ((x - 26.0).powi(2) + (y - 26.0).powi(2)).sqrt();
                    let farbe = if punkt && abstand <= 3.5 {
                        Some([0xf5, 0xb3, 0x01])
                    } else if punkt && abstand <= 5.0 {
                        Some([0x1a, 0x1a, 0x1a]) // dunkler Rand, damit der Punkt auf dem Logo sichtbar ist
                    } else if (im_rund_rechteck(x, y, 3.0, 29.0, 6.0) && !im_rund_rechteck(x, y, 6.5, 25.5, 3.5)) || im_dreieck(x, y) {
                        Some(logo)
                    } else {
                        None
                    };
                    if let Some(c) = farbe {
                        deckung += 1.0;
                        r += c[0] as f32;
                        g += c[1] as f32;
                        b += c[2] as f32;
                    }
                }
            }
            if deckung == 0.0 {
                data.extend([0, 0, 0, 0]);
            } else {
                data.extend([(deckung / 16.0 * 255.0).round() as u8, (r / deckung).round() as u8, (g / deckung).round() as u8, (b / deckung).round() as u8]);
            }
        }
    }
    ksni::Icon { width: groesse, height: groesse, data }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAktion {
    Pause,
    Uebersicht,
    Webtool,
    Verbinden,
    Beenden,
}

pub struct Tray {
    pub zustand: TrayZustand,
    pub senden: Box<dyn Fn(TrayAktion) + Send>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "tippkraft".into()
    }
    fn title(&self) -> String {
        "tippkraft".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![icon(32, &self.zustand), icon(64, &self.zustand)]
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "tippkraft".into(), description: statuszeile(&self.zustand), ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        (self.senden)(TrayAktion::Pause);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        let eintrag = |label: &str, aktion: TrayAktion| -> ksni::MenuItem<Self> {
            StandardItem { label: label.into(), activate: Box::new(move |t: &mut Self| (t.senden)(aktion)), ..Default::default() }.into()
        };
        vec![
            StandardItem { label: statuszeile(&self.zustand), enabled: false, ..Default::default() }.into(),
            ksni::MenuItem::Separator,
            eintrag(if self.zustand.pausiert { "Fortsetzen" } else { "Pausieren" }, TrayAktion::Pause),
            eintrag("Bausteine anzeigen", TrayAktion::Uebersicht),
            eintrag("Webtool öffnen", TrayAktion::Webtool),
            eintrag("Neu verbinden …", TrayAktion::Verbinden),
            ksni::MenuItem::Separator,
            eintrag("Beenden", TrayAktion::Beenden),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(pausiert: bool, verbindung: Verbindung) -> TrayZustand {
        TrayZustand { pausiert, verbindung, anzahl: 3, server: "http://localhost:4321".into() }
    }

    /// ARGB eines Pixels
    fn px(i: &ksni::Icon, x: i32, y: i32) -> [u8; 4] {
        let o = ((y * i.width + x) * 4) as usize;
        [i.data[o], i.data[o + 1], i.data[o + 2], i.data[o + 3]]
    }

    #[test]
    fn statuszeile_ac3() {
        assert_eq!(statuszeile(&z(false, Verbindung::Verbunden)), "Aktiv · verbunden · 3 Bausteine");
        assert_eq!(statuszeile(&z(true, Verbindung::Offline)), "Pausiert · offline · 3 Bausteine");
        assert_eq!(statuszeile(&z(false, Verbindung::NichtAngemeldet)), "Aktiv · nicht verbunden · 3 Bausteine");
    }

    #[test]
    fn icon_farbe_und_warnpunkt_ac2() {
        let aktiv = icon(32, &z(false, Verbindung::Verbunden));
        assert_eq!((aktiv.width, aktiv.height, aktiv.data.len()), (32, 32, 32 * 32 * 4));
        assert_eq!(px(&aktiv, 0, 0)[0], 0, "Ecke transparent");
        assert_eq!(px(&aktiv, 4, 16), [255, 0xd9, 0x48, 0x0f], "Rahmen in Akzentfarbe");
        assert_eq!(px(&aktiv, 16, 9)[0], 0, "Innenraum über dem Dreieck transparent");
        assert_eq!(px(&aktiv, 16, 15), [255, 0xd9, 0x48, 0x0f], "Dreieck");
        assert_eq!(px(&aktiv, 26, 26)[1..], [0xd9, 0x48, 0x0f], "ohne Warnung kein Punkt");

        let pause = icon(32, &z(true, Verbindung::Verbunden));
        assert_eq!(px(&pause, 4, 16), [255, 0x8a, 0x8a, 0x8a], "pausiert grau");

        let offline = icon(64, &z(false, Verbindung::Offline));
        assert_eq!(px(&offline, 52, 52), [255, 0xf5, 0xb3, 0x01], "gelber Warnpunkt");
    }
}
