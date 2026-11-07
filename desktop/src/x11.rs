//! F36: X11-Sitzungen (Cinnamon, Xfce, MATE …). Tastaturlayout, aktives Fenster und Zwischenablage
//! direkt über das X11-Protokoll (x11rb, rein in Rust), ohne Zusatzprogramme.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xkb::ConnectionExt as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

use crate::uinput::Zwischenablage;

type Fehler = Box<dyn std::error::Error>;

fn verbinden() -> Result<(RustConnection, Window), String> {
    let (c, n) = x11rb::connect(None).map_err(|e| format!("Keine X11-Sitzung: {e}"))?;
    let root = c.setup().roots[n].root;
    Ok((c, root))
}

fn atom(c: &RustConnection, name: &str) -> Result<Atom, Fehler> {
    Ok(c.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

/// `_XKB_RULES_NAMES` am Root-Fenster: rules, model, layout, variant, options, jeweils mit NUL getrennt
pub fn rules_namen(daten: &[u8]) -> [String; 5] {
    let mut teile = daten.split(|b| *b == 0).map(|t| String::from_utf8_lossy(t).into_owned());
    std::array::from_fn(|_| teile.next().unwrap_or_default())
}

/// Keymap der Sitzung als Text, aus den XKB-Namen am Root-Fenster kompiliert.
/// ponytail: wird nur beim Start gelesen, Layouts neu hinzufügen braucht einen Neustart (Gruppe wechselt live)
pub fn keymap() -> Result<String, String> {
    let (c, root) = verbinden()?;
    let a = atom(&c, "_XKB_RULES_NAMES").map_err(|e| e.to_string())?;
    let daten = c.get_property(false, root, a, AtomEnum::STRING, 0, 1024).ok().and_then(|k| k.reply().ok()).map(|r| r.value).unwrap_or_default();
    let [rules, model, layout, variant, options] = rules_namen(&daten);
    let ctx = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
    let km = xkbcommon::xkb::Keymap::new_from_names(&ctx, &rules, &model, &layout, &variant, Some(options), xkbcommon::xkb::KEYMAP_COMPILE_NO_FLAGS).ok_or("X11-Tastaturlayout nicht lesbar")?;
    Ok(km.get_as_string(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1))
}

/// Aktive Layout-Gruppe (0 = erstes Layout). Nicht lesbar = 0.
pub fn gruppe() -> u32 {
    let lesen = || -> Result<u32, Fehler> {
        let (c, _) = verbinden()?;
        c.xkb_use_extension(1, 0)?.reply()?;
        let r = c.xkb_get_state(x11rb::protocol::xkb::ID::USE_CORE_KBD.into())?.reply()?;
        Ok(u8::from(r.group) as u32)
    };
    lesen().unwrap_or(0)
}

/// Aktives Fenster melden (Klasse aus WM_CLASS), bei jedem Wechsel von `_NET_ACTIVE_WINDOW`
pub fn beobachten(melden: &mut dyn FnMut(Option<String>)) -> Result<(), String> {
    let lauf = |melden: &mut dyn FnMut(Option<String>)| -> Result<(), Fehler> {
        let (c, root) = verbinden()?;
        let aktiv = atom(&c, "_NET_ACTIVE_WINDOW")?;
        c.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE))?;
        c.flush()?;
        let mut letzte: Option<Option<String>> = None;
        loop {
            let fenster = c.get_property(false, root, aktiv, AtomEnum::WINDOW, 0, 1)?.reply()?.value32().and_then(|mut v| v.next()).unwrap_or(NONE);
            let klasse = (fenster != NONE)
                .then(|| c.get_property(false, fenster, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256).ok()?.reply().ok())
                .flatten()
                .and_then(|r| wm_klasse(&r.value));
            if letzte.as_ref() != Some(&klasse) {
                melden(klasse.clone());
                letzte = Some(klasse);
            }
            // bis zur nächsten Änderung des aktiven Fensters warten
            loop {
                if let Event::PropertyNotify(e) = c.wait_for_event()? {
                    if e.atom == aktiv {
                        break;
                    }
                }
            }
        }
    };
    lauf(melden).map_err(|e| format!("X11: {e}"))
}

