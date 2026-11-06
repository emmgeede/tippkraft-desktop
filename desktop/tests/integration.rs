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
    umgebung: Vec<(String, String)>,
}

/// Eigene XDG-Verzeichnisse pro Test, damit weder echte Config noch echter Socket berührt werden
fn umgebung(name: &str) -> Vec<(String, String)> {
    let basis = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("xdg-{name}"));
    let _ = std::fs::remove_dir_all(&basis);
    let mut v = vec![];
    // Socket-Pfade dürfen höchstens 108 Zeichen lang sein, deshalb das Laufzeitverzeichnis unter /run/user
    let lauf = std::path::PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or("/tmp".into())).join(format!("tk-test-{name}"));
    let _ = std::fs::remove_dir_all(&lauf);
    for (var, d) in [("XDG_CONFIG_HOME", basis.join("config")), ("XDG_DATA_HOME", basis.join("data")), ("XDG_RUNTIME_DIR", lauf)] {
        std::fs::create_dir_all(&d).unwrap();
        v.push((var.to_string(), d.to_string_lossy().into_owned()));
    }
    v.push(("TIPPKRAFT_KEINE_BENACHRICHTIGUNG".into(), "1".into()));
    // Die Tests prüfen deutsche Texte (F15: sonst Systemsprache)
    v.push(("TIPPKRAFT_SPRACHE".into(), "de".into()));
    // Wayland sucht seinen Socket relativ zu XDG_RUNTIME_DIR, deshalb als absoluten Pfad mitgeben
    if let (Ok(r), Ok(w)) = (std::env::var("XDG_RUNTIME_DIR"), std::env::var("WAYLAND_DISPLAY")) {
        v.push(("WAYLAND_DISPLAY".into(), format!("{r}/{w}")));
    }
    v
}

