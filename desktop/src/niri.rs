//! Fenster-Fokus über die Niri-IPC (rohes JSON, damit keine Versionskopplung an niri-ipc entsteht).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use serde_json::Value;

/// Merkt sich app_ids und das fokussierte Fenster aus dem Event-Stream
#[derive(Default)]
pub struct Fokus {
    fenster: HashMap<u64, Option<String>>,
    aktiv: Option<u64>,
}

impl Fokus {
    /// Verarbeitet ein Ereignis. Liefert `Some(app_id)`, wenn der Fokus gewechselt hat.
    pub fn ereignis(&mut self, e: &Value) -> Option<Option<String>> {
        let vorher = self.aktiv;
        let fenster_merken = |f: &Value, s: &mut Self| {
            let id = f["id"].as_u64()?;
            s.fenster.insert(id, f["app_id"].as_str().map(String::from));
            if f["is_focused"].as_bool() == Some(true) {
                s.aktiv = Some(id);
            }
            Some(())
        };
        if let Some(liste) = e["WindowsChanged"]["windows"].as_array() {
            self.fenster.clear();
            self.aktiv = None;
            for f in liste {
                fenster_merken(f, self);
            }
        } else if e.get("WindowOpenedOrChanged").is_some() {
            fenster_merken(&e["WindowOpenedOrChanged"]["window"], self);
        } else if let Some(id) = e["WindowClosed"]["id"].as_u64() {
            self.fenster.remove(&id);
            if self.aktiv == Some(id) {
                self.aktiv = None;
            }
        } else if let Some(f) = e.get("WindowFocusChanged") {
            self.aktiv = f["id"].as_u64();
        } else {
            return None;
        }
        (self.aktiv != vorher).then(|| self.aktiv.and_then(|id| self.fenster.get(&id).cloned().flatten()))
    }
}

/// Liest den Event-Stream und ruft `melden` bei jedem Fokuswechsel auf. Kehrt nur bei Fehlern zurück.
pub fn beobachten(mut melden: impl FnMut(Option<String>)) -> Result<(), String> {
    let pfad = std::env::var("NIRI_SOCKET").map_err(|_| "NIRI_SOCKET nicht gesetzt".to_string())?;
    let mut sock = UnixStream::connect(&pfad).map_err(|e| format!("Niri-IPC nicht erreichbar: {e}"))?;
    sock.write_all(b"\"EventStream\"\n").map_err(|e| e.to_string())?;
    let mut fokus = Fokus::default();
    for zeile in BufReader::new(sock).lines() {
        let zeile = zeile.map_err(|e| e.to_string())?;
        let Ok(v) = serde_json::from_str::<Value>(&zeile) else { continue };
        if let Some(app) = fokus.ereignis(&v) {
            melden(app);
        }
    }
    Err("Niri-IPC beendet".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn verfolgt_den_fokus() {
        let mut f = Fokus::default();
        let start = json!({"WindowsChanged": {"windows": [
            {"id": 1, "app_id": "foot", "is_focused": false},
            {"id": 2, "app_id": "org.keepassxc.KeePassXC", "is_focused": true}
        ]}});
        assert_eq!(f.ereignis(&start), Some(Some("org.keepassxc.KeePassXC".into())));
        assert_eq!(f.ereignis(&json!({"WindowFocusChanged": {"id": 1}})), Some(Some("foot".into())));
        assert_eq!(f.ereignis(&json!({"WindowFocusChanged": {"id": 1}})), None, "kein Wechsel");
        assert_eq!(
            f.ereignis(&json!({"WindowOpenedOrChanged": {"window": {"id": 3, "app_id": "firefox", "is_focused": true}}})),
            Some(Some("firefox".into()))
        );
        assert_eq!(f.ereignis(&json!({"WindowClosed": {"id": 3}})), Some(None));
        assert_eq!(f.ereignis(&json!({"WindowFocusChanged": {"id": null}})), None);
        assert_eq!(f.ereignis(&json!({"WorkspaceActivated": {"id": 5, "focused": true}})), None);
        assert_eq!(f.ereignis(&json!({"WindowFocusChanged": {"id": 2}})), Some(Some("org.keepassxc.KeePassXC".into())));
    }
}
