//! Wayland: Keymap des Compositors lesen und Text über zwp_virtual_keyboard_v1 tippen.

use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use crate::format::{Aktion, Taste};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;
use xkbcommon::xkb;

#[derive(Default)]
pub struct Zustand {
    /// Keymap-Text vom Compositor, `neu` sobald er sich geändert hat (Layoutwechsel in der Config)
    pub keymap: Option<String>,
    pub neu: bool,
}

pub struct Wayland {
    conn: Connection,
    queue: EventQueue<Zustand>,
    pub zustand: Zustand,
    vk: Option<ZwpVirtualKeyboardV1>,
    start: Instant,
}

impl Wayland {
    /// `ausgabe: false` nur für `--trocken`: Keymap lesen, aber keine virtuelle Tastatur anlegen
    pub fn verbinden(ausgabe: bool) -> Result<Self, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("Keine Wayland-Sitzung: {e}"))?;
        let (globals, mut queue) = registry_queue_init::<Zustand>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).map_err(|e| format!("Kein wl_seat: {e}"))?;
        seat.get_keyboard(&qh, ());
        let vk = if ausgabe {
            let mgr: ZwpVirtualKeyboardManagerV1 =
                globals.bind(&qh, 1..=1, ()).map_err(|e| format!("Compositor kann keine virtuelle Tastatur (zwp_virtual_keyboard_v1): {e}"))?;
            Some(mgr.create_virtual_keyboard(&seat, &qh, ()))
        } else {
            None
        };
        let mut zustand = Zustand::default();
        for _ in 0..5 {
            queue.roundtrip(&mut zustand).map_err(|e| e.to_string())?;
            if zustand.keymap.is_some() {
                break;
            }
        }
        if zustand.keymap.is_none() {
            return Err("Compositor hat kein Keymap geschickt".into());
        }
        Ok(Wayland { conn, queue, zustand, vk, start: Instant::now() })
    }

    /// Wartende Ereignisse verarbeiten, ohne zu blockieren
    pub fn pumpen(&mut self) {
        let _ = self.conn.flush();
        if let Some(guard) = self.queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut fds = [rustix::event::PollFd::new(&fd, rustix::event::PollFlags::IN)];
            let bereit = rustix::event::poll(&mut fds, Some(&rustix::time::Timespec { tv_sec: 0, tv_nsec: 0 })).unwrap_or(0);
            if bereit > 0 {
                let _ = guard.read();
            }
        }
        let _ = self.queue.dispatch_pending(&mut self.zustand);
    }

    /// `loeschen`-mal Backspace, dann die Aktionen tippen. `lebt` wird pro Taste aufgerufen (Watchdog).
    pub fn tippen(&mut self, loeschen: usize, aktionen: &[Aktion], verzoegerung: Duration, lebt: impl Fn()) -> Result<(), String> {
        let vk = self.vk.as_ref().ok_or("keine virtuelle Tastatur")?;
        let folge = tastenfolge(loeschen, aktionen);
        if folge.is_empty() {
            return Ok(());
        }
        let mut symbole: Vec<xkb::Keysym> = Vec::new();
        for (s, _) in &folge {
            if !symbole.contains(s) {
                symbole.push(*s);
            }
        }
        // Eigenes Keymap mit genau den benötigten Zeichen, je Zeichen eine Taste ohne Ebenen
        let keymap = keymap_text(&symbole);
        let fd = rustix::fs::memfd_create("tippkraft-keymap", rustix::fs::MemfdFlags::CLOEXEC).map_err(|e| e.to_string())?;
        let mut datei = std::fs::File::from(fd);
        datei.write_all(keymap.as_bytes()).and_then(|_| datei.write_all(&[0])).map_err(|e| e.to_string())?;
        vk.keymap(xkb::KEYMAP_FORMAT_TEXT_V1, datei.as_fd(), keymap.len() as u32 + 1);
        vk.modifiers(0, 0, 0, 0);
        for (s, umschalt) in folge {
            lebt();
            let code = symbole.iter().position(|x| *x == s).unwrap() as u32 + 1; // evdev-Code = xkb-Code - 8
            let t = self.start.elapsed().as_millis() as u32;
            if umschalt {
                vk.modifiers(SHIFT, 0, 0, 0);
            }
            vk.key(t, code, 1);
            vk.key(t, code, 0);
            if umschalt {
                vk.modifiers(0, 0, 0, 0);
            }
            self.conn.flush().map_err(|e| e.to_string())?;
            if !verzoegerung.is_zero() {
                std::thread::sleep(verzoegerung);
            }
        }
        self.conn.flush().map_err(|e| e.to_string())
    }
}

fn taste_symbol(t: Taste) -> xkb::Keysym {
    use xkb::Keysym as K;
    match t {
        Taste::Tab => K::Tab,
        Taste::Enter => K::Return,
        Taste::Space => K::space,
        Taste::Backspace => K::BackSpace,
        Taste::Delete => K::Delete,
        Taste::Escape => K::Escape,
        Taste::Left => K::Left,
        Taste::Right => K::Right,
        Taste::Up => K::Up,
        Taste::Down => K::Down,
        Taste::Home => K::Home,
        Taste::End => K::End,
    }
}

/// Modifier-Maske für Umschalt im xkb-Standard ("complete")
const SHIFT: u32 = 1;

