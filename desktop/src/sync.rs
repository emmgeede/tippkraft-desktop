//! Anmeldung, lokale Dateien und SSE-Sync mit dem Server (Gegenstück zu web/src/pages/api/sync/).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::format::{self, Stand};
use crate::i18n::{T, f, tr};

/// Wo tippkraft seine Dateien ablegt (XDG, für Tests per Umgebung umlenkbar)
pub struct Pfade {
    pub zugang: PathBuf,
    /// #196: alle verbundenen Konten dieses Rechners, `zugang` ist das aktive
    pub konten: PathBuf,
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
            konten: xdg("XDG_CONFIG_HOME", ".config").join("tippkraft/konten.json"),
            stand: xdg("XDG_DATA_HOME", ".local/share").join("tippkraft/stand.json"),
            socket: laufzeit.join("tippkraft.sock"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Zugang {
    pub server: String,
    pub token: String,
    /// #196: E-Mail des Kontos (ältere Server liefern sie nicht)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
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
    /// Anzeigename im Menü und in `tippkraft account`
    pub fn name(&self) -> String {
        if self.email.is_empty() { self.server.clone() } else { format!("{} ({})", self.email, self.server.trim_start_matches("https://")) }
    }
    fn gleich(&self, o: &Zugang) -> bool {
        self.server == o.server && (self.token == o.token || (!self.email.is_empty() && self.email == o.email))
    }
}

// ---------- #196: mehrere Konten ----------

/// Gespeicherte Konten. Ohne `konten.json` (ältere Version) zählt das aktive Konto als einziges (AC-7).
pub fn konten_laden(pfade: &Pfade) -> Vec<Zugang> {
    let mut k: Vec<Zugang> = std::fs::read_to_string(&pfade.konten).ok().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    if let Some(a) = Zugang::laden(&pfade.zugang).filter(|a| !k.iter().any(|x| x.gleich(a))) {
        k.insert(0, a);
    }
    k
}

fn konten_schreiben(pfade: &Pfade, k: &[Zugang]) -> Result<(), String> {
    privat_schreiben(&pfade.konten, serde_json::to_string_pretty(k).unwrap().as_bytes()).map_err(|e| e.to_string())
}

/// Konto in die Liste aufnehmen oder dort ersetzen (neuer Token für dasselbe Konto)
pub fn konto_merken(pfade: &Pfade, z: &Zugang) -> Result<(), String> {
    let mut k = konten_laden(pfade);
    match k.iter_mut().find(|x| x.gleich(z)) {
        Some(x) => *x = z.clone(),
        None => k.push(z.clone()),
    }
    konten_schreiben(pfade, &k)
}

/// Index des aktiven Kontos in `konten`
pub fn aktiv_index(pfade: &Pfade, konten: &[Zugang]) -> Option<usize> {
    let a = Zugang::laden(&pfade.zugang)?;
    konten.iter().position(|x| x.token == a.token)
}

/// `1`, `2`, … oder E-Mail → Index
pub fn konto_finden(konten: &[Zugang], wahl: &str) -> Option<usize> {
    match wahl.parse::<usize>() {
        Ok(n) if n >= 1 && n <= konten.len() => Some(n - 1),
        Ok(_) => None,
        Err(_) => konten.iter().position(|k| k.email.eq_ignore_ascii_case(wahl)),
    }
}

/// Wechsel: holt erst den Stand des Kontos (prüft so den Token), schreibt dann Zugang und Stand.
/// Schlägt das fehl, bleibt das bisherige Konto aktiv (AC-5). Der Dienst lädt den Stand selbst nach.
pub fn konto_wechseln(pfade: &Pfade, z: &Zugang) -> Result<usize, String> {
    zugang_speichern(pfade, z)
}

/// Entfernt ein nicht aktives Konto lokal (AC-6)
pub fn konto_entfernen(pfade: &Pfade, i: usize) -> Result<Zugang, String> {
    let mut k = konten_laden(pfade);
    if aktiv_index(pfade, &k) == Some(i) {
        return Err(tr(T::KontoAktivEntfernen).into());
    }
    let weg = k.remove(i);
    konten_schreiben(pfade, &k)?;
    Ok(weg)
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
    let (schema, rest) = url.split_once("://").ok_or(tr(T::UrlSchema))?;
    let host = rest.split(['/', ':']).next().unwrap_or_default();
    match schema {
        "https" if !host.is_empty() => Ok(url.to_string()),
        "http" if host == "localhost" || host == "127.0.0.1" => Ok(url.to_string()),
        "http" => Err(tr(T::BitteHttps).into()),
        _ => Err(tr(T::UrlSchema).into()),
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
    /// Server antwortet 402: mehr Geräte verbunden, als der Tarif erlaubt
    TarifGrenze,
}

impl std::fmt::Display for Verbindung {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(tr(match self {
            Verbindung::NichtAngemeldet => T::VerbNichtAngemeldet,
            Verbindung::Verbunden => T::VerbVerbunden,
            Verbindung::Offline => T::VerbOffline,
            Verbindung::TokenUngueltig => T::VerbTokenUngueltig,
            Verbindung::ServerNeuer => T::VerbServerNeuer,
            Verbindung::TarifGrenze => T::VerbTarifGrenze,
        }))
    }
}

enum Fehler {
    TokenUngueltig,
    TarifGrenze,
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
        ureq::Error::StatusCode(402) => Fehler::TarifGrenze,
        e => Fehler::Netz(e.to_string()),
    })
}

