//! Anmeldung, lokale Dateien und SSE-Sync mit dem Server (Gegenstück zu web/src/pages/api/sync/).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::format::{self, Stand};

/// Wo tippkraft seine Dateien ablegt (XDG, für Tests per Umgebung umlenkbar)
pub struct Pfade {
    pub zugang: PathBuf,
    pub stand: PathBuf,
    pub socket: PathBuf,
}

impl Pfade {
    pub fn aus_umgebung() -> Self {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let xdg = |var: &str, fallback: &str| std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| home.join(fallback));
        let laufzeit = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        Pfade {
            zugang: xdg("XDG_CONFIG_HOME", ".config").join("tippkraft/zugang.json"),
            stand: xdg("XDG_DATA_HOME", ".local/share").join("tippkraft/stand.json"),
            socket: laufzeit.join("tippkraft.sock"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Zugang {
    pub server: String,
    pub token: String,
}

impl std::fmt::Debug for Zugang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Zugang({})", self.server)
    }
}

impl Zugang {
    pub fn laden(pfad: &Path) -> Option<Zugang> {
        serde_json::from_str(&std::fs::read_to_string(pfad).ok()?).ok()
    }
}

/// Schreibt atomar (temporäre Datei + rename), Datei 0600, Ordner 0700
pub fn privat_schreiben(pfad: &Path, inhalt: &[u8]) -> std::io::Result<()> {
    let dir = pfad.parent().unwrap_or(Path::new("."));
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let tmp = pfad.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(inhalt)?;
    f.sync_all()?;
    std::fs::rename(tmp, pfad)
}

/// Normalisiert die Server-URL. http nur für localhost, sonst ginge der Token unverschlüsselt übers Netz.
pub fn server_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    let (schema, rest) = url.split_once("://").ok_or("URL braucht http:// oder https://")?;
    let host = rest.split(['/', ':']).next().unwrap_or_default();
    match schema {
        "https" if !host.is_empty() => Ok(url.to_string()),
        "http" if host == "localhost" || host == "127.0.0.1" => Ok(url.to_string()),
        "http" => Err("Bitte https verwenden, sonst geht der Token unverschlüsselt übers Netz".into()),
        _ => Err("URL braucht http:// oder https://".into()),
    }
}

pub fn naechster_backoff(sekunden: u64) -> u64 {
    (sekunden * 2).min(60)
}

/// Zerlegt Server-Sent Events zeilenweise. Liefert (event, data), sobald ein Ereignis komplett ist.
#[derive(Default)]
pub struct Sse {
    event: String,
    data: Vec<String>,
}

impl Sse {
    pub fn zeile(&mut self, z: &str) -> Option<(String, String)> {
        if z.is_empty() {
            let event = std::mem::take(&mut self.event);
            let data = std::mem::take(&mut self.data);
            return (!data.is_empty()).then(|| (if event.is_empty() { "message".into() } else { event }, data.join("\n")));
        }
        if let Some(v) = z.strip_prefix("event:") {
            self.event = v.trim_start().to_string();
        } else if let Some(v) = z.strip_prefix("data:") {
            self.data.push(v.strip_prefix(' ').unwrap_or(v).to_string());
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verbindung {
    NichtAngemeldet,
    Verbunden,
    Offline,
    TokenUngueltig,
    ServerNeuer,
}

impl std::fmt::Display for Verbindung {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Verbindung::NichtAngemeldet => "nicht angemeldet",
            Verbindung::Verbunden => "verbunden",
            Verbindung::Offline => "offline, versuche es weiter",
            Verbindung::TokenUngueltig => "Token ungültig, bitte neu anmelden",
            Verbindung::ServerNeuer => "Server neuer als tippkraft, bitte aktualisieren",
        })
    }
}

enum Fehler {
    TokenUngueltig,
    Netz(String),
}

fn anfrage(z: &Zugang, pfad: &str, body_timeout: Option<Duration>) -> Result<ureq::http::Response<ureq::Body>, Fehler> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(15)))
        .timeout_recv_body(body_timeout)
        .build()
        .into();
    agent.get(format!("{}{pfad}", z.server)).header("Authorization", format!("Bearer {}", z.token)).call().map_err(|e| match e {
        ureq::Error::StatusCode(401) => Fehler::TokenUngueltig,
        e => Fehler::Netz(e.to_string()),
    })
}