/// Tasten mit Umschalt-Flag. Ein Zeilenumbruch im Text wird Umschalt+Eingabe: Chats wie Messenger,
/// Slack oder WhatsApp schicken bei reiner Eingabetaste ab. Nur [[Enter]] tippt die echte Eingabetaste.
pub fn tastenfolge(loeschen: usize, aktionen: &[Aktion]) -> Vec<(xkb::Keysym, bool)> {
    let mut out = vec![(xkb::Keysym::BackSpace, false); loeschen];
    for a in aktionen {
        match a {
            Aktion::Text(t) => {
                for c in t.chars() {
                    out.push(match c {
                        '\n' => (xkb::Keysym::Return, true),
                        '\t' => (xkb::Keysym::Tab, false),
                        c => (xkb::utf32_to_keysym(c as u32), false),
                    });
                }
            }
            Aktion::Taste(t, n) => out.extend(std::iter::repeat_n((taste_symbol(*t), false), *n as usize)),
        }
    }
    out.retain(|(s, _)| s.raw() != 0); // NoSymbol: nicht darstellbar
    out
}

pub fn keymap_text(symbole: &[xkb::Keysym]) -> String {
    let mut codes = String::new();
    let mut syms = String::new();
    for (i, s) in symbole.iter().enumerate() {
        codes += &format!("<K{i}> = {};\n", i + 9);
        syms += &format!("key <K{i}> {{[ {} ]}};\n", xkb::keysym_get_name(*s));
    }
    format!(
        "xkb_keymap {{\nxkb_keycodes \"tippkraft\" {{ minimum = 8; maximum = {}; {codes}}};\nxkb_types \"tippkraft\" {{ include \"complete\" }};\nxkb_compatibility \"tippkraft\" {{ include \"complete\" }};\nxkb_symbols \"tippkraft\" {{ {syms}}};\n}};\n",
        symbole.len() + 9
    )
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Zustand {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_seat::WlSeat, ()> for Zustand {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for Zustand {
    fn event(_: &mut Self, _: &ZwpVirtualKeyboardManagerV1, _: <ZwpVirtualKeyboardManagerV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwpVirtualKeyboardV1, ()> for Zustand {
    fn event(_: &mut Self, _: &ZwpVirtualKeyboardV1, _: <ZwpVirtualKeyboardV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for Zustand {
    fn event(z: &mut Self, _: &wl_keyboard::WlKeyboard, e: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_keyboard::Event::Keymap { format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1), fd, size } = e {
            // pread ab Position 0: Das fd teilt sich den Lesezeiger mit anderen Clients
            let mut buf = vec![0u8; size as usize];
            if std::os::unix::fs::FileExt::read_exact_at(&std::fs::File::from(fd), &mut buf, 0).is_ok() {
                let text = String::from_utf8_lossy(&buf).trim_end_matches('\0').to_string();
                // Das eigene Ausgabe-Keymap schickt der Compositor auch an uns. Nicht als Layout übernehmen.
                if !text.contains("tippkraft") {
                    z.keymap = Some(text);
                    z.neu = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keysym-Name ohne führende Nullen bei Unicode-Namen (ältere libxkbcommon: "U0001F642", neuere: "U1F642")
    fn name(s: xkb::Keysym) -> String {
        let n = xkb::keysym_get_name(s);
        match n.strip_prefix('U').and_then(|h| u32::from_str_radix(h, 16).ok()) {
            Some(cp) => format!("U{cp:X}"),
            None => n,
        }
    }

    #[test]
    fn tastenfolge_mit_umlauten_emoji_und_tokens() {
        let f = tastenfolge(2, &[Aktion::Text("ä€🙂\n".into()), Aktion::Taste(Taste::Tab, 2)]);
        let namen: Vec<String> = f.iter().map(|(s, _)| name(*s)).collect();
        assert_eq!(namen, ["BackSpace", "BackSpace", "adiaeresis", "EuroSign", "U1F642", "Return", "Tab", "Tab"]);
    }

    /// Zeilenumbruch im Text = Umschalt+Eingabe (neue Zeile in Chats), nur [[Enter]] ist die echte Eingabetaste
    #[test]
    fn zeilenumbruch_mit_umschalt_enter_token_ohne() {
        let f = tastenfolge(0, &[Aktion::Text("a\nb".into()), Aktion::Taste(Taste::Enter, 1)]);
        let r = xkb::Keysym::Return;
        assert_eq!(f, [(xkb::utf32_to_keysym('a' as u32), false), (r, true), (xkb::utf32_to_keysym('b' as u32), false), (r, false)]);
    }

    #[test]
    fn keymap_ist_kompilierbar() {
        let syms: Vec<xkb::Keysym> = tastenfolge(1, &[Aktion::Text("Grüße 🙂 A".into())]).into_iter().map(|(s, _)| s).collect();
        let mut uniq = vec![];
        for s in syms {
            if !uniq.contains(&s) {
                uniq.push(s);
            }
        }
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let km = xkb::Keymap::new_from_string(&ctx, keymap_text(&uniq), xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS).expect("Keymap kompiliert nicht");
        let st = xkb::State::new(&km);
        let g = uniq.iter().position(|s| name(*s) == "G").unwrap() as u32;
        let emoji = uniq.iter().position(|s| name(*s) == "U1F642").unwrap() as u32;
        assert_eq!(st.key_get_utf8(xkb::Keycode::new(g + 9)), "G");
        assert_eq!(st.key_get_utf8(xkb::Keycode::new(emoji + 9)), "🙂");
    }
}
