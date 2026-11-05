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
use tippkraft::engine::{Eingabe, Engine, Entscheidung, nach_popup};
use tippkraft::format::Aktion;
use tippkraft::format::{self, Stand};
use tippkraft::sync::{self, Pfade, Verbindung, Zugang};
use tippkraft::i18n::{self, Sprache, T, tr};
use tippkraft::{steuerung, wayland};
use xkbcommon::xkb;

const WATCHDOG: Duration = Duration::from_secs(2);
const WEITERGABE_NAME: &str = "tippkraft-weitergabe";

enum Ereignis {
    Taste { key: KeyCode, wert: i32 },
    Klick,
    Fokus(Option<String>),
    Sync(sync::Meldung),
    Steuerung(String, Sender<String>),
    /// Popup geschlossen: Werte oder `None` bei Abbruch (F8)
    Tray(tippkraft::tray::TrayAktion),
    PopupFertig { werte: Option<std::collections::HashMap<String, String>>, loeschen: usize, inhalt: String, gross: bool, abbruch: Vec<Aktion> },
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pfade = Pfade::aus_umgebung();
    // Sprache aus dem gespeicherten Stand (Account), sonst Systemsprache (F15)
    i18n::setzen(Sprache::fuer(stand_laden(&pfade.stand).sprache.as_deref()));
    let ergebnis = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..], pfade),
        Some("verbinden" | "connect") => verbinden(&pfade, &args[1..]),
        Some("login") => Err(format!("{}\n\n{}", tr(T::LoginWeg), tr(T::Hilfe))),
        Some("status") => status(&pfade),
        Some("pause") => pause(&pfade),
        Some("einrichten" | "setup") => einrichten(),
        Some("popup") => popup(),
        // F22: zeigt jeden erkannten Fensterwechsel (Fehlersuche, Container-Tests)
        Some("fokus") => {
            tippkraft::fokus::dauerhaft(|app| {
                use std::io::Write;
                println!("fokus: {}", app.as_deref().unwrap_or("-"));
                let _ = std::io::stdout().flush();
            });
            Ok(())
        }
        Some("uebersicht" | "overview") => {
            let stand = std::fs::read_to_string(&pfade.stand).ok().and_then(|j| format::parse(&j).ok()).unwrap_or_default();
            tippkraft::uebersicht::zeigen(stand, &pfade.socket.with_file_name("tippkraft-uebersicht.lock"))
        }
        _ => {
            println!("{}", tr(T::Hilfe));
            return;
        }
    };
    if let Err(e) = ergebnis {
        eprintln!("tippkraft: {e}");
        std::process::exit(1);
    }
}

/// `tippkraft popup`: vom Daemon gestartet, Anfrage auf stdin, Werte auf stdout, Exit 1 bei Abbruch
fn popup() -> Result<(), String> {
    let anfrage: tippkraft::popup::Anfrage = serde_json::from_reader(std::io::stdin()).map_err(|e| format!("Popup-Anfrage: {e}"))?;
    match tippkraft::popup::zeigen(anfrage)? {
        Some(werte) => {
            println!("{}", serde_json::to_string(&tippkraft::popup::Antwort { werte }).unwrap());
            Ok(())
        }
        None => std::process::exit(1),
    }
}

