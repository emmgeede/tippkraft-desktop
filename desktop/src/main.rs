//! tippkraft-Daemon: übernimmt die Tastaturen exklusiv, reicht jede Taste über eine uinput-Tastatur weiter
//! und hält nur die Auslösetaste eines passenden Kürzels zurück.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device, EventType, InputEvent, KeyCode};
use tippkraft::engine::{Eingabe, Engine, Entscheidung};
use tippkraft::format::{self, Stand};
use tippkraft::{niri, wayland};
use xkbcommon::xkb;

const WATCHDOG: Duration = Duration::from_secs(2);
const WEITERGABE_NAME: &str = "tippkraft-weitergabe";

enum Ereignis {
    Taste { key: KeyCode, wert: i32 },
    Klick,
    Fokus(Option<String>),
}

struct Optionen {
    stand: PathBuf,
    geraete: Vec<PathBuf>,
    trocken: bool,
    verzoegerung: Duration,
}

fn optionen() -> Result<Optionen, String> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("run") {
        return Err("Aufruf: tippkraft run [--stand DATEI] [--geraet /dev/input/eventN] [--verzoegerung MS] [--trocken]".into());
    }
    let daten = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    let mut o = Optionen { stand: daten.join("tippkraft/stand.json"), geraete: vec![], trocken: false, verzoegerung: Duration::from_millis(1) };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--stand" => o.stand = args.next().ok_or("--stand braucht einen Pfad")?.into(),
            "--geraet" => o.geraete.push(args.next().ok_or("--geraet braucht einen Pfad")?.into()),
            "--verzoegerung" => o.verzoegerung = Duration::from_millis(args.next().and_then(|v| v.parse().ok()).ok_or("--verzoegerung braucht Millisekunden")?),
            "--trocken" => o.trocken = true,
            _ => return Err(format!("Unbekannte Option {a}")),
        }
    }
    Ok(o)
}

fn stand_laden(pfad: &PathBuf) -> Stand {
    match std::fs::read_to_string(pfad) {
        Ok(json) => format::parse(&json).unwrap_or_else(|e| {
            eprintln!("tippkraft: {e} ({})", pfad.display());
            Stand::default()
        }),
        Err(_) => Stand::default(),
    }
}

fn ist_tastatur(d: &Device) -> bool {
    d.name() != Some(WEITERGABE_NAME)
        && d.supported_keys().is_some_and(|k| k.contains(KeyCode::KEY_A) && k.contains(KeyCode::KEY_Z) && k.contains(KeyCode::KEY_ENTER))
}

fn ist_zeiger(d: &Device) -> bool {
    d.supported_keys().is_some_and(|k| k.contains(KeyCode::BTN_LEFT) || k.contains(KeyCode::BTN_TOUCH))
}

/// Übernimmt die Tastatur erst, wenn keine Taste mehr gedrückt ist, sonst bleibt eine Taste "hängen"
fn uebernehmen(d: &mut Device) -> std::io::Result<()> {
    let ende = Instant::now() + Duration::from_secs(5);
    while d.get_key_state()?.iter().next().is_some() && Instant::now() < ende {
        std::thread::sleep(Duration::from_millis(20));
    }
    d.grab()
}

fn lesen(mut d: Device, tx: Sender<Ereignis>, tastatur: bool) {
    std::thread::spawn(move || {
        loop {
            let Ok(events) = d.fetch_events() else { return };
            for ev in events {
                if ev.event_type() != EventType::KEY {
                    continue;
                }
                let key = KeyCode::new(ev.code());
                let e = if tastatur {
                    Ereignis::Taste { key, wert: ev.value() }
                } else if ev.value() == 1 {
                    Ereignis::Klick
                } else {
                    continue;
                };
                if tx.send(e).is_err() {
                    return;
                }
            }
        }
    });
}

fn ist_modifier(k: KeyCode) -> bool {
    use KeyCode as K;
    matches!(k, K::KEY_LEFTSHIFT | K::KEY_RIGHTSHIFT | K::KEY_LEFTCTRL | K::KEY_RIGHTCTRL | K::KEY_LEFTALT | K::KEY_RIGHTALT | K::KEY_LEFTMETA | K::KEY_RIGHTMETA | K::KEY_CAPSLOCK)
}

