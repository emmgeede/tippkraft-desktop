//! Fensterwechsel unter GNOME (Wayland). GNOME hat keine Schnittstelle für das aktive Fenster,
//! deshalb legt tippkraft eine kleine Shell-Erweiterung an, die bei jedem Fensterwechsel die
//! Fensterklasse per D-Bus meldet (nur die Klasse, keinen Titel), wie das KWin-Skript unter KDE.
//! Eine neu angelegte Erweiterung lädt GNOME unter Wayland erst nach Ab- und Anmelden.

use std::time::Duration;

pub const UUID: &str = "tippkraft-fokus@tippkraft.com";

const METADATA: &str = r#"{
  "uuid": "tippkraft-fokus@tippkraft.com",
  "name": "tippkraft focus",
  "description": "Tells the tippkraft desktop app which window is active, so its exclusion list works. Sends only the window class, never titles or content.",
  "shell-version": ["45", "46", "47", "48", "49", "50"],
  "url": "https://tippkraft.com/help/desktop.html"
}
"#;

pub const EXTENSION_JS: &str = r#"import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const NAME = 'com.tippkraft.Fokus';

export default class TippkraftFokus extends Extension {
    enable() {
        const melden = () => {
            const w = global.display.focus_window;
            Gio.DBus.session.call(NAME, '/Fokus', NAME, 'Fenster',
                new GLib.Variant('(s)', [w ? (w.get_wm_class() || '') : '']),
                null, Gio.DBusCallFlags.NO_AUTO_START, -1, null, null);
        };
        this._fokus = global.display.connect('notify::focus-window', melden);
        // tippkraft (neu) gestartet: aktuelles Fenster sofort melden
        this._name = Gio.bus_watch_name(Gio.BusType.SESSION, NAME, Gio.BusNameWatcherFlags.NONE, melden, null);
    }

    disable() {
        global.display.disconnect(this._fokus);
        Gio.bus_unwatch_name(this._name);
    }
}
"#;

fn ordner(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".local/share/gnome-shell/extensions").join(UUID)
}

/// Legt die Erweiterung an (nur wenn geändert) und trägt sie in die eingeschalteten Erweiterungen ein.
pub fn installieren(home: &std::path::Path) -> Result<(), String> {
    dateien(home)?;
    // Läuft die Shell schon mit der Erweiterung, reicht das; sonst gsettings, damit sie nach dem Anmelden startet
    if aktiv() {
        return Ok(());
    }
    let alt = std::process::Command::new("gsettings").args(["get", "org.gnome.shell", "enabled-extensions"]).output().map_err(|e| e.to_string())?;
    let alt = String::from_utf8_lossy(&alt.stdout).trim().to_string();
    if let Some(neu) = eingeschaltet(&alt) {
        std::process::Command::new("gsettings").args(["set", "org.gnome.shell", "enabled-extensions", &neu]).status().map_err(|e| e.to_string())?;
    }
    let _ = std::process::Command::new("gnome-extensions").args(["enable", UUID]).output();
    Ok(())
}

fn dateien(home: &std::path::Path) -> Result<(), String> {
    let dir = ordner(home);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, inhalt) in [("metadata.json", METADATA), ("extension.js", EXTENSION_JS)] {
        let p = dir.join(name);
        if std::fs::read_to_string(&p).ok().as_deref() != Some(inhalt) {
            std::fs::write(&p, inhalt).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Liste aus `gsettings get` um die UUID ergänzt; None, wenn sie schon drinsteht
fn eingeschaltet(alt: &str) -> Option<String> {
    if alt.contains(&format!("'{UUID}'")) {
        return None;
    }
    let innen = alt.trim_start_matches("@as").trim().trim_start_matches('[').trim_end_matches(']').trim();
    Some(if innen.is_empty() { format!("['{UUID}']") } else { format!("[{innen}, '{UUID}']") })
}

/// Ist die Erweiterung in der laufenden Shell aktiv?
pub fn aktiv() -> bool {
    std::process::Command::new("gnome-extensions").args(["info", UUID]).output().map(|o| String::from_utf8_lossy(&o.stdout).contains("ACTIVE")).unwrap_or(false)
}

pub fn beobachten(melden: &mut dyn FnMut(Option<String>)) -> Result<(), String> {
    if let Some(Err(e)) = std::env::var_os("HOME").map(|h| installieren(std::path::Path::new(&h))) {
        eprintln!("tippkraft: GNOME-Erweiterung nicht angelegt: {e}");
    }
    let (_conn, rx) = crate::kde::dienst()?;
    loop {
        // Ohne aktive Erweiterung kommt nie etwas; dann ersetzt tippkraft wie bisher überall
        if let Ok(k) = rx.recv_timeout(Duration::from_secs(3600)) {
            melden(if k.is_empty() { None } else { Some(k) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ergaenzt_die_liste_der_erweiterungen() {
        assert_eq!(eingeschaltet("@as []").as_deref(), Some("['tippkraft-fokus@tippkraft.com']"));
        assert_eq!(eingeschaltet("['a@b']").as_deref(), Some("['a@b', 'tippkraft-fokus@tippkraft.com']"));
        assert_eq!(eingeschaltet("['tippkraft-fokus@tippkraft.com']"), None);
    }

    #[test]
    fn installiert_die_dateien() {
        let home = std::env::temp_dir().join(format!("tk-gnome-{}", std::process::id()));
        dateien(&home).unwrap();
        let js = std::fs::read_to_string(ordner(&home).join("extension.js")).unwrap();
        assert!(js.contains("get_wm_class") && !js.contains("title"));
        assert!(std::fs::read_to_string(ordner(&home).join("metadata.json")).unwrap().contains(UUID));
        let _ = std::fs::remove_dir_all(&home);
    }
}
