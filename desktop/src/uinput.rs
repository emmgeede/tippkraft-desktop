//! F11: Ausgabe auf GNOME und KDE. Dort fehlt das virtual-keyboard-Protokoll, also tippt tippkraft
//! über die uinput-Tastatur echte Tasten passend zum Layout der Sitzung. Zeichen, die das Layout
//! nicht hergibt (Fremdzeichen), gehen auf KDE über die Zwischenablage, auf GNOME über Strg+Umschalt+U.

use crate::format::{Aktion, Taste};
use xkbcommon::xkb;

/// Wie Fremdzeichen ankommen
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fremd {
    /// KDE: ganzen Textabschnitt per Zwischenablage einfügen
    Einfuegen,
    /// GNOME: jedes Fremdzeichen per Strg+Umschalt+U und Hex-Code
    Unicode,
}

/// Ein Schritt der Ausgabe. `code` ist der evdev-Code der Taste.
#[derive(Debug, Clone, PartialEq)]
pub enum Schritt {
    Taste { code: u16, umschalt: bool, altgr: bool },
    Einfuegen(String),
    Unicode(char),
}

pub const BACKSPACE: u16 = 14;
pub const TAB: u16 = 15;
pub const ENTER: u16 = 28;
pub const LEERTASTE: u16 = 57;
pub const LINKS_UMSCHALT: u16 = 42;
pub const LINKS_STRG: u16 = 29;
pub const RECHTS_ALT: u16 = 100;
pub const EINFG: u16 = 110;
/// ponytail: feste Pause für IBus (gemessen in der GNOME-Test-VM), kein Rückkanal vorhanden
const IBUS_PAUSE: std::time::Duration = std::time::Duration::from_millis(50);

fn taste(code: u16) -> Schritt {
    Schritt::Taste { code, umschalt: false, altgr: false }
}

fn token_code(t: Taste) -> u16 {
    match t {
        Taste::Tab => TAB,
        Taste::Enter => ENTER,
        Taste::Space => LEERTASTE,
        Taste::Backspace => BACKSPACE,
        Taste::Delete => 111,
        Taste::Escape => 1,
        Taste::Left => 105,
        Taste::Right => 106,
        Taste::Up => 103,
        Taste::Down => 108,
        Taste::Home => 102,
        Taste::End => 107,
    }
}

/// Tote Taste, die zusammen mit der Leertaste das Zeichen selbst ergibt (z. B. `^` auf `de`)
fn tot(c: char) -> Option<xkb::Keysym> {
    use xkb::Keysym as K;
    Some(match c {
        '^' => K::dead_circumflex,
        '`' => K::dead_grave,
        '´' => K::dead_acute,
        '~' => K::dead_tilde,
        _ => return None,
    })
}

/// Erste Taste (niedrigster Code, wenigste Umschalter), die auf diesem Layout `ziel` erzeugt
fn suchen(km: &xkb::Keymap, layout: u32, ziel: xkb::Keysym) -> Option<(u16, bool, bool)> {
    for (umschalt, altgr) in [(false, false), (true, false), (false, true), (true, true)] {
        // Nur der normale Tastenblock (evdev < 128): Sondertasten wie KEY_EURO hat kaum eine echte Tastatur
        for code in km.min_keycode().raw()..=km.max_keycode().raw().min(128 + 7) {
            let mut st = xkb::State::new(km);
            st.update_mask(0, 0, 0, 0, 0, layout);
            if umschalt {
                st.update_key(xkb::Keycode::new(LINKS_UMSCHALT as u32 + 8), xkb::KeyDirection::Down);
            }
            if altgr {
                st.update_key(xkb::Keycode::new(RECHTS_ALT as u32 + 8), xkb::KeyDirection::Down);
            }
            if code >= 8 && st.key_get_one_sym(xkb::Keycode::new(code)) == ziel {
                return Some(((code - 8) as u16, umschalt, altgr));
            }
        }
    }
    None
}

/// Tasten für ein Zeichen, oder None für ein Fremdzeichen
fn zeichen(km: &xkb::Keymap, layout: u32, c: char) -> Option<Vec<Schritt>> {
    match c {
        '\n' => return Some(vec![Schritt::Taste { code: ENTER, umschalt: true, altgr: false }]), // neue Zeile statt Absenden in Chats
        '\t' => return Some(vec![taste(TAB)]),
        _ => {}
    }
    let sym = xkb::utf32_to_keysym(c as u32);
    if let Some((code, umschalt, altgr)) = suchen(km, layout, sym) {
        return Some(vec![Schritt::Taste { code, umschalt, altgr }]);
    }
    let (code, umschalt, altgr) = suchen(km, layout, tot(c)?)?;
    Some(vec![Schritt::Taste { code, umschalt, altgr }, taste(LEERTASTE)])
}