/// Übersetzt einen Tastendruck in eine Engine-Eingabe. `None`: Puffer nicht anfassen (reine Modifier).
fn klassifizieren(key: KeyCode, state: &xkb::State) -> Option<Eingabe> {
    use KeyCode as K;
    if ist_modifier(key) {
        return None;
    }
    let mod_aktiv = |m: &str| state.mod_name_is_active(m, xkb::STATE_MODS_EFFECTIVE);
    if mod_aktiv(xkb::MOD_NAME_CTRL) || mod_aktiv(xkb::MOD_NAME_ALT) || mod_aktiv(xkb::MOD_NAME_LOGO) {
        return Some(Eingabe::Zuruecksetzen);
    }
    Some(match key {
        K::KEY_SPACE => Eingabe::Leertaste,
        K::KEY_ENTER | K::KEY_KPENTER => Eingabe::Enter,
        K::KEY_TAB if mod_aktiv(xkb::MOD_NAME_SHIFT) => Eingabe::Zuruecksetzen,
        K::KEY_TAB => Eingabe::Tab,
        K::KEY_BACKSPACE => Eingabe::Backspace,
        _ => {
            let s = state.key_get_utf8(xkb::Keycode::new(key.code() as u32 + 8));
            let mut cs = s.chars();
            match (cs.next(), cs.next()) {
                (Some(c), None) if !c.is_control() => Eingabe::Zeichen(c),
                // Navigation, Funktionstasten, Dead Keys: Position unklar
                _ => Eingabe::Zuruecksetzen,
            }
        }
    })
}

fn xkb_state(keymap: &str) -> Result<xkb::State, String> {
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let km = xkb::Keymap::new_from_string(&ctx, keymap.to_string(), xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS).ok_or("Keymap des Compositors nicht lesbar")?;
    Ok(xkb::State::new(&km))
}

