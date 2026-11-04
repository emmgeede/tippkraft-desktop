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
    for k in [KeyCode::KEY_A, KeyCode::KEY_B, KeyCode::KEY_C, KeyCode::KEY_Z, KeyCode::KEY_M, KeyCode::KEY_F, KeyCode::KEY_G, KeyCode::KEY_ENTER, KeyCode::KEY_SPACE, KeyCode::KEY_BACKSPACE, KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTALT] {
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
}

impl Server {
    fn neu(stand: &str) -> Server {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let s = Server {
            url: format!("http://127.0.0.1:{}", l.local_addr().unwrap().port()),
            stand: Arc::new(Mutex::new(stand.into())),
            gueltig: Arc::new(AtomicBool::new(true)),
            streams: Arc::default(),
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

#[test]
fn login_speichert_privat_ac1_ac4() {
    use std::os::unix::fs::PermissionsExt;
    let srv = Server::new_mfg();
    let u = umgebung("login-ok");
    let (code, out) = cli(&u, &["login", &format!("{}/", srv.url), "tk_gut"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.trim(), format!("Angemeldet bei {}, 1 Bausteine", srv.url));
    let config = std::path::PathBuf::from(&u[0].1).join("tippkraft");
    let daten = std::path::PathBuf::from(&u[1].1).join("tippkraft");
    for (datei, modus) in [(config.join("zugang.json"), 0o600), (daten.join("stand.json"), 0o600), (config.clone(), 0o700), (daten.clone(), 0o700)] {
        assert_eq!(std::fs::metadata(&datei).unwrap().permissions().mode() & 0o777, modus, "{}", datei.display());
    }
    assert!(std::fs::read_to_string(daten.join("stand.json")).unwrap().contains("mfg"));
    assert!(std::fs::read_to_string(config.join("zugang.json")).unwrap().contains(&srv.url));
}

#[test]
fn login_fehler_speichert_nichts_ac2_ac3() {
    let srv = Server::new_mfg();
    let u = umgebung("login-fehler");
    let (code, out) = cli(&u, &["login", &srv.url, "tk_falsch"]);
    assert_ne!(code, 0);
    assert!(out.contains("Token ungültig"), "{out}");
    let (code, out) = cli(&u, &["login", "http://127.0.0.1:9", "tk_gut"]);
    assert_ne!(code, 0);
    assert!(out.contains("nicht erreichbar"), "{out}");
    let (code, out) = cli(&u, &["login", "http://tippkraft.com", "tk_gut"]);
    assert_ne!(code, 0);
    assert!(out.contains("https"), "{out}");
    assert!(!std::path::PathBuf::from(&u[0].1).join("tippkraft/zugang.json").exists());
}

impl Server {
    fn new_mfg() -> Server {
        Server::neu(STAND_MFG)
    }
}

/// Angemeldeten Daemon gegen den Testserver starten
fn angemeldet(name: &str, srv: &Server) -> Lauf {
    let u = umgebung(name);
    assert_eq!(cli(&u, &["login", &srv.url, "tk_gut"]).0, 0);
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
    sync_zugang_schreiben(&u, "http://127.0.0.1:9");
    let l2 = starten_mit("offline", &[], u, None);
    assert!(warten_bis(Duration::from_secs(3), || status(&l2).contains("offline")), "{}", status(&l2));
}

fn sync_zugang_schreiben(u: &[(String, String)], server: &str) {
    let d = std::path::PathBuf::from(&u[0].1).join("tippkraft");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("zugang.json"), format!(r#"{{"server":"{server}","token":"tk_gut"}}"#)).unwrap();
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
    for _ in 0..2 {
        let out = Command::new(env!("CARGO_BIN_EXE_tippkraft")).arg("einrichten").env("HOME", &home).env("TIPPKRAFT_SYSTEMCTL", &fake).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
    assert!(home.join(".local/bin/tippkraft").exists());
    let unit = std::fs::read_to_string(home.join(".config/systemd/user/tippkraft.service")).unwrap();
    assert!(unit.contains("ExecStart=%h/.local/bin/tippkraft run") && unit.contains("Restart=on-failure") && unit.contains("WantedBy=graphical-session.target"));
    let aufrufe = std::fs::read_to_string(log).unwrap();
    assert_eq!(aufrufe.lines().collect::<Vec<_>>(), ["--user daemon-reload", "--user enable tippkraft.service", "--user restart tippkraft.service"].repeat(2));
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
    let (code, out) = cli(&u, &["login", &url, &token]);
    assert_eq!(code, 0, "{out}");
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