/// Ausgabe für eine Ersetzung: erst `loeschen` Rücktasten, dann die Aktionen
pub fn plan(km: &xkb::Keymap, layout: u32, loeschen: usize, aktionen: &[Aktion], fremd: Fremd) -> Vec<Schritt> {
    let mut out = vec![taste(BACKSPACE); loeschen];
    for a in aktionen {
        match a {
            Aktion::Taste(t, n) => out.extend(std::iter::repeat_n(taste(token_code(*t)), *n as usize)),
            Aktion::Text(text) => {
                let teile: Vec<Option<Vec<Schritt>>> = text.chars().map(|c| zeichen(km, layout, c)).collect();
                if fremd == Fremd::Einfuegen && teile.iter().any(Option::is_none) {
                    out.push(Schritt::Einfuegen(text.clone()));
                    continue;
                }
                for (c, t) in text.chars().zip(teile) {
                    match t {
                        Some(s) => out.extend(s),
                        None => out.push(Schritt::Unicode(c)),
                    }
                }
            }
        }
    }
    out
}

/// Strg+Umschalt+U, Hex-Ziffern, Leertaste: Unicode-Eingabe von IBus (GNOME)
pub fn unicode_tasten(km: &xkb::Keymap, layout: u32, c: char) -> Vec<Schritt> {
    let mut out = vec![];
    for h in format!("{:x}", c as u32).chars() {
        if let Some(s) = zeichen(km, layout, h) {
            out.extend(s);
        }
    }
    out.push(taste(LEERTASTE));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keymap(layout: &str) -> xkb::Keymap {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(&ctx, "evdev", "pc105", layout, "", None, xkb::KEYMAP_COMPILE_NO_FLAGS).expect("Layout fehlt (xkeyboard-config)")
    }
    fn t(code: u16, umschalt: bool, altgr: bool) -> Schritt {
        Schritt::Taste { code, umschalt, altgr }
    }

    #[test]
    fn deutsches_layout_umlaute_euro_und_umschalt() {
        let km = keymap("de");
        let p = plan(&km, 0, 1, &[Aktion::Text("Grüße, 30 €".into())], Fremd::Einfuegen);
        assert_eq!(
            p,
            [
                t(BACKSPACE, false, false),
                t(34, true, false),  // G
                t(19, false, false), // r
                t(26, false, false), // ü
                t(12, false, false), // ß
                t(18, false, false), // e
                t(51, false, false), // ,
                t(57, false, false),
                t(4, false, false),  // 3
                t(11, false, false), // 0
                t(57, false, false),
                t(18, false, true), // € = AltGr+E
            ]
        );
    }

    #[test]
    fn zeilenumbruch_tab_und_tokens() {
        let km = keymap("us");
        let p = plan(&km, 0, 0, &[Aktion::Text("a\nb\t".into()), Aktion::Taste(Taste::Enter, 2)], Fremd::Unicode);
        assert_eq!(p, [t(30, false, false), t(ENTER, true, false), t(48, false, false), t(TAB, false, false), t(ENTER, false, false), t(ENTER, false, false)]);
    }

    #[test]
    fn tote_taste_mit_leertaste() {
        let km = keymap("de");
        assert_eq!(plan(&km, 0, 0, &[Aktion::Text("^".into())], Fremd::Unicode), [t(41, false, false), t(LEERTASTE, false, false)]);
    }

    #[test]
    fn fremdzeichen_kde_ganzer_abschnitt_gnome_einzeln() {
        let km = keymap("us");
        let a = [Aktion::Text("Grüße 🙂".into()), Aktion::Taste(Taste::Tab, 1), Aktion::Text("ok".into())];
        let kde = plan(&km, 0, 0, &a, Fremd::Einfuegen);
        assert_eq!(kde, [Schritt::Einfuegen("Grüße 🙂".into()), t(TAB, false, false), t(24, false, false), t(37, false, false)]);
        let gnome = plan(&km, 0, 0, &[Aktion::Text("aü".into())], Fremd::Unicode);
        assert_eq!(gnome, [t(30, false, false), Schritt::Unicode('ü')]);
    }

    #[test]
    fn unicode_eingabe_hex_und_leertaste() {
        let km = keymap("us");
        // ü = U+00FC -> "fc"
        assert_eq!(unicode_tasten(&km, 0, 'ü'), [t(33, false, false), t(46, false, false), t(LEERTASTE, false, false)]);
    }

    #[test]
    fn zweites_layout_der_gruppe() {
        // GNOME/KDE liefern alle Layouts in einer Keymap, das aktive ist der Index
        let km = keymap("us,de");
        assert_eq!(plan(&km, 1, 0, &[Aktion::Text("z".into())], Fremd::Unicode), [t(21, false, false)]); // de: z liegt auf der Y-Taste
        assert_eq!(plan(&km, 0, 0, &[Aktion::Text("z".into())], Fremd::Unicode), [t(44, false, false)]);
    }
}

