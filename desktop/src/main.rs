//! tippkraft: Textexpander für Wayland.
//! `run` übernimmt die Tastaturen exklusiv, reicht jede Taste über eine uinput-Tastatur weiter
//! und hält nur die Auslösetaste eines passenden Kürzels zurück.

use std::collections::HashSet;
use std::os::fd::{AsFd, OwnedFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device, EventType, InputEvent, KeyCode};
use tippkraft::engine::{Eingabe, Engine, Entscheidung};
use tippkraft::format::{self, Stand};
use tippkraft::sync::{self, Pfade, Verbindung, Zugang};
use tippkraft::{niri, steuerung, wayland};
use xkbcommon::xkb;

const WATCHDOG: Duration = Duration::from_secs(2);
const WEITERGABE_NAME: &str = "tippkraft-weitergabe";
const HILFE: &str = "tippkraft – Textexpander für Wayland

  tippkraft einrichten            Installieren, Autostart einrichten, starten und mit dem Konto verbinden
  tippkraft verbinden             Mit dem Konto verbinden (öffnet den Browser, dort bestätigen)
  tippkraft status                Zustand anzeigen
  tippkraft pause                 Pausieren bzw. fortsetzen (gibt die Tastatur frei)
  tippkraft run [--verzoegerung MS] [--stand DATEI] [--geraet /dev/input/eventN] [--trocken]

Notausschalter: linke Strg + linke Alt + Backspace beendet tippkraft sofort.";

enum Ereignis {
    Taste { key: KeyCode, wert: i32 },
    Klick,
    Fokus(Option<String>),
    Sync(sync::Meldung),
    Steuerung(String, Sender<String>),
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pfade = Pfade::aus_umgebung();
    let ergebnis = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..], pfade),
        Some("verbinden") => verbinden(&pfade, &args[1..]),
        Some("login") => Err(format!("`login` gibt es nicht mehr. Verbinden geht jetzt über den Browser: tippkraft verbinden\n\n{HILFE}")),
        Some("status") => status(&pfade),
        Some("pause") => pause(&pfade),
        Some("einrichten") => einrichten(),
        _ => {
            println!("{HILFE}");
            return;
        }
    };
    if let Err(e) = ergebnis {
        eprintln!("tippkraft: {e}");
        std::process::exit(1);
    }
}

fn benachrichtigen(text: &str) {
    if std::env::var_os("TIPPKRAFT_KEINE_BENACHRICHTIGUNG").is_none() {
        let _ = std::process::Command::new("notify-send").args(["-a", "tippkraft", "-t", "4000", text]).status();
    }
}

fn verbinden(pfade: &Pfade, args: &[String]) -> Result<(), String> {
    let server = match args {
        [] => None,
        [o, url] if o == "--server" => Some(url.as_str()),
        _ => return Err(format!("Aufruf: tippkraft verbinden [--server URL]\n\n{HILFE}")),
    };
    let browser = std::env::var("TIPPKRAFT_BROWSER").unwrap_or_else(|_| "xdg-open".into());
    match sync::verbinden(pfade, server, &browser) {
        Ok((server, n)) => {
            let text = format!("Verbunden mit {server}, {n} Bausteine");
            println!("{text}");
            benachrichtigen(&format!("tippkraft: {text}"));
            Ok(())
        }
        Err(e) => {
            // auch sichtbar, wenn über den App-Starter ohne Terminal gestartet
            benachrichtigen(&format!("tippkraft: {e}"));
            Err(e)
        }
    }
}

fn status(pfade: &Pfade) -> Result<(), String> {
    let antwort = steuerung::senden(&pfade.socket, "status").ok_or("tippkraft läuft nicht")?;
    println!("{antwort}");
    Ok(())
}

fn pause(pfade: &Pfade) -> Result<(), String> {
    let antwort = steuerung::senden(&pfade.socket, "pause").ok_or("tippkraft läuft nicht")?;
    let text = format!("tippkraft {antwort}");
    println!("{text}");
    benachrichtigen(&text);
    Ok(())
}

const UNIT: &str = "[Unit]
Description=tippkraft Textexpander
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart=%h/.local/bin/tippkraft run
# Exit 0 (Notausschalter) startet nicht neu, Exit 3 (Watchdog) und Abstürze schon
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
";

