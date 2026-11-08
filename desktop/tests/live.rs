//! Ende-zu-Ende-Test in der echten Sitzung (AC-19 bis AC-21). Läuft nur explizit:
//!   cargo test --release --test live -- --ignored --nocapture
//! Öffnet ein eigenes Alacritty-Fenster mit `cat > datei`, übernimmt ALLE Tastaturen und tippt über eine
//! virtuelle Quelltastatur. Während des Tests Maus und Tastatur nicht anfassen.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};

const APP_ID: &str = "tippkraft-test";

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fokus() -> String {
    let out = Command::new("niri").args(["msg", "--json", "focused-window"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    v["app_id"].as_str().unwrap_or_default().to_string()
}

fn quelle() -> VirtualDevice {
    let mut keys = AttributeSet::<KeyCode>::new();
    for k in [
        KeyCode::KEY_A, KeyCode::KEY_D, KeyCode::KEY_F, KeyCode::KEY_G, KeyCode::KEY_M, KeyCode::KEY_Q, KeyCode::KEY_R, KeyCode::KEY_X, KeyCode::KEY_Z,
        KeyCode::KEY_SPACE, KeyCode::KEY_ENTER, KeyCode::KEY_BACKSPACE, KeyCode::KEY_LEFTCTRL,
    ] {
        keys.insert(k);
    }
    let d = VirtualDevice::builder().unwrap().name("tippkraft-live-quelle").with_keys(&keys).unwrap().build().unwrap();
    std::thread::sleep(Duration::from_millis(500));
    d
}

fn taste(q: &mut VirtualDevice, k: KeyCode) {
    q.emit(&[InputEvent::new(EventType::KEY.0, k.code(), 1)]).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    q.emit(&[InputEvent::new(EventType::KEY.0, k.code(), 0)]).unwrap();
    std::thread::sleep(Duration::from_millis(30));
}

fn tippe(q: &mut VirtualDevice, keys: &[KeyCode]) {
    for &k in keys {
        taste(q, k);
    }
    std::thread::sleep(Duration::from_millis(300));
}

fn strg_d(q: &mut VirtualDevice) {
    q.emit(&[InputEvent::new(EventType::KEY.0, KeyCode::KEY_LEFTCTRL.code(), 1)]).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    taste(q, KeyCode::KEY_D);
    q.emit(&[InputEvent::new(EventType::KEY.0, KeyCode::KEY_LEFTCTRL.code(), 0)]).unwrap();
}

/// Ein Durchgang: Fenster öffnen, Fokus prüfen, tippkraft starten, `ablauf` tippen, Dateiinhalt liefern
fn durchgang(name: &str, stand: &str, ablauf: impl FnOnce(&mut VirtualDevice)) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(dir).unwrap();
    let datei = dir.join(format!("live-{name}.txt"));
    let stand_datei = dir.join(format!("live-{name}.json"));
    let _ = std::fs::remove_file(&datei);
    std::fs::write(&stand_datei, stand).unwrap();

    let mut fenster = Kill(Command::new("alacritty").args(["--class", APP_ID, "-e", "sh", "-c", &format!("cat > '{}'", datei.display())]).spawn().unwrap());
    let ende = Instant::now() + Duration::from_secs(5);
    while fokus() != APP_ID {
        assert!(Instant::now() < ende, "Testfenster hat keinen Fokus, Abbruch ohne Eingaben (Fokus: {})", fokus());
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut q = quelle();
    let mut tk = Kill(
        Command::new(env!("CARGO_BIN_EXE_tippkraft"))
            .args(["run", "--state", stand_datei.to_str().unwrap()])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut err = BufReader::new(tk.0.stderr.take().unwrap()).lines();
    assert!(err.next().unwrap().unwrap().contains("bereit"));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fokus(), APP_ID, "Fokus verloren, Abbruch");

    ablauf(&mut q);
    strg_d(&mut q);
    let ende = Instant::now() + Duration::from_secs(3);
    while fenster.0.try_wait().unwrap().is_none() && Instant::now() < ende {
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(tk);
    drop(fenster);
    std::fs::read_to_string(&datei).unwrap_or_default()
}

#[test]
#[ignore]
fn live_ende_zu_ende() {
    use KeyCode as K;
    let stand = r#"{"version":1,"bausteine":[
        {"kuerzel":"mfg","inhalt":"Mit freundlichen Grüßen €🙂","ausloeser":"leertaste","wortgrenze":true,"grossKleinBeachten":false,"schreibweiseUebernehmen":true},
        {"kuerzel":"qq","inhalt":"Signatur","ausloeser":"sofort","wortgrenze":true,"grossKleinBeachten":false,"schreibweiseUebernehmen":true},
        {"kuerzel":"adr","inhalt":"Musterstraße 1[[Tab]]ä","ausloeser":"enter","wortgrenze":true,"grossKleinBeachten":false,"schreibweiseUebernehmen":true}
    ],"ausschluesse":[]}"#;
    let text = durchgang("aktiv", stand, |q| {
        tippe(q, &[K::KEY_M, K::KEY_F, K::KEY_G, K::KEY_SPACE]); // AC-19 Leertaste, Umlaute, €, Emoji
        tippe(q, &[K::KEY_Q, K::KEY_Q]); // sofort
        tippe(q, &[K::KEY_SPACE, K::KEY_A, K::KEY_D, K::KEY_R, K::KEY_ENTER]); // AC-20 Enter wird verbraucht
        tippe(q, &[K::KEY_X, K::KEY_ENTER]); // normales Enter
        tippe(q, &[K::KEY_M, K::KEY_F, K::KEY_G, K::KEY_SPACE, K::KEY_BACKSPACE]); // Rückgängig
        tippe(q, &[K::KEY_ENTER]);
    });
    // Fremde Eingaben vor dem Test (z. B. Diktier-Apps beim Fensterwechsel) ignorieren, aber melden
    let soll = "Mit freundlichen Grüßen €🙂Signatur Musterstraße 1\täx\nmfg \n";
    assert!(text.ends_with(soll), "{text:?}");
    if text != soll {
        eprintln!("Hinweis: fremde Eingabe vor dem Test: {:?}", &text[..text.len() - soll.len()]);
    }

    let ausgeschlossen = stand.replace(r#""ausschluesse":[]"#, &format!(r#""ausschluesse":["{APP_ID}"]"#));
    let text = durchgang("ausgeschlossen", &ausgeschlossen, |q| {
        tippe(q, &[K::KEY_M, K::KEY_F, K::KEY_G, K::KEY_SPACE, K::KEY_ENTER]); // AC-21
    });
    assert!(text.ends_with("mfg \n"), "{text:?}");
}
