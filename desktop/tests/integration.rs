//! Startet tippkraft im Trocken-Modus gegen eine virtuelle Testtastatur.
//! Die echte Tastatur und das fokussierte Fenster bleiben unberührt (--geraet, --trocken).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device, EventType, InputEvent, KeyCode};

struct Lauf {
    tastatur: VirtualDevice,
    pfad: std::path::PathBuf,
    kind: Child,
    stdout: mpsc::Receiver<String>,
}

impl Drop for Lauf {
    fn drop(&mut self) {
        let _ = self.kind.kill();
    }
}

fn starten(name: &str, env: &[(&str, &str)]) -> Lauf {
    let mut keys = AttributeSet::<KeyCode>::new();
    for k in [KeyCode::KEY_A, KeyCode::KEY_Z, KeyCode::KEY_M, KeyCode::KEY_F, KeyCode::KEY_G, KeyCode::KEY_ENTER, KeyCode::KEY_SPACE, KeyCode::KEY_BACKSPACE, KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTALT] {
        keys.insert(k);
    }
    let mut tastatur = VirtualDevice::builder().unwrap().name(&format!("tippkraft-test-{name}")).with_keys(&keys).unwrap().build().unwrap();
    let pfad = tastatur.enumerate_dev_nodes_blocking().unwrap().next().unwrap().unwrap();
    // udev braucht einen Moment für die Rechte am Gerät
    std::thread::sleep(Duration::from_millis(300));

    std::fs::create_dir_all(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let stand = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("stand-{name}.json"));
    std::fs::write(&stand, r#"{"version":1,"bausteine":[{"kuerzel":"mfg","inhalt":"Mit freundlichen Grüßen","ausloeser":"leertaste","wortgrenze":true,"grossKleinBeachten":false,"schreibweiseUebernehmen":true}],"ausschluesse":[]}"#).unwrap();

    let mut kind = Command::new(env!("CARGO_BIN_EXE_tippkraft"))
        .args(["run", "--trocken", "--geraet", pfad.to_str().unwrap(), "--stand", stand.to_str().unwrap()])
        .envs(env.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(kind.stderr.take().unwrap()).lines();
    let mut bisher = String::new();
    loop {
        let zeile = stderr.next().unwrap_or_else(|| panic!("tippkraft endete vor 'bereit': {bisher}")).unwrap();
        if zeile.contains("bereit") {
            break;
        }
        bisher += &zeile;
    }
    std::thread::spawn(move || for _ in stderr {});
    let (tx, stdout) = mpsc::channel();
    let out = kind.stdout.take().unwrap();
    std::thread::spawn(move || {
        for z in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(z);
        }
    });
    Lauf { tastatur, pfad, kind, stdout }
}

fn druecken(l: &mut Lauf, k: KeyCode, wert: i32) {
    l.tastatur.emit(&[InputEvent::new(EventType::KEY.0, k.code(), wert)]).unwrap();
    std::thread::sleep(Duration::from_millis(20));
}

fn tippen(l: &mut Lauf, keys: &[KeyCode]) {
    for &k in keys {
        druecken(l, k, 1);
        druecken(l, k, 0);
    }
}

fn exit_innerhalb(l: &mut Lauf, max: Duration) -> Option<i32> {
    let ende = Instant::now() + max;
    while Instant::now() < ende {
        if let Some(s) = l.kind.try_wait().unwrap() {
            return s.code();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn ersetzt_und_uebernimmt_die_tastatur_ac16() {
    let mut l = starten("ersetzen", &[]);
    assert!(Device::open(&l.pfad).unwrap().grab().is_err(), "Testtastatur muss exklusiv übernommen sein");
    tippen(&mut l, &[KeyCode::KEY_M, KeyCode::KEY_F, KeyCode::KEY_G, KeyCode::KEY_SPACE]);
    let zeile = l.stdout.recv_timeout(Duration::from_secs(2)).expect("keine Ersetzung gemeldet");
    assert_eq!(zeile, r#"ersetzen loeschen=3 [Text("Mit freundlichen Grüßen")]"#);
    // getippte Zeichen selbst werden nie ausgegeben
    tippen(&mut l, &[KeyCode::KEY_A, KeyCode::KEY_Z]);
    assert!(l.stdout.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn notausschalter_beendet_ac17() {
    let mut l = starten("notaus", &[]);
    druecken(&mut l, KeyCode::KEY_LEFTCTRL, 1);
    druecken(&mut l, KeyCode::KEY_LEFTALT, 1);
    druecken(&mut l, KeyCode::KEY_BACKSPACE, 1);
    assert_eq!(exit_innerhalb(&mut l, Duration::from_secs(2)), Some(0));
    druecken(&mut l, KeyCode::KEY_BACKSPACE, 0);
    druecken(&mut l, KeyCode::KEY_LEFTALT, 0);
    druecken(&mut l, KeyCode::KEY_LEFTCTRL, 0);
    assert!(Device::open(&l.pfad).unwrap().grab().is_ok(), "Tastatur muss wieder frei sein");
}

#[test]
fn watchdog_beendet_bei_haenger_ac18() {
    let mut l = starten("watchdog", &[("TIPPKRAFT_TEST_HAENGEN", "1")]);
    druecken(&mut l, KeyCode::KEY_A, 1);
    assert_eq!(exit_innerhalb(&mut l, Duration::from_secs(4)), Some(3));
    druecken(&mut l, KeyCode::KEY_A, 0);
}