fn jetzt_ms() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn main() {
    if let Err(e) = run() {
        eprintln!("tippkraft: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let o = optionen()?;
    let mut wl = wayland::Wayland::verbinden(!o.trocken)?;
    let mut state = xkb_state(wl.zustand.keymap.as_deref().unwrap_or_default())?;
    wl.zustand.neu = false;

    let mut engine = Engine::new(stand_laden(&o.stand));
    let mut stand_zeit = std::fs::metadata(&o.stand).and_then(|m| m.modified()).ok();

    // Geräte: mit --geraet nur diese (Tests), sonst alle Tastaturen übernehmen und Zeiger nur lesen
    let (tx, rx) = mpsc::channel::<Ereignis>();
    let mut tasten: AttributeSet<KeyCode> = AttributeSet::new();
    let mut anzahl = 0;
    let geraete: Vec<(Device, bool)> = if o.geraete.is_empty() {
        evdev::enumerate()
            .filter_map(|(_, d)| if ist_tastatur(&d) { Some((d, true)) } else if ist_zeiger(&d) { Some((d, false)) } else { None })
            .collect()
    } else {
        o.geraete.iter().map(|p| Device::open(p).map(|d| (d, true)).map_err(|e| format!("{}: {e}", p.display()))).collect::<Result<_, _>>()?
    };
    for (mut d, tastatur) in geraete {
        if tastatur {
            for k in d.supported_keys().map(|k| k.iter().collect::<Vec<_>>()).unwrap_or_default() {
                tasten.insert(k);
            }
            uebernehmen(&mut d).map_err(|e| format!("Tastatur {} nicht übernehmbar: {e}", d.name().unwrap_or("?")))?;
            anzahl += 1;
        }
        lesen(d, tx.clone(), tastatur);
    }
    if anzahl == 0 {
        return Err("Keine Tastatur gefunden (Gruppe input?)".into());
    }
    let mut weitergabe = if o.trocken {
        None
    } else {
        Some(VirtualDevice::builder().and_then(|b| b.name(WEITERGABE_NAME).with_keys(&tasten)).and_then(|b| b.build()).map_err(|e| format!("uinput nicht nutzbar: {e}"))?)
    };

    if o.geraete.is_empty() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            if let Err(e) = niri::beobachten(|app| {
                let _ = tx.send(Ereignis::Fokus(app));
            }) {
                eprintln!("tippkraft: {e}, Ausschlussliste ist ohne Wirkung");
            }
        });
    }

    // Watchdog: hängt die Hauptschleife, Prozess beenden. Der Kernel gibt die Tastaturen dann frei.
    let herzschlag = Arc::new(AtomicU64::new(jetzt_ms()));
    {
        let herzschlag = herzschlag.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            if jetzt_ms().saturating_sub(herzschlag.load(Ordering::Relaxed)) > WATCHDOG.as_millis() as u64 {
                eprintln!("tippkraft: Hauptschleife hängt, beende mich, damit die Tastatur frei wird");
                std::process::exit(3);
            }
        });
    }
    let test_haengen = std::env::var_os("TIPPKRAFT_TEST_HAENGEN").is_some();

    eprintln!("tippkraft: bereit ({anzahl} Tastatur(en), {})", if o.trocken { "trocken" } else { "aktiv" });
    let mut gedrueckt: HashSet<KeyCode> = HashSet::new();
    let mut zurueckgehalten: HashSet<KeyCode> = HashSet::new();
    let mut letzte_pruefung = Instant::now();

    loop {
        herzschlag.store(jetzt_ms(), Ordering::Relaxed);
        wl.pumpen();
        if wl.zustand.neu {
            wl.zustand.neu = false;
            if let Some(km) = &wl.zustand.keymap {
                state = xkb_state(km)?;
            }
        }
        if letzte_pruefung.elapsed() > Duration::from_secs(1) {
            letzte_pruefung = Instant::now();
            let zeit = std::fs::metadata(&o.stand).and_then(|m| m.modified()).ok();
            if zeit != stand_zeit {
                stand_zeit = zeit;
                engine.stand(stand_laden(&o.stand));
            }
        }

        let ereignis = match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(e) => e,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return Err("Alle Eingabegeräte sind weg".into()),
        };
        let (key, wert) = match ereignis {
            Ereignis::Klick => {
                engine.eingabe(Eingabe::Zuruecksetzen);
                continue;
            }
            Ereignis::Fokus(app) => {
                engine.fokus(app.as_deref());
                continue;
            }
            Ereignis::Taste { key, wert } => (key, wert),
        };
        if test_haengen {
            std::thread::sleep(Duration::from_secs(10));
        }
        let kc = xkb::Keycode::new(key.code() as u32 + 8);
        let weiter = |w: &mut Option<VirtualDevice>, wert: i32| {
            if let Some(w) = w {
                let _ = w.emit(&[InputEvent::new(EventType::KEY.0, key.code(), wert)]);
            }
        };

        match wert {
            0 => {
                gedrueckt.remove(&key);
                state.update_key(kc, xkb::KeyDirection::Up);
                if !zurueckgehalten.remove(&key) {
                    weiter(&mut weitergabe, 0);
                }
            }
            2 => {
                // gehaltene Taste: das Programm wiederholt nach eigenem Takt, Puffer ist dann unzuverlässig
                engine.eingabe(Eingabe::Zuruecksetzen);
                if !zurueckgehalten.contains(&key) {
                    weiter(&mut weitergabe, 2);
                }
            }
            _ => {
                gedrueckt.insert(key);
                if key == KeyCode::KEY_BACKSPACE && gedrueckt.contains(&KeyCode::KEY_LEFTCTRL) && gedrueckt.contains(&KeyCode::KEY_LEFTALT) {
                    eprintln!("tippkraft: Notausschalter (Strg+Alt+Backspace), beende mich");
                    std::process::exit(0);
                }
                let eingabe = klassifizieren(key, &state);
                state.update_key(kc, xkb::KeyDirection::Down);
                let entscheidung = eingabe.map_or(Entscheidung::Durchlassen, |e| engine.eingabe(e));
                match entscheidung {
                    Entscheidung::Durchlassen => weiter(&mut weitergabe, 1),
                    Entscheidung::Ersetzen { loeschen, aktionen } => {
                        zurueckgehalten.insert(key);
                        if o.trocken {
                            println!("ersetzen loeschen={loeschen} {aktionen:?}");
                        } else {
                            // kurz warten, damit bereits weitergegebene Tasten vor der Ausgabe ankommen
                            std::thread::sleep(Duration::from_millis(15));
                            if let Err(e) = wl.tippen(loeschen, &aktionen, o.verzoegerung, || herzschlag.store(jetzt_ms(), Ordering::Relaxed)) {
                                eprintln!("tippkraft: Ausgabe fehlgeschlagen: {e}");
                            }
                        }
                    }
                }
            }
        }
    }
}