/// Prüft den Token am Server, speichert dann Zugang und Stand. Liefert die Anzahl Bausteine.
pub fn zugang_speichern(pfade: &Pfade, z: &Zugang) -> Result<usize, String> {
    let mut res = anfrage(z, "/api/sync/snapshot", Some(Duration::from_secs(30))).map_err(|e| match e {
        Fehler::TokenUngueltig => "Token ungültig oder widerrufen".to_string(),
        Fehler::Netz(e) => format!("Server nicht erreichbar: {e}"),
    })?;
    let json = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let stand = format::parse(&json)?;
    privat_schreiben(&pfade.zugang, serde_json::to_string_pretty(z).unwrap().as_bytes()).map_err(|e| e.to_string())?;
    privat_schreiben(&pfade.stand, json.as_bytes()).map_err(|e| e.to_string())?;
    Ok(stand.bausteine.len())
}

// ---------- Gerät per Browser verbinden (F12): Rückruf an 127.0.0.1 mit PKCE (RFC 8252/7636) ----------

pub const STANDARD_SERVER: &str = "https://tippkraft.com";
const VERBINDEN_TIMEOUT: Duration = Duration::from_secs(300);

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn zufall() -> String {
    let mut buf = [0u8; 32];
    rustix::rand::getrandom(&mut buf, rustix::rand::GetRandomFlags::empty()).expect("kein Zufall verfügbar");
    b64(&buf)
}

pub fn challenge(verifier: &str) -> String {
    use sha2::Digest;
    b64(&sha2::Sha256::digest(verifier.as_bytes()))
}

/// (verifier, challenge)
pub fn pkce() -> (String, String) {
    let v = zufall();
    let c = challenge(&v);
    (v, c)
}

