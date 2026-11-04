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

/// `tippkraft login`: Token prüfen, dann Zugang und Stand speichern. Liefert die Anzahl Bausteine.
pub fn login(pfade: &Pfade, url: &str, token: &str) -> Result<usize, String> {
    let z = Zugang { server: server_url(url)?, token: token.trim().to_string() };
    let mut res = anfrage(&z, "/api/sync/snapshot", Some(Duration::from_secs(30))).map_err(|e| match e {
        Fehler::TokenUngueltig => "Token ungültig oder widerrufen".to_string(),
        Fehler::Netz(e) => format!("Server nicht erreichbar: {e}"),
    })?;
    let json = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let stand = format::parse(&json)?;
    privat_schreiben(&pfade.zugang, serde_json::to_string_pretty(&z).unwrap().as_bytes()).map_err(|e| e.to_string())?;
    privat_schreiben(&pfade.stand, json.as_bytes()).map_err(|e| e.to_string())?;
    Ok(stand.bausteine.len())
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
                    let Some((event, data)) = sse.zeile(&zeile) else { continue };
                    if event != "snapshot" {
                        continue;
                    }
                    if erstes {
                        erstes = false;
                        backoff = 1;
                        melden(Meldung::Verbindung(Verbindung::Verbunden));
                    }
                    match format::parse(&data) {
                        Ok(stand) => {
                            if let Err(e) = privat_schreiben(&pfade.stand, data.as_bytes()) {
                                eprintln!("tippkraft: Stand nicht speicherbar: {e}");
                            }
                            melden(Meldung::Stand(stand));
                        }
                        Err(_) => melden(Meldung::Verbindung(Verbindung::ServerNeuer)),
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
    fn zugang_debug_zeigt_keinen_token() {
        let z = Zugang { server: "https://x".into(), token: "tk_geheim".into() };
        assert!(!format!("{z:?}").contains("geheim"));
    }
}