fn einrichten() -> Result<(), String> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME nicht gesetzt")?);
    let ziel = home.join(".local/bin/tippkraft");
    let quelle = std::env::current_exe().map_err(|e| e.to_string())?;
    if quelle != ziel {
        std::fs::create_dir_all(ziel.parent().unwrap()).map_err(|e| e.to_string())?;
        // über temporäre Datei + rename, damit auch eine laufende alte Version ersetzt werden kann
        let tmp = ziel.with_extension("neu");
        std::fs::copy(&quelle, &tmp).and_then(|_| std::fs::rename(&tmp, &ziel)).map_err(|e| format!("{}: {e}", ziel.display()))?;
    }
    let unit = home.join(".config/systemd/user/tippkraft.service");
    std::fs::create_dir_all(unit.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&unit, UNIT).map_err(|e| e.to_string())?;
    let systemctl = std::env::var("TIPPKRAFT_SYSTEMCTL").unwrap_or_else(|_| "systemctl".into());
    for args in [&["--user", "daemon-reload"][..], &["--user", "enable", "tippkraft.service"], &["--user", "restart", "tippkraft.service"]] {
        let ok = std::process::Command::new(&systemctl).args(args).status().map_err(|e| e.to_string())?.success();
        if !ok {
            return Err(format!("systemctl {} fehlgeschlagen", args.join(" ")));
        }
    }
    let starter = home.join(".local/share/applications/tippkraft-verbinden.desktop");
    std::fs::create_dir_all(starter.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&starter, format!("[Desktop Entry]\nType=Application\nName=tippkraft verbinden\nComment=Desktop-App mit dem tippkraft-Konto verbinden\nExec={} verbinden\nIcon=input-keyboard\nTerminal=false\nCategories=Utility;\n", ziel.display())).map_err(|e| e.to_string())?;
    println!("Eingerichtet: {} und tippkraft.service läuft.", ziel.display());
    let pfade = Pfade::aus_umgebung();
    if Zugang::laden(&pfade.zugang).is_some() {
        println!("Schon verbunden. Neu verbinden: „tippkraft verbinden“ im App-Starter.");
        return Ok(());
    }
    verbinden(&pfade, &[])
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

/// Übernommen werden nur reine Tastaturen. Mäuse mit Tasten (z. B. MX Master) haben Achsen,
/// deren Bewegungen die Weitergabe nicht durchreicht, sie würden sonst einfrieren.
fn tastatur_ohne_achsen(name: Option<&str>, keys: Option<&evdev::AttributeSetRef<KeyCode>>, hat_achsen: bool) -> bool {
    name != Some(WEITERGABE_NAME)
        && !hat_achsen
        && keys.is_some_and(|k| k.contains(KeyCode::KEY_A) && k.contains(KeyCode::KEY_Z) && k.contains(KeyCode::KEY_ENTER))
}

fn ist_tastatur(d: &Device) -> bool {
    let achsen = d.supported_relative_axes().is_some_and(|r| r.iter().next().is_some()) || d.supported_absolute_axes().is_some_and(|a| a.iter().next().is_some());
    tastatur_ohne_achsen(d.name(), d.supported_keys(), achsen)
}

fn ist_zeiger(d: &Device) -> bool {
    d.supported_keys().is_some_and(|k| k.contains(KeyCode::BTN_LEFT) || k.contains(KeyCode::BTN_TOUCH))
}

/// EVIOCGRAB auf einem dupliziertem fd (teilt sich die Übernahme mit dem Gerät im Lese-Thread)
fn greifen(fd: &OwnedFd, an: bool) -> std::io::Result<()> {
    const EVIOCGRAB: rustix::ioctl::Opcode = rustix::ioctl::opcode::write::<i32>(b'E', 0x90);
    // SAFETY: EVIOCGRAB erwartet den Wert direkt als Argument, keinen Zeiger
    unsafe { rustix::ioctl::ioctl(fd, rustix::ioctl::IntegerSetter::<EVIOCGRAB>::new_usize(an as usize)) }.map_err(Into::into)
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

struct Optionen {
    geraete: Vec<PathBuf>,
    trocken: bool,
    verzoegerung: Duration,
}

fn optionen(args: &[String], pfade: &mut Pfade) -> Result<Optionen, String> {
    let mut o = Optionen { geraete: vec![], trocken: false, verzoegerung: Duration::from_millis(1) };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--stand" => pfade.stand = it.next().ok_or("--stand braucht einen Pfad")?.into(),
            "--geraet" => o.geraete.push(it.next().ok_or("--geraet braucht einen Pfad")?.into()),
            "--verzoegerung" => o.verzoegerung = Duration::from_millis(it.next().and_then(|v| v.parse().ok()).ok_or("--verzoegerung braucht Millisekunden")?),
            "--trocken" => o.trocken = true,
            _ => return Err(format!("Unbekannte Option {a}\n\n{HILFE}")),
        }
    }
    Ok(o)
}