/// WM_CLASS = "instanz\0Klasse\0", gemeldet wird die Klasse
pub fn wm_klasse(daten: &[u8]) -> Option<String> {
    let [instanz, klasse, ..] = rules_namen(daten);
    Some(if klasse.is_empty() { instanz } else { klasse }).filter(|k| !k.is_empty())
}

// ---------- Zwischenablage ----------

type Inhalt = Vec<(Atom, Vec<u8>)>;
/// ponytail: Formate über dieser Größe (X11 überträgt sie stückweise per INCR) werden nicht gesichert
const MAX_FORMAT: usize = 256 * 1024;

enum Befehl {
    /// Text, optional HTML (F10)
    Setzen(String, Option<String>, Sender<Result<(), String>>),
    Zurueck(Sender<()>),
}

/// F36: wie die KDE-Ablage. Ein Thread besitzt CLIPBOARD und PRIMARY und liefert aus, alter Inhalt wird gesichert und zurückgestellt.
pub struct X11Ablage {
    tx: Sender<Befehl>,
    /// GNOME: Xwayland dazwischen, Mutter überträgt die Auswahl erst nach einem Moment zu Wayland (F10)
    bruecke: bool,
}

/// ponytail: feste Wartezeit (in der GNOME-VM gemessen), Mutter meldet die Übernahme nicht zurück
const BRUECKE_PAUSE: Duration = Duration::from_millis(80);

impl X11Ablage {
    pub fn starten() -> Result<Self, String> {
        let a = Ablage::neu().map_err(|e| format!("X11-Zwischenablage: {e}"))?;
        let (tx, rx) = channel();
        std::thread::spawn(move || a.schleife(rx));
        Ok(X11Ablage { tx, bruecke: std::env::var_os("WAYLAND_DISPLAY").is_some() })
    }
}

impl Zwischenablage for X11Ablage {
    fn setzen(&mut self, text: &str) -> Result<(), String> {
        let (a, b) = channel();
        self.tx.send(Befehl::Setzen(text.to_string(), None, a)).map_err(|_| "Zwischenablage beendet")?;
        let r = b.recv_timeout(Duration::from_secs(3)).map_err(|_| "Zwischenablage antwortet nicht".to_string())?;
        if self.bruecke {
            std::thread::sleep(BRUECKE_PAUSE);
        }
        r
    }
    fn setzen_html(&mut self, html: &str, text: &str) -> Result<(), String> {
        let (a, b) = channel();
        self.tx.send(Befehl::Setzen(text.to_string(), Some(html.to_string()), a)).map_err(|_| "Zwischenablage beendet")?;
        let r = b.recv_timeout(Duration::from_secs(3)).map_err(|_| "Zwischenablage antwortet nicht".to_string())?;
        if self.bruecke {
            std::thread::sleep(BRUECKE_PAUSE);
        }
        r
    }
    fn zuruecksetzen(&mut self) {
        let (a, b) = channel();
        if self.tx.send(Befehl::Zurueck(a)).is_ok() {
            let _ = b.recv_timeout(Duration::from_secs(3));
        }
    }
}

struct Ablage {
    c: RustConnection,
    win: Window,
    /// CLIPBOARD und PRIMARY
    sel: [Atom; 2],
    targets: Atom,
    eigen: Atom,
    text: [Atom; 4],
    html: Atom,
    /// was wir pro Auswahl gerade ausliefern
    liefern: HashMap<Atom, Inhalt>,
    /// wie oft ein Programm unseren Inhalt abgeholt hat
    abgeholt: u64,
}

