//! Fenster-Fokus über die Sway-IPC (i3-Protokoll, F22). Rohes JSON wie bei Niri, kein Crate.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use serde_json::Value;

const MAGIC: &[u8; 6] = b"i3-ipc";
const GET_TREE: u32 = 4;
const SUBSCRIBE: u32 = 2;

fn senden(s: &mut UnixStream, typ: u32, inhalt: &[u8]) -> std::io::Result<()> {
    let mut b = MAGIC.to_vec();
    b.extend((inhalt.len() as u32).to_le_bytes());
    b.extend(typ.to_le_bytes());
    b.extend(inhalt);
    s.write_all(&b)
}

fn lesen(s: &mut impl Read) -> Result<(u32, Value), String> {
    let mut kopf = [0u8; 14];
    s.read_exact(&mut kopf).map_err(|e| e.to_string())?;
    if &kopf[..6] != MAGIC {
        return Err("Sway-IPC: unbekanntes Format".into());
    }
    let laenge = u32::from_le_bytes(kopf[6..10].try_into().unwrap()) as usize;
    let typ = u32::from_le_bytes(kopf[10..14].try_into().unwrap());
    let mut inhalt = vec![0u8; laenge];
    s.read_exact(&mut inhalt).map_err(|e| e.to_string())?;
    Ok((typ, serde_json::from_slice(&inhalt).unwrap_or(Value::Null)))
}

/// app_id eines Fensters, bei XWayland die Fensterklasse
pub fn app_id(c: &Value) -> Option<String> {
    c["app_id"].as_str().filter(|s| !s.is_empty()).or_else(|| c["window_properties"]["class"].as_str()).map(String::from)
}

/// Fokussiertes Element im Baum (kann ein leerer Arbeitsbereich sein)
fn fokussiert(knoten: &Value) -> Option<&Value> {
    if knoten["focused"].as_bool() == Some(true) {
        return Some(knoten);
    }
    ["nodes", "floating_nodes"].iter().flat_map(|k| knoten[*k].as_array().into_iter().flatten()).find_map(fokussiert)
}

/// Merkt sich das fokussierte Fenster aus den `window`-Ereignissen
#[derive(Default)]
pub struct Fokus {
    aktiv: Option<i64>,
}

impl Fokus {
    /// Startzustand aus GET_TREE
    pub fn baum(&mut self, baum: &Value) -> Option<String> {
        let f = fokussiert(baum)?;
        self.aktiv = f["id"].as_i64();
        app_id(f)
    }

    /// Liefert `Some(app_id)` bei einem Fokuswechsel
    pub fn ereignis(&mut self, e: &Value) -> Option<Option<String>> {
        let c = &e["container"];
        let id = c["id"].as_i64();
        match e["change"].as_str()? {
            "focus" if id != self.aktiv => {
                self.aktiv = id;
                Some(app_id(c))
            }
            "close" if id.is_some() && id == self.aktiv => {
                self.aktiv = None;
                Some(None)
            }
            _ => None,
        }
    }
}

/// Liest den Fokus aus `SWAYSOCK` und ruft `melden` bei jedem Wechsel auf. Kehrt nur bei Fehlern zurück.
pub fn beobachten(pfad: &str, melden: &mut dyn FnMut(Option<String>)) -> Result<(), String> {
    let mut s = UnixStream::connect(pfad).map_err(|e| format!("Sway-IPC nicht erreichbar: {e}"))?;
    let mut fokus = Fokus::default();
    senden(&mut s, GET_TREE, b"").map_err(|e| e.to_string())?;
    let (_, baum) = lesen(&mut s)?;
    melden(fokus.baum(&baum));
    senden(&mut s, SUBSCRIBE, br#"["window"]"#).map_err(|e| e.to_string())?;
    loop {
        let (typ, v) = lesen(&mut s)?;
        // Ereignisse haben das oberste Bit gesetzt, sonst ist es die Antwort auf SUBSCRIBE
        if typ & 0x8000_0000 != 0 {
            if let Some(app) = fokus.ereignis(&v) {
                melden(app);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn startzustand_und_ereignisse_ac1() {
        let mut f = Fokus::default();
        let baum = json!({"id": 1, "nodes": [{"id": 2, "nodes": [
            {"id": 10, "app_id": "foot", "focused": false},
            {"id": 11, "app_id": null, "window_properties": {"class": "Slack"}, "focused": true}
        ], "floating_nodes": []}]});
        assert_eq!(f.baum(&baum).as_deref(), Some("Slack"));
        assert_eq!(f.ereignis(&json!({"change": "focus", "container": {"id": 10, "app_id": "foot"}})), Some(Some("foot".into())));
        assert_eq!(f.ereignis(&json!({"change": "focus", "container": {"id": 10, "app_id": "foot"}})), None, "kein Wechsel");
        assert_eq!(f.ereignis(&json!({"change": "title", "container": {"id": 10, "app_id": "foot"}})), None);
        assert_eq!(f.ereignis(&json!({"change": "close", "container": {"id": 11}})), None, "anderes Fenster");
        assert_eq!(f.ereignis(&json!({"change": "close", "container": {"id": 10}})), Some(None));
        assert_eq!(f.ereignis(&json!({"change": "focus", "container": {"id": 12, "app_id": ""}})), Some(None), "ohne app_id und Klasse");
    }

    #[test]
    fn spricht_das_i3_protokoll_ac1() {
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("tk-sway-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pfad = dir.join("sway.sock");
        let _ = std::fs::remove_file(&pfad);
        let l = UnixListener::bind(&pfad).unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let (typ, _) = lesen(&mut s).unwrap();
            assert_eq!(typ, GET_TREE);
            senden(&mut s, GET_TREE, br#"{"id":1,"nodes":[{"id":5,"app_id":"foot","focused":true}]}"#).unwrap();
            let (typ, v) = lesen(&mut s).unwrap();
            assert_eq!((typ, v), (SUBSCRIBE, json!(["window"])));
            senden(&mut s, SUBSCRIBE, br#"{"success":true}"#).unwrap();
            senden(&mut s, 0x8000_0003, br#"{"change":"focus","container":{"id":6,"app_id":"org.keepassxc.KeePassXC"}}"#).unwrap();
        });
        let mut gemeldet = vec![];
        let fehler = beobachten(pfad.to_str().unwrap(), &mut |a| gemeldet.push(a));
        assert!(fehler.is_err(), "Verbindung endet");
        assert_eq!(gemeldet, [Some("foot".to_string()), Some("org.keepassxc.KeePassXC".to_string())]);
    }
}