fn run(args: &[String], mut pfade: Pfade) -> Result<(), String> {
    let o = optionen(args, &mut pfade)?;
    let (tx, rx) = mpsc::channel::<Ereignis>();

    // Zuerst den Steuer-Socket: läuft schon ein Daemon, keine Tastatur anfassen
    let (steuer_tx, steuer_rx) = mpsc::channel::<(String, Sender<String>)>();
    steuerung::lauschen(&pfade.socket, steuer_tx)?;
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for (befehl, antwort) in steuer_rx {
                if tx.send(Ereignis::Steuerung(befehl, antwort)).is_err() {
                    return;
                }
            }
        });
    }

    let mut wl = wayland::Wayland::verbinden(!o.trocken)?;
    let mut state = xkb_state(wl.zustand.keymap.as_deref().unwrap_or_default())?;
    wl.zustand.neu = false;

    let mut engine = Engine::new(stand_laden(&pfade.stand));
    let mut stand_zeit = std::fs::metadata(&pfade.stand).and_then(|m| m.modified()).ok();

    // Geräte: mit --geraet nur diese (Tests), sonst alle Tastaturen übernehmen und Zeiger nur lesen
    let mut tasten: AttributeSet<KeyCode> = AttributeSet::new();
    let mut griffe: Vec<OwnedFd> = vec![];
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
            griffe.push(d.as_fd().try_clone_to_owned().map_err(|e| e.to_string())?);
        }
        lesen(d, tx.clone(), tastatur);
    }
    if griffe.is_empty() {
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
    {
        let tx = tx.clone();
        let pfade = Pfade { zugang: pfade.zugang.clone(), stand: pfade.stand.clone(), socket: pfade.socket.clone() };
        std::thread::spawn(move || sync::schleife(&pfade, |m| drop(tx.send(Ereignis::Sync(m)))));
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

    eprintln!("tippkraft: bereit ({} Tastatur(en), {})", griffe.len(), if o.trocken { "trocken" } else { "aktiv" });
    let mut gedrueckt: HashSet<KeyCode> = HashSet::new();
    let mut weitergegeben: HashSet<KeyCode> = HashSet::new();
    let mut zurueckgehalten: HashSet<KeyCode> = HashSet::new();
    let mut letzte_pruefung = Instant::now();
    let mut verbindung = Verbindung::NichtAngemeldet;
    let mut pausiert = false;
    let mut greifen_ausstehend = false;

    loop {
        herzschlag.store(jetzt_ms(), Ordering::Relaxed);
        wl.pumpen();
        if wl.zustand.neu {
            wl.zustand.neu = false;
            if let Some(km) = &wl.zustand.keymap {
                state = xkb_state(km)?;
            }
        }
        // Fortsetzen erst, wenn keine Taste mehr gedrückt ist (sonst bleiben z. B. Mod+Alt hängen)
        if greifen_ausstehend && gedrueckt.is_empty() {
            for g in &griffe {
                let _ = greifen(g, true);
            }
            greifen_ausstehend = false;
            pausiert = false;
        }
        if letzte_pruefung.elapsed() > Duration::from_secs(1) {
            letzte_pruefung = Instant::now();
            let zeit = std::fs::metadata(&pfade.stand).and_then(|m| m.modified()).ok();
            if zeit != stand_zeit {
                stand_zeit = zeit;
                engine.stand(stand_laden(&pfade.stand));
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
            Ereignis::Sync(sync::Meldung::Verbindung(v)) => {
                verbindung = v;
                continue;
            }
            Ereignis::Sync(sync::Meldung::Stand(s)) => {
                engine.stand(s);
                stand_zeit = std::fs::metadata(&pfade.stand).and_then(|m| m.modified()).ok();
                continue;
            }
            Ereignis::Steuerung(befehl, antwort) => {
                let text = match befehl.as_str() {
                    "pause" if pausiert || greifen_ausstehend => {
                        greifen_ausstehend = true;
                        "aktiv".to_string()
                    }
                    "pause" => {
                        // Tasten, die über die Weitergabe gedrückt sind, dort loslassen, sonst hängen sie im Compositor
                        if let Some(w) = &mut weitergabe {
                            for k in weitergegeben.drain() {
                                let _ = w.emit(&[InputEvent::new(EventType::KEY.0, k.code(), 0)]);
                            }
                        }
                        for g in &griffe {
                            let _ = greifen(g, false);
                        }
                        engine.eingabe(Eingabe::Zuruecksetzen);
                        pausiert = true;
                        "pausiert".to_string()
                    }
                    "status" => {
                        let server = Zugang::laden(&pfade.zugang).map(|z| z.server).unwrap_or_else(|| "–".into());
                        format!(
                            "Zustand: {} · Verbindung: {verbindung} · Bausteine: {} · Server: {server}",
                            if pausiert || greifen_ausstehend { "pausiert" } else { "aktiv" },
                            engine.anzahl()
                        )
                    }
                    _ => "unbekannter Befehl".to_string(),
                };
                let _ = antwort.send(text);
                continue;
            }
            Ereignis::Taste { key, wert } => (key, wert),
        };
        if test_haengen {
            std::thread::sleep(Duration::from_secs(10));
        }
        let kc = xkb::Keycode::new(key.code() as u32 + 8);
        // Während der Pause kommen die Tasten ohnehin beim Compositor an: nur mitzählen, nichts weitergeben
        if pausiert || greifen_ausstehend {
            match wert {
                0 => {
                    gedrueckt.remove(&key);
                    state.update_key(kc, xkb::KeyDirection::Up);
                }
                1 => {
                    gedrueckt.insert(key);
                    state.update_key(kc, xkb::KeyDirection::Down);
                }
                _ => {}
            }
            continue;
        }
        let mut weiter = |wert: i32| {
            if let Some(w) = &mut weitergabe {
                let _ = w.emit(&[InputEvent::new(EventType::KEY.0, key.code(), wert)]);
                match wert {
                    1 => weitergegeben.insert(key),
                    0 => weitergegeben.remove(&key),
                    _ => false,
                };
            }
        };

        match wert {
            0 => {
                gedrueckt.remove(&key);
                state.update_key(kc, xkb::KeyDirection::Up);
                if !zurueckgehalten.remove(&key) {
                    weiter(0);
                }
            }
            2 => {
                // gehaltene Taste: das Programm wiederholt nach eigenem Takt, Puffer ist dann unzuverlässig
                engine.eingabe(Eingabe::Zuruecksetzen);
                if !zurueckgehalten.contains(&key) {
                    weiter(2);
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
                    Entscheidung::Durchlassen => weiter(1),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(ks: &[KeyCode]) -> AttributeSet<KeyCode> {
        let mut a = AttributeSet::new();
        for k in ks {
            a.insert(*k);
        }
        a
    }

    #[test]
    fn maeuse_mit_tasten_werden_nicht_uebernommen() {
        let tastatur = keys(&[KeyCode::KEY_A, KeyCode::KEY_Z, KeyCode::KEY_ENTER]);
        assert!(tastatur_ohne_achsen(Some("Keychron K2"), Some(&tastatur), false));
        // MX Master 3 und Keychron-Zweitschnittstelle: Tasten A–Z plus Maus-/Absolutachsen
        assert!(!tastatur_ohne_achsen(Some("Logitech MX Master 3 for Mac"), Some(&tastatur), true));
        assert!(!tastatur_ohne_achsen(Some(WEITERGABE_NAME), Some(&tastatur), false));
        assert!(!tastatur_ohne_achsen(Some("Power Button"), Some(&keys(&[KeyCode::KEY_POWER])), false));
    }
}