/// CLI-Befehl in der Testumgebung ausführen: (Exit-Code, stdout+stderr)
fn cli(umgebung: &[(String, String)], args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_tippkraft")).args(args).envs(umgebung.iter().map(|(a, b)| (a, b))).output().unwrap();
    (out.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

impl Drop for Lauf {
    fn drop(&mut self) {
        let _ = self.kind.kill();
    }
}

const STAND_MFG: &str = r#"{"version":1,"bausteine":[{"kuerzel":"mfg","inhalt":"Mit freundlichen Grüßen","ausloeser":"leertaste","wortgrenze":true,"grossKleinBeachten":false,"schreibweiseUebernehmen":true}],"ausschluesse":[]}"#;

fn starten(name: &str, env: &[(&str, &str)]) -> Lauf {
    starten_mit(name, env, umgebung(name), Some(STAND_MFG))
}

fn starten_mit(name: &str, env: &[(&str, &str)], umgebung: Vec<(String, String)>, stand: Option<&str>) -> Lauf {
    let mut keys = AttributeSet::<KeyCode>::new();
    for k in [KeyCode::KEY_A, KeyCode::KEY_B, KeyCode::KEY_C, KeyCode::KEY_R, KeyCode::KEY_I, KeyCode::KEY_E, KeyCode::KEY_Z, KeyCode::KEY_M, KeyCode::KEY_F, KeyCode::KEY_G, KeyCode::KEY_ENTER, KeyCode::KEY_SPACE, KeyCode::KEY_BACKSPACE, KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTALT] {
        keys.insert(k);
    }
    let mut tastatur = VirtualDevice::builder().unwrap().name(&format!("tippkraft-test-{name}")).with_keys(&keys).unwrap().build().unwrap();
    let pfad = tastatur.enumerate_dev_nodes_blocking().unwrap().next().unwrap().unwrap();
    // udev braucht einen Moment für die Rechte am Gerät
    std::thread::sleep(Duration::from_millis(300));

    let mut args = vec!["run".to_string(), "--trocken".into(), "--geraet".into(), pfad.to_string_lossy().into_owned()];
    if let Some(inhalt) = stand {
        let datei = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("stand-{name}.json"));
        std::fs::write(&datei, inhalt).unwrap();
        args.extend(["--stand".into(), datei.to_string_lossy().into_owned()]);
    }
    let mut kind = Command::new(env!("CARGO_BIN_EXE_tippkraft"))
        .args(&args)
        .envs(umgebung.iter().map(|(a, b)| (a, b)))
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
    Lauf { tastatur, pfad, kind, stdout, umgebung }
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

// ---------- F6: Sync, Login, Status, Pause, Einrichten ----------

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Nachgebauter Sync-Server (Gegenstück zu web/src/pages/api/sync/), Token "tk_gut"
#[derive(Clone)]
struct Server {
    url: String,
    stand: Arc<Mutex<String>>,
    gueltig: Arc<AtomicBool>,
    streams: Arc<Mutex<Vec<TcpStream>>>,
    /// challenge aus der Verbinden-URL, gegen die /api/sync/token den verifier prüft
    challenge: Arc<Mutex<String>>,
}

impl Server {
    fn neu(stand: &str) -> Server {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let s = Server {
            url: format!("http://127.0.0.1:{}", l.local_addr().unwrap().port()),
            stand: Arc::new(Mutex::new(stand.into())),
            gueltig: Arc::new(AtomicBool::new(true)),
            streams: Arc::default(),
            challenge: Arc::default(),
        };
        let srv = s.clone();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let srv = srv.clone();
                std::thread::spawn(move || srv.bedienen(c));
            }
        });
        s
    }

    fn bedienen(&self, mut c: TcpStream) {
        let mut buf = vec![0u8; 4096];
        let mut n = 0;
        while !buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
            match c.read(&mut buf[n..]) {
                Ok(0) | Err(_) => return,
                Ok(k) => n += k,
            }
        }
        let anfrage = String::from_utf8_lossy(&buf[..n]).to_lowercase();
        if anfrage.starts_with("post /api/sync/token") {
            let roh = String::from_utf8_lossy(&buf[..n]).into_owned();
            let laenge: usize = anfrage.lines().find_map(|l| l.strip_prefix("content-length:")).map(|v| v.trim().parse().unwrap()).unwrap_or(0);
            let mut body = roh.split_once("\r\n\r\n").unwrap().1.as_bytes().to_vec();
            while body.len() < laenge {
                let mut b = [0u8; 1024];
                let k = c.read(&mut b).unwrap();
                body.extend_from_slice(&b[..k]);
            }
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let ok = v["code"] == "CODE1" && tippkraft::sync::challenge(v["verifier"].as_str().unwrap()) == *self.challenge.lock().unwrap();
            let (status, antwort) = if ok { ("200 OK", r#"{"token":"tk_gut"}"#) } else { ("400 Bad Request", r#"{"error":"Code ungültig oder abgelaufen."}"#) };
            let _ = c.write_all(format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{antwort}", antwort.len()).as_bytes());
            return;
        }
        if !self.gueltig.load(Ordering::SeqCst) || !anfrage.contains("authorization: bearer tk_gut") {
            let _ = c.write_all(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
            return;
        }
        let stand = self.stand.lock().unwrap().clone();
        if anfrage.starts_with("get /api/sync/snapshot") {
            let _ = c.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{stand}", stand.len()).as_bytes());
        } else if anfrage.starts_with("get /api/sync/stream") {
            let _ = c.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\nevent: snapshot\ndata: {stand}\n\n").as_bytes());
            self.streams.lock().unwrap().push(c);
        }
    }

    fn senden(&self, stand: &str) {
        *self.stand.lock().unwrap() = stand.into();
        self.streams.lock().unwrap().retain_mut(|c| c.write_all(format!("event: snapshot\ndata: {stand}\n\n").as_bytes()).is_ok());
    }

    fn trennen(&self) {
        for c in self.streams.lock().unwrap().drain(..) {
            let _ = c.shutdown(std::net::Shutdown::Both);
        }
    }
}

const STAND_ABC: &str = r#"{"version":1,"bausteine":[{"kuerzel":"abc","inhalt":"Neu","ausloeser":"leertaste","wortgrenze":true,"grossKleinBeachten":false,"schreibweiseUebernehmen":true}],"ausschluesse":[]}"#;

fn warten_bis(max: Duration, mut f: impl FnMut() -> bool) -> bool {
    let ende = Instant::now() + max;
    while Instant::now() < ende {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Startet `tippkraft verbinden` ohne echten Browser und liefert (Prozess, Port, state, challenge)
fn verbinden_starten(u: &[(String, String)], args: &[&str]) -> (Child, u16, String, String) {
    let mut kind = Command::new(env!("CARGO_BIN_EXE_tippkraft"))
        .arg("verbinden")
        .args(args)
        .envs(u.iter().map(|(a, b)| (a, b)))
        .env("TIPPKRAFT_BROWSER", "true")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(kind.stdout.take().unwrap()).lines();
    let url = out.find_map(|z| z.ok().filter(|z| z.contains("/connect-device?"))).expect("keine Verbinden-URL");
    std::thread::spawn(move || for _ in out {});
    let q: std::collections::HashMap<String, String> = url.split_once('?').unwrap().1.split('&').map(|p| p.split_once('=').unwrap()).map(|(a, b)| (a.into(), b.into())).collect();
    (kind, q["port"].parse().unwrap(), q["state"].clone(), q["challenge"].clone())
}

/// Ruft den Rückruf-Port wie ein Browser auf und liefert die Statuszeile
fn rueckruf(port: u16, pfad: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(format!("GET {pfad} HTTP/1.1\r\nhost: 127.0.0.1\r\n\r\n").as_bytes()).unwrap();
    let mut antwort = String::new();
    let _ = s.read_to_string(&mut antwort);
    antwort
}

fn fertig(mut kind: Child) -> (i32, String) {
    let ende = Instant::now() + Duration::from_secs(10);
    while kind.try_wait().unwrap().is_none() && Instant::now() < ende {
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut err = String::new();
    let _ = kind.stderr.take().unwrap().read_to_string(&mut err);
    (kind.wait().unwrap().code().unwrap_or(-1), err)
}

#[test]
fn verbinden_ueber_rueckruf_f12_ac13_ac14() {
    use std::os::unix::fs::PermissionsExt;
    let srv = Server::new_mfg();
    let u = umgebung("verbinden");
    let (kind, port, state, challenge) = verbinden_starten(&u, &["--server", &format!("{}/", srv.url)]);
    *srv.challenge.lock().unwrap() = challenge;
    assert!(rueckruf(port, "/favicon.ico").starts_with("HTTP/1.1 400"));
    assert!(rueckruf(port, "/fertig?code=CODE1&state=falsch").starts_with("HTTP/1.1 400"), "fremder state wird ignoriert");
    let antwort = rueckruf(port, &format!("/fertig?code=CODE1&state={state}"));
    assert!(antwort.contains("Verbunden. Du kannst dieses Fenster schließen."), "{antwort}");
    let (code, err) = fertig(kind);
    assert_eq!(code, 0, "{err}");
    let config = std::path::PathBuf::from(&u[0].1).join("tippkraft");
    let daten = std::path::PathBuf::from(&u[1].1).join("tippkraft");
    for (datei, modus) in [(config.join("zugang.json"), 0o600), (daten.join("stand.json"), 0o600), (config.clone(), 0o700), (daten.clone(), 0o700)] {
        assert_eq!(std::fs::metadata(&datei).unwrap().permissions().mode() & 0o777, modus, "{}", datei.display());
    }
    let zugang = std::fs::read_to_string(config.join("zugang.json")).unwrap();
    assert!(zugang.contains(&srv.url) && zugang.contains("tk_gut") && !zugang.contains(&format!("{}/", srv.url)));
    assert!(std::fs::read_to_string(daten.join("stand.json")).unwrap().contains("mfg"));

    // F12 AC-15: ohne --server wird die gespeicherte Adresse wiederverwendet
    let (kind, port, state, challenge) = verbinden_starten(&u, &[]);
    *srv.challenge.lock().unwrap() = challenge;
    rueckruf(port, &format!("/fertig?code=CODE1&state={state}"));
    assert_eq!(fertig(kind).0, 0);
}

#[test]
fn verbinden_abgebrochen_oder_falscher_code_f12_ac14() {
    let srv = Server::new_mfg();
    let u = umgebung("verbinden-ab");
    let (kind, port, state, _) = verbinden_starten(&u, &["--server", &srv.url]);
    rueckruf(port, &format!("/fertig?fehler=abgebrochen&state={state}"));
    let (code, err) = fertig(kind);
    assert_ne!(code, 0);
    assert!(err.contains("Verbinden abgebrochen"), "{err}");

    // challenge passt nicht zum verifier: Server lehnt ab, nichts gespeichert
    let (kind, port, state, _) = verbinden_starten(&u, &["--server", &srv.url]);
    *srv.challenge.lock().unwrap() = "andere".into();
    assert!(rueckruf(port, &format!("/fertig?code=CODE1&state={state}")).contains("Verbinden fehlgeschlagen"));
    let (code, err) = fertig(kind);
    assert_ne!(code, 0);
    assert!(err.contains("Code ungültig"), "{err}");
    assert!(!std::path::PathBuf::from(&u[0].1).join("tippkraft/zugang.json").exists());

    // http ohne localhost wird abgelehnt (F6 AC-3)
    let (code, out) = cli(&u, &["verbinden", "--server", "http://tippkraft.com"]);
    assert_ne!(code, 0);
    assert!(out.contains("https"), "{out}");
}

#[test]
fn login_gibt_es_nicht_mehr_f12_ac17() {
    let (code, out) = cli(&umgebung("login-weg"), &["login", "http://localhost:4321", "tk_x"]);
    assert_ne!(code, 0);
    assert!(out.contains("tippkraft connect"), "{out}");
}

impl Server {
    fn new_mfg() -> Server {
        Server::neu(STAND_MFG)
    }
}

/// Verbundenen Daemon gegen den Testserver starten
fn angemeldet(name: &str, srv: &Server) -> Lauf {
    let u = umgebung(name);
    sync_zugang_schreiben(&u, &srv.url, "tk_gut");
    starten_mit(name, &[], u, None)
}

fn status(l: &Lauf) -> String {
    cli(&l.umgebung, &["status"]).1
}

#[test]
fn sync_bringt_aenderungen_live_ac5_ac9() {
    let srv = Server::new_mfg();
    let mut l = angemeldet("sync", &srv);
    assert!(warten_bis(Duration::from_secs(3), || status(&l).contains("verbunden")), "{}", status(&l));
    let s = status(&l);
    assert!(s.contains("Zustand: aktiv") && s.contains("Bausteine: 1") && s.contains(&srv.url), "{s}");
    assert!(!s.contains("tk_gut"), "Token darf nie im Status stehen");

    tippen(&mut l, &[KeyCode::KEY_M, KeyCode::KEY_F, KeyCode::KEY_G, KeyCode::KEY_SPACE]);
    assert!(l.stdout.recv_timeout(Duration::from_secs(2)).unwrap().contains("Mit freundlichen"));

    srv.senden(STAND_ABC);
    std::thread::sleep(Duration::from_millis(500));
    tippen(&mut l, &[KeyCode::KEY_A, KeyCode::KEY_B, KeyCode::KEY_C, KeyCode::KEY_SPACE]);
    assert!(l.stdout.recv_timeout(Duration::from_secs(2)).unwrap().contains("Neu"));
    let stand = std::path::PathBuf::from(&l.umgebung[1].1).join("tippkraft/stand.json");
    assert!(std::fs::read_to_string(stand).unwrap().contains("abc"));
}

#[test]
fn verbindet_sich_nach_abbruch_neu_ac6() {
    let srv = Server::new_mfg();
    let mut l = angemeldet("neuverbinden", &srv);
    assert!(warten_bis(Duration::from_secs(3), || status(&l).contains("verbunden")));
    *srv.stand.lock().unwrap() = STAND_ABC.into();
    srv.trennen();
    std::thread::sleep(Duration::from_millis(2500));
    tippen(&mut l, &[KeyCode::KEY_A, KeyCode::KEY_B, KeyCode::KEY_C, KeyCode::KEY_SPACE]);
    assert!(l.stdout.recv_timeout(Duration::from_secs(2)).expect("kein neuer Stand nach Neuverbindung").contains("Neu"));
}

#[test]
fn offline_und_widerrufen_ac6_ac7() {
    let srv = Server::new_mfg();
    let l = angemeldet("widerrufen", &srv);
    assert!(warten_bis(Duration::from_secs(3), || status(&l).contains("verbunden")));
    srv.gueltig.store(false, Ordering::SeqCst);
    srv.trennen();
    assert!(warten_bis(Duration::from_secs(4), || status(&l).contains("Token ungültig")), "{}", status(&l));
    // Stand bleibt nutzbar
    assert!(status(&l).contains("Bausteine: 1"));

    let u = umgebung("offline");
    sync_zugang_schreiben(&u, "http://127.0.0.1:9", "tk_gut");
    let l2 = starten_mit("offline", &[], u, None);
    assert!(warten_bis(Duration::from_secs(3), || status(&l2).contains("offline")), "{}", status(&l2));
}

fn sync_zugang_schreiben(u: &[(String, String)], server: &str, token: &str) {
    let d = std::path::PathBuf::from(&u[0].1).join("tippkraft");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("zugang.json"), format!(r#"{{"server":"{server}","token":"{token}"}}"#)).unwrap();
}

#[test]
fn ohne_anmeldung_und_ohne_daemon_ac8_ac9() {
    let u = umgebung("ohne-daemon");
    let (code, out) = cli(&u, &["status"]);
    assert_eq!(code, 1);
    assert!(out.contains("läuft nicht"), "{out}");
    let l = starten("ohne-login", &[]);
    assert!(warten_bis(Duration::from_secs(3), || status(&l).contains("nicht angemeldet")), "{}", status(&l));
}

#[test]
fn pause_gibt_tastatur_frei_ac10() {
    let mut l = starten("pause", &[]);
    let (code, out) = cli(&l.umgebung, &["pause"]);
    assert_eq!((code, out.trim()), (0, "tippkraft pausiert"));
    {
        let mut d = Device::open(&l.pfad).unwrap();
        assert!(d.grab().is_ok(), "pausiert muss die Tastatur frei sein");
        d.ungrab().unwrap();
    }
    tippen(&mut l, &[KeyCode::KEY_M, KeyCode::KEY_F, KeyCode::KEY_G, KeyCode::KEY_SPACE]);
    assert!(l.stdout.recv_timeout(Duration::from_millis(500)).is_err(), "pausiert ersetzt nichts");
    assert!(status(&l).contains("pausiert"));

    let (_, out) = cli(&l.umgebung, &["pause"]);
    assert_eq!(out.trim(), "tippkraft aktiv");
    assert!(warten_bis(Duration::from_secs(2), || Device::open(&l.pfad).unwrap().grab().is_err()), "wieder übernommen");
    tippen(&mut l, &[KeyCode::KEY_M, KeyCode::KEY_F, KeyCode::KEY_G, KeyCode::KEY_SPACE]);
    assert!(l.stdout.recv_timeout(Duration::from_secs(2)).is_ok());
}

#[test]
fn steuer_socket_nur_fuer_den_nutzer() {
    use std::os::unix::fs::PermissionsExt;
    let l = starten("socketrechte", &[]);
    let socket = std::path::PathBuf::from(&l.umgebung[2].1).join("tippkraft.sock");
    assert_eq!(std::fs::metadata(socket).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn zweite_instanz_wird_abgelehnt_ac11() {
    let l = starten("instanz", &[]);
    let (code, out) = cli(&l.umgebung, &["run", "--trocken", "--geraet", l.pfad.to_str().unwrap()]);
    assert_ne!(code, 0);
    assert!(out.contains("läuft schon"), "{out}");
    assert!(status(&l).contains("aktiv"), "erste Instanz läuft weiter");
}

#[test]
fn einrichten_installiert_und_aktiviert_ac13() {
    let home = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("home-einrichten");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let log = home.join("systemctl.log");
    let fake = home.join("systemctl");
    std::fs::write(&fake, format!("#!/bin/sh\necho \"$@\" >> '{}'\n", log.display())).unwrap();
    std::os::unix::fs::PermissionsExt::set_mode(&mut std::fs::metadata(&fake).unwrap().permissions(), 0o755);
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // schon verbunden: einrichten startet kein verbinden
    std::fs::create_dir_all(home.join(".config/tippkraft")).unwrap();
    std::fs::write(home.join(".config/tippkraft/zugang.json"), r#"{"server":"http://localhost:4321","token":"tk_x"}"#).unwrap();
    for _ in 0..2 {
        let out = Command::new(env!("CARGO_BIN_EXE_tippkraft"))
            .arg("einrichten")
            .env("HOME", &home)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .env("TIPPKRAFT_SYSTEMCTL", &fake)
            .env("TIPPKRAFT_SPRACHE", "de")
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("Schon verbunden"));
    }
    let start = std::fs::read_to_string(home.join(".local/share/applications/tippkraft-starten.desktop")).unwrap();
    assert!(start.contains("Name=Start tippkraft") && start.contains("Name[de]=tippkraft starten") && start.contains("Exec=systemctl --user start tippkraft.service"), "F13 AC-8");
    let starter = std::fs::read_to_string(home.join(".local/share/applications/tippkraft-verbinden.desktop")).unwrap();
    assert!(starter.contains("Name[de]=tippkraft verbinden") && starter.contains(&format!("Exec={} connect", home.join(".local/bin/tippkraft").display())));
    assert!(home.join(".local/bin/tippkraft").exists());
    let unit = std::fs::read_to_string(home.join(".config/systemd/user/tippkraft.service")).unwrap();
    assert!(unit.contains("ExecStart=%h/.local/bin/tippkraft run") && unit.contains("Restart=on-failure") && unit.contains("WantedBy=graphical-session.target"));
    let aufrufe = std::fs::read_to_string(log).unwrap();
    assert_eq!(aufrufe.lines().collect::<Vec<_>>(), ["--user daemon-reload", "--user enable tippkraft.service", "--user restart tippkraft.service"].repeat(2));
}

#[test]
fn einrichten_ohne_verbindung_startet_verbinden_f12_ac16() {
    let srv = Server::new_mfg();
    let home = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("home-einrichten-neu");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let fake = home.join("systemctl");
    std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // Browser-Ersatz merkt sich die URL, der Test bricht dann wie ein Nutzer ab
    let url_datei = home.join("url");
    let browser = home.join("browser");
    std::fs::write(&browser, format!("#!/bin/sh\necho \"$1\" > '{}'\n", url_datei.display())).unwrap();
    std::fs::set_permissions(&browser, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut kind = Command::new(env!("CARGO_BIN_EXE_tippkraft"))
        .arg("einrichten")
        .env("HOME", &home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env("TIPPKRAFT_SYSTEMCTL", &fake)
        .env("TIPPKRAFT_SPRACHE", "de")
        .env("TIPPKRAFT_BROWSER", &browser)
        .env("TIPPKRAFT_KEINE_BENACHRICHTIGUNG", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    assert!(warten_bis(Duration::from_secs(5), || url_datei.exists()), "einrichten hat den Browser nicht geöffnet");
    let url = std::fs::read_to_string(&url_datei).unwrap();
    assert!(url.starts_with("https://tippkraft.com/connect-device?"), "Standard-Server: {url}");
    let q: std::collections::HashMap<&str, &str> = url.trim().split_once('?').unwrap().1.split('&').map(|p| p.split_once('=').unwrap()).collect();
    rueckruf(q["port"].parse().unwrap(), &format!("/fertig?fehler=abgebrochen&state={}", q["state"]));
    let (code, err) = fertig(kind_take(&mut kind));
    assert_ne!(code, 0);
    assert!(err.contains("abgebrochen"), "{err}");
    let _ = srv;
}

fn kind_take(k: &mut Child) -> Child {
    std::mem::replace(k, Command::new("true").spawn().unwrap())
}

/// Gegen den echten Webserver (Docker), nur mit TIPPKRAFT_ECHT_URL, _TOKEN und _COOKIE:
///   TIPPKRAFT_ECHT_URL=http://localhost:4321 ... cargo test --test integration echter_server -- --ignored
#[test]
#[ignore]
fn echter_server() {
    let url = std::env::var("TIPPKRAFT_ECHT_URL").unwrap();
    let token = std::env::var("TIPPKRAFT_ECHT_TOKEN").unwrap();
    let cookie = std::env::var("TIPPKRAFT_ECHT_COOKIE").unwrap();
    let u = umgebung("echt");
    sync_zugang_schreiben(&u, &url, &token);
    let mut l = starten_mit("echt", &[], u, None);
    assert!(warten_bis(Duration::from_secs(3), || status(&l).contains("verbunden")), "{}", status(&l));
    // Baustein über die Web-API anlegen, er muss per SSE ankommen
    let out = Command::new("curl")
        .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "-b", &cookie, "-H", &format!("Origin: {url}"), "-H", "content-type: application/json"])
        .args(["-d", r#"{"kuerzel":"abc","inhalt":"Vom Server"}"#, &format!("{url}/api/bausteine")])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "201");
    std::thread::sleep(Duration::from_millis(1500));
    tippen(&mut l, &[KeyCode::KEY_A, KeyCode::KEY_B, KeyCode::KEY_C, KeyCode::KEY_SPACE]);
    assert!(l.stdout.recv_timeout(Duration::from_secs(2)).unwrap().contains("Vom Server"));
}


// ---------- F8: Popup ----------

const STAND_BRIEF: &str = r#"{"version":1,"bausteine":[{"kuerzel":"brief","inhalt":"{{?Anrede:Herr|Frau}} {{?Name}}, {{?Name}}","ausloeser":"leertaste","wortgrenze":true,"grossKleinBeachten":false,"schreibweiseUebernehmen":true}],"ausschluesse":[]}"#;

/// Ersatz für das Popup-Fenster: antwortet mit festen Werten (Exit 0) oder bricht ab (Exit 1)
fn popup_ersatz(name: &str, antwort: Option<&str>) -> String {
    let p = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("popup-{name}.sh"));
    let inhalt = match antwort {
        Some(json) => format!("#!/bin/sh\ncat > '{}.anfrage'\necho '{json}'\n", p.display()),
        None => format!("#!/bin/sh\ncat > '{}.anfrage'\nexit 1\n", p.display()),
    };
    std::fs::write(&p, inhalt).unwrap();
    std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    p.to_string_lossy().into_owned()
}

#[test]
fn popup_fuellt_aus_f8_ac6_ac7() {
    let popup = popup_ersatz("ok", Some(r#"{"werte":{"Anrede":"Frau","Name":"Muster"}}"#));
    let mut l = starten_mit("popup-ok", &[("TIPPKRAFT_POPUP", &popup)], umgebung("popup-ok"), Some(STAND_BRIEF));
    tippen(&mut l, &[KeyCode::KEY_B, KeyCode::KEY_R, KeyCode::KEY_I, KeyCode::KEY_E, KeyCode::KEY_F, KeyCode::KEY_SPACE]);
    let zeile = l.stdout.recv_timeout(Duration::from_secs(3)).expect("keine Ausgabe nach dem Popup");
    assert_eq!(zeile, r#"ersetzen loeschen=5 [Text("Frau Muster, Muster")]"#);
    let anfrage: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(format!("{popup}.anfrage")).unwrap()).unwrap();
    assert_eq!(anfrage["titel"], "brief");
    assert!(anfrage["datum"].as_str().unwrap().len() == 10);
}

#[test]
fn popup_abbrechen_tippt_den_ausloeser_nach_f8_ac9() {
    let popup = popup_ersatz("ab", None);
    let mut l = starten_mit("popup-ab", &[("TIPPKRAFT_POPUP", &popup)], umgebung("popup-ab"), Some(STAND_BRIEF));
    tippen(&mut l, &[KeyCode::KEY_B, KeyCode::KEY_R, KeyCode::KEY_I, KeyCode::KEY_E, KeyCode::KEY_F, KeyCode::KEY_SPACE]);
    assert_eq!(l.stdout.recv_timeout(Duration::from_secs(3)).unwrap(), r#"ersetzen loeschen=0 [Text(" ")]"#);
}


// ---------- F13: Tray ----------

#[test]
fn tray_meldet_sich_an_und_linksklick_pausiert_f13_ac1_ac4() {
    let l = starten("tray", &[("TIPPKRAFT_TRAY", "1")]);
    let pid = l.kind.id();
    let name = format!("org.kde.StatusNotifierItem-{pid}-");
    let gefunden = || {
        let out = Command::new("busctl").args(["--user", "list", "--no-legend"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).lines().find_map(|z| z.split_whitespace().next().filter(|n| n.starts_with(&name)).map(String::from))
    };
    assert!(warten_bis(Duration::from_secs(3), || gefunden().is_some()), "kein StatusNotifierItem auf dem Session-Bus");
    let dienst = gefunden().unwrap();
    let out = Command::new("busctl").args(["--user", "call", &dienst, "/StatusNotifierItem", "org.kde.StatusNotifierItem", "Activate", "ii", "0", "0"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(warten_bis(Duration::from_secs(2), || status(&l).contains("pausiert")), "{}", status(&l));
    // Tooltip zeigt die Statuszeile
    let tip = Command::new("busctl").args(["--user", "get-property", &dienst, "/StatusNotifierItem", "org.kde.StatusNotifierItem", "ToolTip"]).output().unwrap();
    assert!(String::from_utf8_lossy(&tip.stdout).contains("Pausiert"), "{}", String::from_utf8_lossy(&tip.stdout));
}


// ---------- F15: Sprache ----------

#[test]
fn sprache_aus_dem_stand_f15_ac4_ac8() {
    // Stand sagt Englisch, obwohl die Umgebung Deutsch vorgibt: Der Account gewinnt
    let stand = r#"{"version":2,"sprache":"en","bausteine":[],"ausschluesse":[]}"#;
    let mut l = starten_mit("sprache-en", &[], umgebung("sprache-en"), Some(stand));
    let s = status(&l);
    assert!(s.starts_with("State: active · Connection: not connected · Snippets: 0"), "{s}");
    let (_, out) = cli(&l.umgebung, &["pause"]);
    assert_eq!(out.trim(), "tippkraft paused");
    let _ = &mut l;
    // Hilfe und englische Befehlsnamen
    let mut u = umgebung("sprache-hilfe");
    u.retain(|(k, _)| k != "TIPPKRAFT_SPRACHE");
    u.push(("TIPPKRAFT_SPRACHE".into(), "en".into()));
    let (_, hilfe) = cli(&u, &[]);
    assert!(hilfe.contains("tippkraft connect") && hilfe.contains("Emergency stop"), "{hilfe}");
    let (code, out) = cli(&u, &["status"]);
    assert_eq!((code, out.trim()), (1, "tippkraft: tippkraft is not running"));
}