impl Ablage {
    fn neu() -> Result<Self, Fehler> {
        let (c, root) = verbinden()?;
        let win = c.generate_id()?;
        c.create_window(0, win, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE))?;
        let sel = [atom(&c, "CLIPBOARD")?, AtomEnum::PRIMARY.into()];
        let text = [atom(&c, "UTF8_STRING")?, AtomEnum::STRING.into(), atom(&c, "text/plain;charset=utf-8")?, atom(&c, "text/plain")?];
        let targets = atom(&c, "TARGETS")?;
        let eigen = atom(&c, "TIPPKRAFT_ABLAGE")?;
        let html = atom(&c, "text/html")?;
        c.flush()?;
        Ok(Ablage { c, win, sel, targets, eigen, text, html, liefern: HashMap::new(), abgeholt: 0 })
    }

    fn schleife(mut self, rx: Receiver<Befehl>) {
        let mut gesichert: [Inhalt; 2] = [vec![], vec![]];
        let mut abgeholt_vorher = 0;
        loop {
            self.pumpen(Duration::from_millis(10));
            match rx.try_recv() {
                Ok(Befehl::Setzen(text, html, antwort)) => {
                    abgeholt_vorher = self.abgeholt;
                    let sel = self.sel;
                    gesichert = sel.map(|s| self.lesen(s));
                    let mut inhalt: Inhalt = self.text.iter().map(|t| (*t, text.as_bytes().to_vec())).collect();
                    if let Some(h) = html {
                        // F10: HTML zuerst anbieten
                        inhalt.insert(0, (self.html, h.into_bytes()));
                    }
                    let r = sel.iter().try_for_each(|s| self.besitzen(*s, inhalt.clone())).map_err(|e| e.to_string());
                    let _ = antwort.send(r);
                }
                Ok(Befehl::Zurueck(antwort)) => {
                    // erst zurückstellen, wenn das Programm abgeholt hat (siehe ablage::ABHOLUNG_MAX)
                    let ende = Instant::now() + crate::ablage::ABHOLUNG_MAX;
                    while self.abgeholt == abgeholt_vorher && Instant::now() < ende {
                        self.pumpen(Duration::from_millis(10));
                    }
                    self.pumpen(Duration::from_millis(50));
                    for (s, inhalt) in self.sel.into_iter().zip(std::mem::take(&mut gesichert)) {
                        if inhalt.is_empty() {
                            self.liefern.remove(&s);
                            let _ = self.c.set_selection_owner(NONE, s, CURRENT_TIME);
                        } else {
                            let _ = self.besitzen(s, inhalt);
                        }
                    }
                    let _ = self.c.flush();
                    let _ = antwort.send(());
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                Err(_) => {}
            }
        }
    }

    fn besitzen(&mut self, s: Atom, inhalt: Inhalt) -> Result<(), Fehler> {
        self.liefern.insert(s, inhalt);
        self.c.set_selection_owner(self.win, s, CURRENT_TIME)?;
        if self.c.get_selection_owner(s)?.reply()?.owner != self.win {
            return Err("Auswahl nicht übernehmbar".into());
        }
        Ok(())
    }

    /// Ereignisse bis zu `zeit` verarbeiten, Anfragen anderer Apps beantworten. Gibt ein SelectionNotify an uns zurück.
    fn pumpen(&mut self, zeit: Duration) -> Option<SelectionNotifyEvent> {
        let ende = Instant::now() + zeit;
        loop {
            match self.c.poll_for_event() {
                Ok(Some(Event::SelectionRequest(r))) => self.antworten(r),
                Ok(Some(Event::SelectionClear(e))) => {
                    self.liefern.remove(&e.selection);
                }
                Ok(Some(Event::SelectionNotify(e))) => return Some(e),
                Ok(Some(_)) => {}
                Ok(None) => {
                    if Instant::now() >= ende {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(_) => return None,
            }
        }
    }

    fn antworten(&mut self, r: SelectionRequestEvent) {
        let prop = if r.property == NONE { r.target } else { r.property };
        let inhalt = self.liefern.get(&r.selection);
        let ok = match inhalt {
            Some(i) if r.target == self.targets => {
                let mut liste: Vec<Atom> = i.iter().map(|(t, _)| *t).collect();
                liste.push(self.targets);
                self.c.change_property32(PropMode::REPLACE, r.requestor, prop, AtomEnum::ATOM, &liste).is_ok()
            }
            Some(i) => {
                let ok = i.iter().find(|(t, _)| *t == r.target).is_some_and(|(t, d)| self.c.change_property8(PropMode::REPLACE, r.requestor, prop, *t, d).is_ok());
                self.abgeholt += ok as u64;
                ok
            }
            None => false,
        };
        let ev = SelectionNotifyEvent { response_type: SELECTION_NOTIFY_EVENT, sequence: 0, time: r.time, requestor: r.requestor, selection: r.selection, target: r.target, property: if ok { prop } else { NONE } };
        let _ = self.c.send_event(false, r.requestor, EventMask::NO_EVENT, ev);
        let _ = self.c.flush();
    }

    /// Ein Format der Auswahl abholen (leer = nicht vorhanden)
    fn abholen(&mut self, s: Atom, ziel: Atom) -> Option<(Atom, Vec<u8>)> {
        self.c.convert_selection(self.win, s, ziel, self.eigen, CURRENT_TIME).ok()?;
        self.c.flush().ok()?;
        let ende = Instant::now() + Duration::from_millis(500);
        while Instant::now() < ende {
            let Some(e) = self.pumpen(Duration::from_millis(20)) else { continue };
            if e.selection != s || e.target != ziel {
                continue;
            }
            if e.property == NONE {
                return None;
            }
            let r = self.c.get_property(true, self.win, self.eigen, AtomEnum::ANY, 0, (MAX_FORMAT / 4) as u32).ok()?.reply().ok()?;
            // INCR (große Inhalte) und abgeschnittene Inhalte nicht sichern
            if r.bytes_after > 0 || r.type_ == atom(&self.c, "INCR").ok()? {
                return None;
            }
            return Some((r.type_, r.value));
        }
        None
    }

    /// Alle Formate der aktuellen Auswahl lesen. Gehört sie uns schon, unseren Inhalt nehmen.
    fn lesen(&mut self, s: Atom) -> Inhalt {
        if self.c.get_selection_owner(s).ok().and_then(|c| c.reply().ok()).map(|r| r.owner) == Some(self.win) {
            return self.liefern.get(&s).cloned().unwrap_or_default();
        }
        let Some((_, liste)) = self.abholen(s, self.targets) else { return vec![] };
        let auslassen = ["TARGETS", "MULTIPLE", "TIMESTAMP", "SAVE_TARGETS", "DELETE", "INSERT_SELECTION", "INSERT_PROPERTY"].map(|n| atom(&self.c, n).unwrap_or(NONE));
        let ziele: Vec<Atom> = liste.chunks_exact(4).map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]])).filter(|t| !auslassen.contains(t)).collect();
        ziele.into_iter().filter_map(|t| self.abholen(s, t).map(|(_, d)| (t, d))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zerlegt_xkb_rules_names() {
        assert_eq!(rules_namen(b"evdev\0pc105\0de,us\0nodeadkeys,\0grp:alt_shift_toggle\0"), ["evdev", "pc105", "de,us", "nodeadkeys,", "grp:alt_shift_toggle"].map(String::from));
        assert_eq!(rules_namen(b"evdev\0pc105\0us\0"), ["evdev", "pc105", "us", "", ""].map(String::from));
    }

    #[test]
    fn wm_class_liefert_die_klasse() {
        assert_eq!(wm_klasse(b"gnome-terminal-server\0Gnome-terminal\0").as_deref(), Some("Gnome-terminal"));
        assert_eq!(wm_klasse(b"xterm\0").as_deref(), Some("xterm"));
        assert_eq!(wm_klasse(b""), None);
    }
}