/// Startet das Popup als eigenen Prozess, damit die Hauptschleife (Tastenweitergabe, Watchdog) weiterläuft
fn popup_starten(tx: Sender<Ereignis>, titel: String, loeschen: usize, inhalt: String, gross: bool, abbruch: Vec<Aktion>, auto: tippkraft::format::Automatisch) {
    std::thread::spawn(move || {
        let programm = std::env::var_os("TIPPKRAFT_POPUP").map(PathBuf::from).or_else(|| std::env::current_exe().ok()).unwrap_or_else(|| "tippkraft".into());
        let anfrage = serde_json::to_string(&tippkraft::popup::Anfrage { titel, inhalt: inhalt.clone(), datum: auto.datum, zwischenablage: auto.zwischenablage }).unwrap();
        let werte = (|| {
            let mut kind = std::process::Command::new(&programm).arg("popup").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().ok()?;
            std::io::Write::write_all(&mut kind.stdin.take()?, anfrage.as_bytes()).ok()?;
            let out = kind.wait_with_output().ok()?;
            if !out.status.success() {
                return None;
            }
            serde_json::from_slice::<tippkraft::popup::Antwort>(&out.stdout).ok().map(|a| a.werte)
        })();
        let _ = tx.send(Ereignis::PopupFertig { werte, loeschen, inhalt, gross, abbruch });
    });
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
        _ => return Err(tr(T::Hilfe).to_string()),
    };
    let browser = std::env::var("TIPPKRAFT_BROWSER").unwrap_or_else(|_| "xdg-open".into());
    match sync::verbinden(pfade, server, &browser) {
        Ok((server, n)) => {
            let text = i18n::f(i18n::aktuell(), T::VerbundenMit, &[&server, &n]);
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

const RELEASES: &str = "https://github.com/emmgeede/tippkraft-desktop/releases/latest";
const INSTALLIEREN: &str = "curl -fsSL https://tippkraft.com/install.sh | sh";

fn status(pfade: &Pfade) -> Result<(), String> {
    let antwort = steuerung::senden(&pfade.socket, "status").ok_or(tr(T::LaeuftNicht))?;
    println!("{antwort}");
    // F21 AC-8: Hinweis aus dem zuletzt geladenen Stand
    let stand = std::fs::read_to_string(&pfade.stand).ok().and_then(|j| format::parse(&j).ok());
    if let Some(v) = stand.and_then(|s| s.desktop_version).filter(|v| format::neuer(v, env!("CARGO_PKG_VERSION"))) {
        println!("{}", i18n::f(i18n::aktuell(), T::UpdateStatus, &[&v, &INSTALLIEREN]));
    }
    Ok(())
}

fn pause(pfade: &Pfade) -> Result<(), String> {
    let antwort = steuerung::senden(&pfade.socket, "pause").ok_or(tr(T::LaeuftNicht))?;
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
    let starter_start = home.join(".local/share/applications/tippkraft-starten.desktop");
    std::fs::create_dir_all(starter_start.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&starter_start, "[Desktop Entry]\nType=Application\nName=Start tippkraft\nName[de]=tippkraft starten\nComment=Start the text expander\nComment[de]=Textexpander starten\nExec=systemctl --user start tippkraft.service\nIcon=input-keyboard\nTerminal=false\nCategories=Utility;\n").map_err(|e| e.to_string())?;
    let starter = home.join(".local/share/applications/tippkraft-verbinden.desktop");
    std::fs::create_dir_all(starter.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&starter, format!("[Desktop Entry]\nType=Application\nName=Connect tippkraft\nName[de]=tippkraft verbinden\nComment=Connect the desktop app to your tippkraft account\nComment[de]=Desktop-App mit dem tippkraft-Konto verbinden\nExec={} verbinden\nIcon=input-keyboard\nTerminal=false\nCategories=Utility;\n", ziel.display())).map_err(|e| e.to_string())?;
    println!("{}", i18n::f(i18n::aktuell(), T::Eingerichtet, &[&ziel.display()]));
    // F22 AC-5: Tastenkürzel für den laufenden Compositor zum Kopieren
    if let Some(q) = tippkraft::fokus::erkennen(|k| std::env::var(k).ok().filter(|v| !v.is_empty())) {
        let (name, datei, zeilen) = tippkraft::fokus::tastenkuerzel(&q, &ziel.display().to_string());
        println!("\n{}\n\n{zeilen}\n", i18n::f(i18n::aktuell(), T::Tastenkuerzel, &[&name, &datei]));
    }
    let pfade = Pfade::aus_umgebung();
    if Zugang::laden(&pfade.zugang).is_some() {
        println!("{}", tr(T::SchonVerbunden));
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
            _ => return Err(format!("{a}?\n\n{}", tr(T::Hilfe))),
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

    let start = stand_laden(&pfade.stand);
    i18n::setzen(Sprache::fuer(start.sprache.as_deref()));
    let mut engine = Engine::new(start);
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
        // F22: Niri, Sway oder Hyprland, mit Neuverbindung
        std::thread::spawn(move || tippkraft::fokus::dauerhaft(|app| drop(tx.send(Ereignis::Fokus(app)))));
    }
    {
        let tx = tx.clone();
        let pfade = Pfade { zugang: pfade.zugang.clone(), stand: pfade.stand.clone(), socket: pfade.socket.clone() };
        std::thread::spawn(move || sync::schleife(&pfade, |m| drop(tx.send(Ereignis::Sync(m)))));
    }

    // Tray-Icon (F13). In Tests mit --geraet nur auf ausdrücklichen Wunsch.
    let tray = if o.geraete.is_empty() || std::env::var_os("TIPPKRAFT_TRAY").is_some() {
        use ksni::blocking::TrayMethods;
        let tx = tx.clone();
        let t = tippkraft::tray::Tray {
            zustand: tippkraft::tray::TrayZustand { pausiert: false, verbindung: Verbindung::NichtAngemeldet, anzahl: engine.anzahl(), server: String::new(), sprache: i18n::aktuell(), update: None },
            senden: Box::new(move |a| drop(tx.send(Ereignis::Tray(a)))),
        };
        // Dienst startet oft vor der DMS-Leiste: dann warten, bis der Watcher da ist, statt aufzugeben
        t.assume_sni_available(true).spawn().map_err(|e| eprintln!("tippkraft: kein Tray-Icon ({e})")).ok()
    } else {
        None
    };
    let mut tray_zustand = None;

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
    let mut update: Option<String> = None; // F21: neuere Version laut Server
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
        if let Some(t) = &tray {
            let z = tippkraft::tray::TrayZustand {
                pausiert: pausiert || greifen_ausstehend,
                verbindung,
                anzahl: engine.anzahl(),
                server: Zugang::laden(&pfade.zugang).map(|z| z.server).unwrap_or_default(),
                sprache: i18n::aktuell(),
                update: update.clone(),
            };
            if tray_zustand.as_ref() != Some(&z) {
                t.update(|t| t.zustand = z.clone());
                tray_zustand = Some(z);
            }
        }
        if letzte_pruefung.elapsed() > Duration::from_secs(1) {
            letzte_pruefung = Instant::now();
            let zeit = std::fs::metadata(&pfade.stand).and_then(|m| m.modified()).ok();
            if zeit != stand_zeit {
                stand_zeit = zeit;
                let s = stand_laden(&pfade.stand);
                i18n::setzen(Sprache::fuer(s.sprache.as_deref()));
                engine.stand(s);
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
                i18n::setzen(Sprache::fuer(s.sprache.as_deref()));
                update = s.desktop_version.clone().filter(|v| tippkraft::format::neuer(v, env!("CARGO_PKG_VERSION")));
                engine.stand(s);
                stand_zeit = std::fs::metadata(&pfade.stand).and_then(|m| m.modified()).ok();
                continue;
            }
            Ereignis::Steuerung(befehl, antwort) => {
                let text = match befehl.as_str() {
                    "pause" if pausiert || greifen_ausstehend => {
                        greifen_ausstehend = true;
                        tr(T::Aktiv).to_string()
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
                        tr(T::Pausiert).to_string()
                    }
                    "status" => {
                        let server = Zugang::laden(&pfade.zugang).map(|z| z.server).unwrap_or_else(|| "–".into());
                        let zustand = tr(if pausiert || greifen_ausstehend { T::Pausiert } else { T::Aktiv });
                        i18n::f(i18n::aktuell(), T::StatusZeile, &[&zustand, &verbindung, &engine.anzahl(), &server])
                    }
                    _ => "unbekannter Befehl".to_string(),
                };
                let _ = antwort.send(text);
                continue;
            }
            Ereignis::Tray(aktion) => {
                use tippkraft::tray::TrayAktion;
                match aktion {
                    TrayAktion::Pause => {
                        // gleicher Weg wie `tippkraft pause`, danach Benachrichtigung
                        let (atx, arx) = mpsc::channel();
                        let _ = tx.send(Ereignis::Steuerung("pause".into(), atx));
                        std::thread::spawn(move || {
                            if let Ok(a) = arx.recv_timeout(Duration::from_secs(3)) {
                                benachrichtigen(&format!("tippkraft {a}"));
                            }
                        });
                    }
                    TrayAktion::Webtool => {
                        let url = Zugang::laden(&pfade.zugang).map(|z| z.server).unwrap_or_else(|| sync::STANDARD_SERVER.into());
                        let browser = std::env::var("TIPPKRAFT_BROWSER").unwrap_or_else(|_| "xdg-open".into());
                        let _ = std::process::Command::new(browser).arg(url).spawn();
                    }
                    TrayAktion::Update => {
                        let browser = std::env::var("TIPPKRAFT_BROWSER").unwrap_or_else(|_| "xdg-open".into());
                        let _ = std::process::Command::new(browser).arg(RELEASES).spawn();
                    }
                    TrayAktion::Uebersicht => {
                        if let Ok(exe) = std::env::current_exe() {
                            let _ = std::process::Command::new(exe).arg("uebersicht").spawn();
                        }
                    }
                    TrayAktion::Verbinden => {
                        if let Ok(exe) = std::env::current_exe() {
                            let _ = std::process::Command::new(exe).arg("verbinden").spawn();
                        }
                    }
                    TrayAktion::Beenden => {
                        if let Some(w) = &mut weitergabe {
                            for k in weitergegeben.drain() {
                                let _ = w.emit(&[InputEvent::new(EventType::KEY.0, k.code(), 0)]);
                            }
                        }
                        eprintln!("tippkraft: über das Tray beendet");
                        std::process::exit(0);
                    }
                }
                continue;
            }
            Ereignis::PopupFertig { werte, loeschen, inhalt, gross, abbruch } => {
                let (loeschen, aktionen) = match werte {
                    Some(w) => (loeschen, nach_popup(&inhalt, &w, &engine.automatisch(&inhalt), gross)),
                    // Abbruch: nichts löschen, nur den zurückgehaltenen Auslöser nachholen
                    None => (0, abbruch),
                };
                if o.trocken {
                    println!("ersetzen loeschen={loeschen} {aktionen:?}");
                } else {
                    // Fokus muss erst ins ursprüngliche Fenster zurück. ponytail: feste Wartezeit statt Fokus-Ereignis abwarten
                    std::thread::sleep(Duration::from_millis(200));
                    if let Err(e) = wl.tippen(loeschen, &aktionen, o.verzoegerung, || herzschlag.store(jetzt_ms(), Ordering::Relaxed)) {
                        eprintln!("tippkraft: Ausgabe fehlgeschlagen: {e}");
                    }
                }
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
                    Entscheidung::Popup { titel, loeschen, inhalt, gross, abbruch } => {
                        zurueckgehalten.insert(key);
                        let auto = engine.automatisch(&inhalt);
                        popup_starten(tx.clone(), titel, loeschen, inhalt, gross, abbruch, auto);
                    }
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