/// Prüft den Token am Server, speichert dann Zugang und Stand. Liefert die Anzahl Bausteine.
pub fn zugang_speichern(pfade: &Pfade, z: &Zugang) -> Result<usize, String> {
    let mut res = anfrage(z, "/api/sync/snapshot", Some(Duration::from_secs(30))).map_err(|e| match e {
        Fehler::TokenUngueltig => tr(T::TokenWiderrufen).to_string(),
        Fehler::TarifGrenze => tr(T::VerbTarifGrenze).to_string(),
        Fehler::Netz(e) => f(crate::i18n::aktuell(), T::ServerNichtErreichbar, &[&e]),
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
    format!("{server}/connect-device?port={port}&state={}&challenge={}&name={}", url_kodieren(state), url_kodieren(challenge), url_kodieren(name))
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

fn code_einloesen(server: &str, code: &str, verifier: &str) -> Result<(String, String), String> {
    #[derive(Deserialize)]
    struct Antwort {
        token: String,
        #[serde(default)]
        email: String,
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).http_status_as_error(false).build().into();
    let mut res = agent
        .post(format!("{server}/api/sync/token"))
        .header("content-type", "application/json")
        .send(serde_json::json!({ "code": code, "verifier": verifier }).to_string())
        .map_err(|e| f(crate::i18n::aktuell(), T::ServerNichtErreichbar, &[&e]))?;
    let text = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if res.status() != 200 {
        let msg = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["error"].as_str().map(String::from));
        return Err(msg.unwrap_or_else(|| format!("Server antwortet {}", res.status())));
    }
    serde_json::from_str::<Antwort>(&text).map(|a| (a.token, a.email)).map_err(|e| e.to_string())
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
    println!("{}\n{url}", tr(T::BrowserOeffnet));
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
            Rueckruf::Fremd => antworten(&s, "400 Bad Request", tr(T::SeiteUnbekannt)),
            Rueckruf::Abgebrochen => {
                antworten(&s, "200 OK", tr(T::SeiteAbgebrochen));
                return Err(tr(T::VerbindenAbgebrochen).into());
            }
            Rueckruf::Code(code) => {
                let ergebnis = code_einloesen(&server, &code, &verifier).and_then(|(token, email)| {
                    let z = Zugang { server: server.clone(), token, email };
                    // #196 AC-1: das bisherige Konto bleibt in der Liste, das neue kommt dazu
                    let _ = konto_merken(pfade, &Zugang::laden(&pfade.zugang).unwrap_or_else(|| z.clone()));
                    let n = zugang_speichern(pfade, &z)?;
                    konto_merken(pfade, &z)?;
                    Ok(n)
                });
                match &ergebnis {
                    Ok(_) => antworten(&s, "200 OK", tr(T::SeiteVerbunden)),
                    Err(e) => antworten(&s, "200 OK", &f(crate::i18n::aktuell(), T::SeiteFehler, &[e])),
                }
                return ergebnis.map(|n| (server, n));
            }
        }
    }
    Err(tr(T::KeineRueckmeldung).into())
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
            Err(Fehler::TarifGrenze) => {
                melden(Meldung::Verbindung(Verbindung::TarifGrenze));
                // Gerät getrennt oder Pro gebucht? Einmal pro Minute nachsehen.
                std::thread::sleep(Duration::from_secs(60));
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
                    // #196: Konto gewechselt → Verbindung des alten Kontos beenden (spätestens beim nächsten Ping)
                    if Zugang::laden(&pfade.zugang).is_none_or(|n| n.token != z.token) {
                        break;
                    }
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
            "http://localhost:4321/connect-device?port=45123&state=S&challenge=C&name=mein%20rechner%2F%C3%A4"
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
        let z = Zugang { server: "https://x".into(), token: "tk_geheim".into(), email: String::new() };
        assert!(!format!("{z:?}").contains("geheim"));
    }

    #[test]
    fn konten_alte_installation_merken_finden_entfernen() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("tippkraft-konten-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pfade = Pfade { zugang: dir.join("zugang.json"), konten: dir.join("konten.json"), stand: dir.join("stand.json"), socket: dir.join("s") };
        let z = |t: &str, e: &str| Zugang { server: "https://tippkraft.com".into(), token: t.into(), email: e.into() };
        assert!(konten_laden(&pfade).is_empty());
        // AC-7: nur zugang.json (alte Version, ohne E-Mail) → ein Konto, aktiv
        privat_schreiben(&pfade.zugang, br#"{"server":"https://tippkraft.com","token":"tk_a"}"#).unwrap();
        let k = konten_laden(&pfade);
        assert_eq!((k.len(), aktiv_index(&pfade, &k)), (1, Some(0)));
        assert_eq!(k[0].name(), "https://tippkraft.com");
        // AC-1: zweites Konto kommt dazu, neuer Token für dasselbe Konto ersetzt statt zu doppeln
        konto_merken(&pfade, &z("tk_a", "")).unwrap();
        konto_merken(&pfade, &z("tk_b", "firma@example.com")).unwrap();
        konto_merken(&pfade, &z("tk_b2", "firma@example.com")).unwrap();
        let k = konten_laden(&pfade);
        assert_eq!(k.iter().map(|x| x.token.as_str()).collect::<Vec<_>>(), ["tk_a", "tk_b2"]);
        assert_eq!(k[1].name(), "firma@example.com (tippkraft.com)");
        // AC-2/3: Auswahl per Nummer oder E-Mail
        assert_eq!(konto_finden(&k, "2"), Some(1));
        assert_eq!(konto_finden(&k, "FIRMA@example.com"), Some(1));
        assert_eq!(konto_finden(&k, "0"), None);
        assert_eq!(konto_finden(&k, "3"), None);
        // AC-6: aktives Konto nicht entfernbar, anderes schon
        assert!(konto_entfernen(&pfade, 0).is_err());
        assert_eq!(konto_entfernen(&pfade, 1).unwrap().token, "tk_b2");
        assert_eq!(konten_laden(&pfade).len(), 1);
        // AC-9: nur für den Nutzer lesbar
        assert_eq!(std::fs::metadata(&pfade.konten).unwrap().permissions().mode() & 0o777, 0o600);
        // Token taucht nicht in Debug auf
        assert!(!format!("{:?}", z("tk_geheim", "x@y.z")).contains("tk_geheim"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
