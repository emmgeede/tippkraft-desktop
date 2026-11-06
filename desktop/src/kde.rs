//! F11 AC-7: Fensterwechsel unter KDE. tippkraft lädt ein kleines KWin-Skript, das bei jedem
//! Fensterwechsel die Fensterklasse per D-Bus an tippkraft meldet (nur die Klasse, keinen Titel).

use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

const NAME: &str = "com.tippkraft.Fokus";
const SKRIPT: &str = "tippkraft-fokus";

pub const SKRIPT_JS: &str = r#"function melden(w) {
    callDBus("com.tippkraft.Fokus", "/Fokus", "com.tippkraft.Fokus", "Fenster", w ? String(w.resourceClass || "") : "");
}
workspace.windowActivated.connect(melden);
melden(workspace.activeWindow);
"#;

struct Fokus {
    tx: Sender<String>,
}

#[zbus::interface(name = "com.tippkraft.Fokus")]
impl Fokus {
    fn fenster(&self, klasse: String) {
        let _ = self.tx.send(klasse);
    }
}

fn kwin(conn: &zbus::blocking::Connection, methode: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> zbus::Result<zbus::message::Message> {
    conn.call_method(Some("org.kde.KWin"), "/Scripting", Some("org.kde.kwin.Scripting"), methode, body)
}

pub fn beobachten(melden: &mut dyn FnMut(Option<String>)) -> Result<(), String> {
    let (tx, rx) = channel();
    let conn = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(NAME))
        .and_then(|b| b.serve_at("/Fokus", Fokus { tx }))
        .and_then(|b| b.build())
        .map_err(|e| format!("D-Bus nicht erreichbar: {e}"))?;
    let datei = std::env::temp_dir().join(format!("{SKRIPT}-{}.js", std::process::id()));
    std::fs::write(&datei, SKRIPT_JS).map_err(|e| e.to_string())?;
    let _ = kwin(&conn, "unloadScript", &(SKRIPT,));
    kwin(&conn, "loadScript", &(datei.to_string_lossy().as_ref(), SKRIPT)).map_err(|e| format!("KWin-Skript nicht ladbar: {e}"))?;
    kwin(&conn, "start", &()).map_err(|e| format!("KWin-Skript startet nicht: {e}"))?;
    loop {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(k) => melden(if k.is_empty() { None } else { Some(k) }),
            // KWin neu gestartet: Skript ist weg, neu laden (fokus::dauerhaft versucht es erneut)
            Err(_) => {
                let geladen = kwin(&conn, "isScriptLoaded", &(SKRIPT,)).ok().and_then(|m| m.body().deserialize::<bool>().ok()).unwrap_or(false);
                if !geladen {
                    return Err("KWin-Skript nicht mehr geladen".into());
                }
            }
        }
    }
}
