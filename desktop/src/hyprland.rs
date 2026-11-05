//! Fenster-Fokus über die Hyprland-IPC (F22): `j/activewindow` und Ereignisse aus `.socket2.sock`

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Verzeichnis der Sockets: neuere Versionen unter $XDG_RUNTIME_DIR/hypr, ältere unter /tmp/hypr
pub fn verzeichnis(signatur: &str, runtime: Option<&str>) -> PathBuf {
    let neu = runtime.map(|r| Path::new(r).join("hypr").join(signatur));
    match neu {
        Some(p) if p.join(".socket2.sock").exists() => p,
        _ => Path::new("/tmp/hypr").join(signatur),
    }
}

/// Klasse aus einer Zeile `activewindow>>klasse,titel`. `Some(None)` heißt kein Fenster.
pub fn zeile(z: &str) -> Option<Option<String>> {
    let rest = z.strip_prefix("activewindow>>")?;
    let klasse = rest.split_once(',').map_or(rest, |(k, _)| k);
    Some((!klasse.is_empty()).then(|| klasse.to_string()))
}

fn aktives_fenster(verz: &Path) -> Option<String> {
    let mut s = UnixStream::connect(verz.join(".socket.sock")).ok()?;
    s.write_all(b"j/activewindow").ok()?;
    let mut antwort = String::new();
    s.read_to_string(&mut antwort).ok()?;
    let v: Value = serde_json::from_str(&antwort).ok()?;
    v["class"].as_str().filter(|k| !k.is_empty()).map(String::from)
}

/// Liest den Fokus und ruft `melden` bei jedem Fensterwechsel auf. Kehrt nur bei Fehlern zurück.
pub fn beobachten(verz: &Path, melden: &mut dyn FnMut(Option<String>)) -> Result<(), String> {
    let ereignisse = UnixStream::connect(verz.join(".socket2.sock")).map_err(|e| format!("Hyprland-IPC nicht erreichbar: {e}"))?;
    let mut zuletzt = aktives_fenster(verz);
    melden(zuletzt.clone());
    for z in BufReader::new(ereignisse).lines() {
        // Hyprland 0.57 schickt denselben Wechsel teils doppelt
        if let Some(app) = zeile(&z.map_err(|e| e.to_string())?).filter(|a| *a != zuletzt) {
            zuletzt = app.clone();
            melden(app);
        }
    }
    Err("Hyprland-IPC beendet".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn liest_ereigniszeilen_ac2() {
        assert_eq!(zeile("activewindow>>foot,~/code"), Some(Some("foot".into())));
        assert_eq!(zeile("activewindow>>firefox,Titel, mit Komma"), Some(Some("firefox".into())));
        assert_eq!(zeile("activewindow>>,"), Some(None));
        assert_eq!(zeile("activewindowv2>>55d1f5a0"), None);
        assert_eq!(zeile("workspace>>2"), None);
    }

    #[test]
    fn spricht_mit_beiden_sockets_ac2() {
        let verz = std::env::temp_dir().join(format!("tk-hypr-{}", std::process::id()));
        std::fs::create_dir_all(&verz).unwrap();
        for s in [".socket.sock", ".socket2.sock"] {
            let _ = std::fs::remove_file(verz.join(s));
        }
        let befehle = UnixListener::bind(verz.join(".socket.sock")).unwrap();
        let ereignisse = UnixListener::bind(verz.join(".socket2.sock")).unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = befehle.accept().unwrap();
            let mut buf = [0u8; 64];
            let n = s.read(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"j/activewindow");
            s.write_all(br#"{"class":"foot","title":"x"}"#).unwrap();
        });
        std::thread::spawn(move || {
            let (mut s, _) = ereignisse.accept().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            s.write_all(b"workspace>>2\nactivewindow>>foot,x\nactivewindow>>org.keepassxc.KeePassXC,DB\nactivewindowv2>>abc\nactivewindow>>org.keepassxc.KeePassXC,DB\nactivewindow>>,\n").unwrap();
        });
        let mut gemeldet = vec![];
        assert!(beobachten(&verz, &mut |a| gemeldet.push(a)).is_err());
        assert_eq!(gemeldet, [Some("foot".to_string()), Some("org.keepassxc.KeePassXC".to_string()), None]);
    }

    #[test]
    fn findet_das_socket_verzeichnis_ac2() {
        let r = std::env::temp_dir().join(format!("tk-hypr-run-{}", std::process::id()));
        assert_eq!(verzeichnis("abc", Some(r.to_str().unwrap())), Path::new("/tmp/hypr/abc"));
        std::fs::create_dir_all(r.join("hypr/abc")).unwrap();
        std::fs::write(r.join("hypr/abc/.socket2.sock"), "").unwrap();
        assert_eq!(verzeichnis("abc", Some(r.to_str().unwrap())), r.join("hypr/abc"));
    }
}