/// Welche Art Sitzung: wlroots-artige mit virtual-keyboard (Niri, Sway, Hyprland) oder GNOME/KDE
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sitzung {
    Wlroots,
    Gnome,
    Kde,
    /// F36: X11-Sitzung (Cinnamon, Xfce, MATE, GNOME/KDE unter X11)
    X11,
}

pub fn sitzung(env: impl Fn(&str) -> Option<String>) -> Sitzung {
    // F36: ohne Wayland, aber mit DISPLAY ist es X11 (auch GNOME/KDE unter X11)
    if env("WAYLAND_DISPLAY").is_none() && env("DISPLAY").is_some() {
        return Sitzung::X11;
    }
    if env("NIRI_SOCKET").is_some() || env("SWAYSOCK").is_some() || env("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Sitzung::Wlroots;
    }
    let desktop = env("XDG_CURRENT_DESKTOP").unwrap_or_default().to_uppercase();
    if desktop.split(':').any(|d| d == "KDE") {
        Sitzung::Kde
    } else if desktop.split(':').any(|d| d == "GNOME") {
        Sitzung::Gnome
    } else {
        Sitzung::Wlroots
    }
}

/// Index des aktiven Layouts auf GNOME aus `gsettings get … sources` und `… mru-sources`:
/// das zuletzt benutzte (erstes in mru) an seiner Stelle in sources. Unbekannt = 0.
pub fn gnome_layout(sources: &str, mru: &str) -> u32 {
    let liste = |s: &str| -> Vec<String> { s.split("('").skip(1).map(|t| t.split("')").next().unwrap_or("").replace("', '", ":")).collect() };
    let (alle, zuletzt) = (liste(sources), liste(mru));
    zuletzt.first().and_then(|z| alle.iter().position(|a| a == z)).unwrap_or(0) as u32
}

#[cfg(test)]
mod sitzung_tests {
    use super::*;

