//! Steuer-Socket zwischen CLI (`status`, `pause`) und laufendem Daemon. Eine Zeile hin, eine zurück.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

/// Befehl an den Daemon schicken, Antwort zurückgeben. `None`, wenn kein Daemon antwortet.
pub fn senden(socket: &Path, befehl: &str) -> Option<String> {
    let mut s = UnixStream::connect(socket).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    s.write_all(format!("{befehl}\n").as_bytes()).ok()?;
    let mut antwort = String::new();
    BufReader::new(s).read_line(&mut antwort).ok()?;
    Some(antwort.trim_end().to_string())
}

/// Socket öffnen. Fehler, wenn schon ein Daemon antwortet. Ein verwaister Socket wird übernommen.
pub fn lauschen(socket: &Path, tx: Sender<(String, Sender<String>)>) -> Result<(), String> {
    if senden(socket, "status").is_some() {
        return Err("tippkraft läuft schon".into());
    }
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket).map_err(|e| format!("Steuer-Socket {}: {e}", socket.display()))?;
    // nur der eigene Nutzer darf pausieren, auch wenn XDG_RUNTIME_DIR fehlt und der Socket in /tmp liegt
    std::fs::set_permissions(socket, std::os::unix::fs::PermissionsExt::from_mode(0o600)).map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut zeile = String::new();
                let mut r = BufReader::new(&s);
                if r.read_line(&mut zeile).is_err() {
                    return;
                }
                let (antwort_tx, antwort_rx) = mpsc::channel();
                if tx.send((zeile.trim().to_string(), antwort_tx)).is_err() {
                    return;
                }
                if let Ok(a) = antwort_rx.recv_timeout(Duration::from_secs(2)) {
                    let _ = (&s).write_all(format!("{a}\n").as_bytes());
                }
            });
        }
    });
    Ok(())
}