fn url_kodieren(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn url_dekodieren(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())) {
            (b'%', Some(x)) => {
                out.push(x);
                i += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                i += 1;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn verbinden_url(server: &str, port: u16, state: &str, challenge: &str, name: &str) -> String {
    format!("{server}/geraet-verbinden?port={port}&state={}&challenge={}&name={}", url_kodieren(state), url_kodieren(challenge), url_kodieren(name))
}

pub fn rechnername() -> String {
    let n = rustix::system::uname().nodename().to_string_lossy().into_owned();
    // "fedora.fritz.box" → "fedora"
    let n: String = n.split('.').next().unwrap_or_default().chars().filter(|c| !c.is_control()).take(50).collect();
    if n.is_empty() { "Desktop".into() } else { n }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Rueckruf {
    Code(String),
    Abgebrochen,
    /// anderer Pfad oder falscher state: ignorieren und weiter warten
    Fremd,
}

/// Wertet die erste Zeile einer HTTP-Anfrage an den Rückruf-Port aus
pub fn rueckruf(anfragezeile: &str, state: &str) -> Rueckruf {
    let pfad = anfragezeile.split(' ').nth(1).unwrap_or_default();
    let Some(query) = pfad.strip_prefix("/fertig?") else { return Rueckruf::Fremd };
    let mut werte = std::collections::HashMap::new();
    for paar in query.split('&') {
        if let Some((k, v)) = paar.split_once('=') {
            werte.insert(k, url_dekodieren(v));
        }
    }
    if werte.get("state").map(String::as_str) != Some(state) {
        return Rueckruf::Fremd;
    }
    match (werte.remove("code"), werte.get("fehler")) {
        (_, Some(_)) => Rueckruf::Abgebrochen,
        (Some(c), None) if !c.is_empty() => Rueckruf::Code(c),
        _ => Rueckruf::Fremd,
    }
}

fn antworten(mut s: &std::net::TcpStream, status: &str, text: &str) {
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>tippkraft</title><body style=\"font:16px system-ui;max-width:28rem;margin:15vh auto;padding:0 1rem\"><h1 style=\"color:#d9480f\">tippkraft</h1><p>{text}</p>"
    );
    let _ = s.write_all(format!("HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{html}", html.len()).as_bytes());
}

fn code_einloesen(server: &str, code: &str, verifier: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Antwort {
        token: String,
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).http_status_as_error(false).build().into();
    let mut res = agent
        .post(format!("{server}/api/sync/token"))
        .header("content-type", "application/json")
        .send(serde_json::json!({ "code": code, "verifier": verifier }).to_string())
        .map_err(|e| format!("Server nicht erreichbar: {e}"))?;
    let text = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if res.status() != 200 {
        let msg = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["error"].as_str().map(String::from));
        return Err(msg.unwrap_or_else(|| format!("Server antwortet {}", res.status())));
    }
    serde_json::from_str::<Antwort>(&text).map(|a| a.token).map_err(|e| e.to_string())
}

/// `tippkraft verbinden`: Browser öffnen, auf den Rückruf warten, Code gegen Token tauschen, speichern.
/// Liefert (Server, Anzahl Bausteine).
pub fn verbinden(pfade: &Pfade, server: Option<&str>, browser: &str) -> Result<(String, usize), String> {
    let server = server_url(&server.map(String::from).or_else(|| Zugang::laden(&pfade.zugang).map(|z| z.server)).unwrap_or_else(|| STANDARD_SERVER.into()))?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let state = zufall();
    let (verifier, ch) = pkce();
    let url = verbinden_url(&server, port, &state, &ch, &rechnername());
    println!("Browser öffnet sich. Falls nicht, diese Adresse öffnen:\n{url}");
    let _ = std::process::Command::new(browser).arg(&url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();

    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let ende = std::time::Instant::now() + VERBINDEN_TIMEOUT;
    while std::time::Instant::now() < ende {
        let Ok((s, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        let _ = s.set_nonblocking(false);
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let mut zeile = String::new();
        if BufReader::new(&s).read_line(&mut zeile).is_err() {
            continue;
        }
        match rueckruf(zeile.trim_end(), &state) {
            Rueckruf::Fremd => antworten(&s, "400 Bad Request", "Unbekannte Anfrage."),
            Rueckruf::Abgebrochen => {
                antworten(&s, "200 OK", "Verbinden abgebrochen. Du kannst dieses Fenster schließen.");
                return Err("Verbinden abgebrochen".into());
            }
            Rueckruf::Code(code) => {
                let ergebnis = code_einloesen(&server, &code, &verifier).and_then(|token| zugang_speichern(pfade, &Zugang { server: server.clone(), token }));
                match &ergebnis {
                    Ok(_) => antworten(&s, "200 OK", "Verbunden. Du kannst dieses Fenster schließen."),
                    Err(e) => antworten(&s, "200 OK", &format!("Verbinden fehlgeschlagen: {e}")),
                }
                return ergebnis.map(|n| (server, n));
            }
        }
    }
    Err("Keine Rückmeldung vom Browser innerhalb von 5 Minuten".into())
}

pub enum Meldung {
    Verbindung(Verbindung),
    Stand(Stand),
}

/// Läuft im Daemon für immer: verbindet sich, schreibt jeden neuen Stand nach `stand.json` und meldet ihn.
pub fn schleife(pfade: &Pfade, mut melden: impl FnMut(Meldung)) {
    let mut backoff = 1;
    loop {
        let Some(z) = Zugang::laden(&pfade.zugang) else {
            melden(Meldung::Verbindung(Verbindung::NichtAngemeldet));
            std::thread::sleep(Duration::from_secs(5));
            continue;
        };
        // ponytail: ureq kennt kein Timeout pro Lesevorgang. Neuaufbau alle 2 Minuten erkennt still getrennte Verbindungen.
        match anfrage(&z, "/api/sync/stream", Some(Duration::from_secs(120))) {
            Err(Fehler::TokenUngueltig) => {
                melden(Meldung::Verbindung(Verbindung::TokenUngueltig));
                // kurz, damit ein neues "tippkraft verbinden" schnell greift
                std::thread::sleep(Duration::from_secs(10));
            }
            Err(Fehler::Netz(_)) => {
                melden(Meldung::Verbindung(Verbindung::Offline));
                std::thread::sleep(Duration::from_secs(backoff));
                backoff = naechster_backoff(backoff);
            }
            Ok(mut res) => {
                let mut sse = Sse::default();
                let mut erstes = true;
                for zeile in BufReader::new(res.body_mut().as_reader()).lines() {
                    let Ok(zeile) = zeile else { break };
                    let Some((event, data)) = sse.zeile(&zeile) else { continue };
                    if event != "snapshot" {
                        continue;
                    }
                    match format::parse(&data) {
                        Ok(stand) => {
                            if let Err(e) = privat_schreiben(&pfade.stand, data.as_bytes()) {
                                eprintln!("tippkraft: Stand nicht speicherbar: {e}");
                            }
                            melden(Meldung::Stand(stand));
                            // erst nach dem Stand, damit der Status nie "verbunden" mit altem Stand zeigt
                            if erstes {
                                erstes = false;
                                backoff = 1;
                                melden(Meldung::Verbindung(Verbindung::Verbunden));
                            }
                        }
                        Err(_) => {
                            erstes = false;
                            melden(Meldung::Verbindung(Verbindung::ServerNeuer));
                        }
                    }
                }
                if erstes {
                    melden(Meldung::Verbindung(Verbindung::Offline));
                    std::thread::sleep(Duration::from_secs(backoff));
                    backoff = naechster_backoff(backoff);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parser_liefert_snapshot_ereignisse_und_ignoriert_pings() {
        let mut p = Sse::default();
        assert_eq!(p.zeile("event: snapshot"), None);
        assert_eq!(p.zeile("data: {\"version\":1}"), None);
        assert_eq!(p.zeile(""), Some(("snapshot".into(), "{\"version\":1}".into())));
        assert_eq!(p.zeile(": ping"), None);
        assert_eq!(p.zeile(""), None, "Ping ohne Daten ist kein Ereignis");
        // mehrzeilige data-Felder werden mit \n verbunden
        p.zeile("event: snapshot");
        p.zeile("data: a");
        p.zeile("data: b");
        assert_eq!(p.zeile(""), Some(("snapshot".into(), "a\nb".into())));
    }

    #[test]
    fn url_pruefung() {
        assert_eq!(server_url("https://tippkraft.com/").unwrap(), "https://tippkraft.com");
        assert_eq!(server_url("http://localhost:4321").unwrap(), "http://localhost:4321");
        assert_eq!(server_url("http://127.0.0.1:4321/").unwrap(), "http://127.0.0.1:4321");
        assert!(server_url("http://tippkraft.com").unwrap_err().contains("https"));
        assert!(server_url("http://localhost.evil.example").is_err());
        assert!(server_url("ftp://x").is_err());
    }

    #[test]
    fn backoff_waechst_bis_60() {
        let mut b = 1;
        let folge: Vec<u64> = (0..8).map(|_| { let x = b; b = naechster_backoff(b); x }).collect();
        assert_eq!(folge, [1, 2, 4, 8, 16, 32, 60, 60]);
    }

    #[test]
    fn dateien_sind_nur_fuer_den_nutzer_lesbar() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("tippkraft-perm-{}", std::process::id()));
        let datei = dir.join("unter/zugang.json");
        privat_schreiben(&datei, b"{}").unwrap();
        assert_eq!(std::fs::metadata(&datei).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(datei.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pkce_challenge_ist_base64url_sha256() {
        // SHA-256("abc"), base64url ohne Padding
        assert_eq!(challenge("abc"), "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0");
        let (v, c) = pkce();
        assert_eq!(v.len(), 43);
        assert_eq!(c, challenge(&v));
        assert_ne!(pkce().0, v, "zufällig");
    }

    #[test]
    fn verbinden_url_kodiert_den_namen() {
        assert_eq!(
            verbinden_url("http://localhost:4321", 45123, "S", "C", "mein rechner/ä"),
            "http://localhost:4321/geraet-verbinden?port=45123&state=S&challenge=C&name=mein%20rechner%2F%C3%A4"
        );
    }

    #[test]
    fn rueckruf_auswerten() {
        assert_eq!(rueckruf("GET /fertig?code=abc&state=S HTTP/1.1", "S"), Rueckruf::Code("abc".into()));
        assert_eq!(rueckruf("GET /fertig?state=S&fehler=abgebrochen HTTP/1.1", "S"), Rueckruf::Abgebrochen);
        assert_eq!(rueckruf("GET /fertig?code=abc&state=X HTTP/1.1", "S"), Rueckruf::Fremd);
        assert_eq!(rueckruf("GET /favicon.ico HTTP/1.1", "S"), Rueckruf::Fremd);
        assert_eq!(rueckruf("GET /fertig?code=a%2Db&state=S HTTP/1.1", "S"), Rueckruf::Code("a-b".into()));
    }

    #[test]
    fn rechnername_ist_gesetzt_und_kurz() {
        let n = rechnername();
        assert!(!n.is_empty() && n.chars().count() <= 50 && !n.contains('.'));
    }

    #[test]
    fn zugang_debug_zeigt_keinen_token() {
        let z = Zugang { server: "https://x".into(), token: "tk_geheim".into() };
        assert!(!format!("{z:?}").contains("geheim"));
    }
}