    fn env<'a>(paare: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| paare.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn erkennt_sitzungen() {
        assert_eq!(sitzung(env(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")])), Sitzung::Gnome);
        assert_eq!(sitzung(env(&[("XDG_CURRENT_DESKTOP", "KDE")])), Sitzung::Kde);
        assert_eq!(sitzung(env(&[("XDG_CURRENT_DESKTOP", "GNOME"), ("NIRI_SOCKET", "/x")])), Sitzung::Wlroots);
        assert_eq!(sitzung(env(&[("XDG_CURRENT_DESKTOP", "niri")])), Sitzung::Wlroots);
        assert_eq!(sitzung(env(&[])), Sitzung::Wlroots);
        // F36
        assert_eq!(sitzung(env(&[("DISPLAY", ":0"), ("XDG_CURRENT_DESKTOP", "X-Cinnamon")])), Sitzung::X11);
        assert_eq!(sitzung(env(&[("DISPLAY", ":0"), ("XDG_CURRENT_DESKTOP", "GNOME")])), Sitzung::X11);
        assert_eq!(sitzung(env(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0"), ("XDG_CURRENT_DESKTOP", "GNOME")])), Sitzung::Gnome);
    }

    #[test]
    fn gnome_aktives_layout() {
        let sources = "[('xkb', 'us'), ('xkb', 'de+nodeadkeys'), ('ibus', 'mozc-jp')]";
        assert_eq!(gnome_layout(sources, "[('xkb', 'de+nodeadkeys'), ('xkb', 'us')]"), 1);
        assert_eq!(gnome_layout(sources, "@a(ss) []"), 0);
        assert_eq!(gnome_layout(sources, "[('xkb', 'fr')]"), 0);
    }
}

/// Zwischenablage für Fremdzeichen auf KDE (ext-data-control). `setzen` merkt sich den alten Inhalt.
pub trait Zwischenablage {
    fn setzen(&mut self, text: &str) -> Result<(), String>;
    fn zuruecksetzen(&mut self);
}

/// Schritte als Tastenereignisse senden: `senden(code, 1 = drücken / 0 = loslassen)`
pub fn ausfuehren(
    schritte: &[Schritt],
    km: &xkb::Keymap,
    layout: u32,
    senden: &mut dyn FnMut(u16, i32),
    ablage: &mut dyn Zwischenablage,
    verzoegerung: std::time::Duration,
    lebt: &dyn Fn(),
) -> Result<(), String> {
    let drueck = |code: u16, umschalt: bool, altgr: bool, strg: bool, senden: &mut dyn FnMut(u16, i32)| {
        lebt();
        let mods: Vec<u16> = [(strg, LINKS_STRG), (umschalt, LINKS_UMSCHALT), (altgr, RECHTS_ALT)].iter().filter(|(an, _)| *an).map(|(_, c)| *c).collect();
        for m in &mods {
            senden(*m, 1);
        }
        senden(code, 1);
        senden(code, 0);
        for m in mods.iter().rev() {
            senden(*m, 0);
        }
        if !verzoegerung.is_zero() {
            std::thread::sleep(verzoegerung);
        }
    };
    for s in schritte {
        match s {
            Schritt::Taste { code, umschalt, altgr } => drueck(*code, *umschalt, *altgr, false, senden),
            Schritt::Unicode(c) => {
                let u = suchen(km, layout, xkb::Keysym::u).map(|(code, _, _)| code).ok_or("keine U-Taste im Layout")?;
                drueck(u, true, false, true, senden);
                // IBus braucht einen Moment, um in den Unicode-Modus zu wechseln und das Zeichen einzufügen
                std::thread::sleep(IBUS_PAUSE);
                for t in unicode_tasten(km, layout, *c) {
                    if let Schritt::Taste { code, umschalt, altgr } = t {
                        drueck(code, umschalt, altgr, false, senden);
                    }
                }
                std::thread::sleep(IBUS_PAUSE);
            }
            Schritt::Einfuegen(text) => {
                ablage.setzen(text)?;
                drueck(EINFG, true, false, false, senden);
                // Die App holt den Inhalt asynchron ab. ponytail: feste Wartezeit statt Abholung abzuwarten
                std::thread::sleep(std::time::Duration::from_millis(150));
                ablage.zuruecksetzen();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod ausfuehren_tests {
    use super::*;

    #[derive(Default)]
    struct Attrappe {
        log: Vec<String>,
    }
    impl Zwischenablage for Attrappe {
        fn setzen(&mut self, text: &str) -> Result<(), String> {
            self.log.push(format!("setzen {text}"));
            Ok(())
        }
        fn zuruecksetzen(&mut self) {
            self.log.push("zurück".into());
        }
    }

    fn lauf(schritte: &[Schritt]) -> (Vec<(u16, i32)>, Vec<String>) {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let km = xkb::Keymap::new_from_names(&ctx, "evdev", "pc105", "us", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS).unwrap();
        let mut ev = vec![];
        let mut ab = Attrappe::default();
        ausfuehren(schritte, &km, 0, &mut |c, w| ev.push((c, w)), &mut ab, std::time::Duration::ZERO, &|| {}).unwrap();
        (ev, ab.log)
    }

    #[test]
    fn umschalt_und_altgr_umschliessen_die_taste() {
        let (ev, _) = lauf(&[Schritt::Taste { code: 18, umschalt: true, altgr: true }]);
        assert_eq!(ev, [(42, 1), (100, 1), (18, 1), (18, 0), (100, 0), (42, 0)]);
    }

    #[test]
    fn unicode_eingabe_strg_umschalt_u_dann_hex() {
        let (ev, _) = lauf(&[Schritt::Unicode('é')]); // U+00E9 -> e9
        assert_eq!(ev, [(29, 1), (42, 1), (22, 1), (22, 0), (42, 0), (29, 0), (18, 1), (18, 0), (10, 1), (10, 0), (57, 1), (57, 0)]);
    }

    #[test]
    fn einfuegen_setzt_ablage_drueckt_umschalt_einfg_und_stellt_zurueck() {
        let (ev, log) = lauf(&[Schritt::Einfuegen("🙂".into())]);
        assert_eq!(ev, [(42, 1), (110, 1), (110, 0), (42, 0)]);
        assert_eq!(log, ["setzen 🙂", "zurück"]);
    }
}

/// F11 AC-5: aktives Layout bei jeder Ersetzung frisch lesen. Nicht lesbar = erstes Layout.
pub fn aktuelles_layout(s: Sitzung) -> u32 {
    match s {
        Sitzung::Kde => crate::kde::layout().unwrap_or(0),
        Sitzung::Gnome => {
            let lesen = |k: &str| {
                std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.input-sources", k]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
            };
            gnome_layout(&lesen("sources"), &lesen("mru-sources"))
        }
        Sitzung::X11 => crate::x11::gruppe(),
        Sitzung::Wlroots => 0,
    }
}

/// GNOME: Zwischenablage wird nie benutzt (Fremdzeichen per Unicode-Eingabe)
pub struct KeineAblage;
impl Zwischenablage for KeineAblage {
    fn setzen(&mut self, _: &str) -> Result<(), String> {
        Err("keine Zwischenablage".into())
    }
    fn zuruecksetzen(&mut self) {}
}
