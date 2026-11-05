//! Übersetzungen am Desktop (F15): Englisch als Basis, Deutsch.
//! Die Sprache kommt aus dem Stand (Account), sonst aus TIPPKRAFT_SPRACHE oder der Systemsprache.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sprache {
    En,
    De,
}

impl Sprache {
    pub fn aus_code(code: &str) -> Option<Sprache> {
        match code {
            "en" => Some(Sprache::En),
            "de" => Some(Sprache::De),
            _ => None,
        }
    }

    /// TIPPKRAFT_SPRACHE, sonst LC_ALL / LC_MESSAGES / LANG: beginnt mit "de" → Deutsch, sonst Englisch
    pub fn aus_umgebung() -> Sprache {
        if let Some(s) = std::env::var("TIPPKRAFT_SPRACHE").ok().and_then(|v| Sprache::aus_code(&v)) {
            return s;
        }
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|v| std::env::var(v).ok()).find(|v| !v.is_empty()).unwrap_or_default();
        if locale.starts_with("de") { Sprache::De } else { Sprache::En }
    }

    /// Sprache aus dem Stand, sonst Umgebung
    pub fn fuer(stand_sprache: Option<&str>) -> Sprache {
        stand_sprache.and_then(Sprache::aus_code).unwrap_or_else(Sprache::aus_umgebung)
    }
}

static AKTUELL: AtomicU8 = AtomicU8::new(0);

/// Sprache für Meldungen der Kommandozeile und des Daemons
pub fn setzen(s: Sprache) {
    AKTUELL.store(s as u8, Ordering::Relaxed);
}

pub fn aktuell() -> Sprache {
    if AKTUELL.load(Ordering::Relaxed) == Sprache::De as u8 { Sprache::De } else { Sprache::En }
}

#[derive(Debug, Clone, Copy)]
pub enum T {
    Hilfe,
    LaeuftNicht,
    LaeuftSchon,
    Pausiert,
    Aktiv,
    StatusZeile,
    VerbNichtAngemeldet,
    VerbVerbunden,
    VerbOffline,
    VerbTokenUngueltig,
    VerbServerNeuer,
    KurzNichtVerbunden,
    KurzVerbunden,
    KurzOffline,
    KurzTokenUngueltig,
    KurzServerNeuer,
    TrayAktiv,
    TrayPausiert,
    TrayBausteine,
    TrayPausieren,
    TrayFortsetzen,
    TrayUebersicht,
    TrayWebtool,
    TrayUpdate,
    Tastenkuerzel,
    UpdateStatus,
    TrayNeuVerbinden,
    TrayBeenden,
    BrowserOeffnet,
    VerbundenMit,
    VerbindenAbgebrochen,
    KeineRueckmeldung,
    SeiteVerbunden,
    SeiteAbgebrochen,
    SeiteFehler,
    SeiteUnbekannt,
    TokenWiderrufen,
    ServerNichtErreichbar,
    BitteHttps,
    UrlSchema,
    Eingerichtet,
    SchonVerbunden,
    LoginWeg,
    PopupBaustein,
    PopupVorschau,
    PopupEinfuegen,
    PopupAbbrechen,
    PopupTasten,
    PopupNichts,
    UebTitel,
    UebAnzahl,
    UebVonN,
    UebSuchen,
    UebKeineTreffer,
    UebLeer,
    UebLoestNicht,
    AuslSofort,
    AuslLeertaste,
    AuslEnter,
    AuslTab,
}

/// Text in der Sprache `s`. Platzhalter in Texten: {0}, {1}, … (siehe `f`)
pub fn text(s: Sprache, t: T) -> &'static str {
    use T::*;
    let (en, de) = match t {
        Hilfe => (
            "tippkraft – text expander for Wayland

  tippkraft setup        Install, set up autostart, start and connect to your account
  tippkraft connect      Connect to your account (opens the browser, confirm there)
  tippkraft status       Show the current state
  tippkraft pause        Pause or resume (releases the keyboard)
  tippkraft overview     Show and filter all snippets (Mod+Alt+Space)
  tippkraft run [--verzoegerung MS] [--stand FILE] [--geraet /dev/input/eventN] [--trocken]

Emergency stop: left Ctrl + left Alt + Backspace quits tippkraft immediately.",
            "tippkraft – Textexpander für Wayland

  tippkraft einrichten   Installieren, Autostart einrichten, starten und mit dem Konto verbinden
  tippkraft verbinden    Mit dem Konto verbinden (öffnet den Browser, dort bestätigen)
  tippkraft status       Zustand anzeigen
  tippkraft pause        Pausieren bzw. fortsetzen (gibt die Tastatur frei)
  tippkraft uebersicht   Alle Bausteine anzeigen und filtern (Mod+Alt+Leertaste)
  tippkraft run [--verzoegerung MS] [--stand DATEI] [--geraet /dev/input/eventN] [--trocken]

Notausschalter: linke Strg + linke Alt + Backspace beendet tippkraft sofort.",
        ),
        LaeuftNicht => ("tippkraft is not running", "tippkraft läuft nicht"),
        LaeuftSchon => ("tippkraft is already running", "tippkraft läuft schon"),
        Pausiert => ("paused", "pausiert"),
        Aktiv => ("active", "aktiv"),
        StatusZeile => ("State: {0} · Connection: {1} · Snippets: {2} · Server: {3}", "Zustand: {0} · Verbindung: {1} · Bausteine: {2} · Server: {3}"),
        VerbNichtAngemeldet => ("not connected", "nicht angemeldet"),
        VerbVerbunden => ("connected", "verbunden"),
        VerbOffline => ("offline, retrying", "offline, versuche es weiter"),
        VerbTokenUngueltig => ("token invalid, please reconnect", "Token ungültig, bitte neu anmelden"),
        VerbServerNeuer => ("server is newer than tippkraft, please update", "Server neuer als tippkraft, bitte aktualisieren"),
        KurzNichtVerbunden => ("not connected", "nicht verbunden"),
        KurzVerbunden => ("connected", "verbunden"),
        KurzOffline => ("offline", "offline"),
        KurzTokenUngueltig => ("token invalid", "Token ungültig"),
        KurzServerNeuer => ("server newer, please update", "Server neuer, bitte aktualisieren"),
        TrayAktiv => ("Active", "Aktiv"),
        TrayPausiert => ("Paused", "Pausiert"),
        TrayBausteine => ("{0} snippets", "{0} Bausteine"),
        TrayPausieren => ("Pause", "Pausieren"),
        TrayFortsetzen => ("Resume", "Fortsetzen"),
        TrayUebersicht => ("Show snippets", "Bausteine anzeigen"),
        TrayWebtool => ("Open web app", "Webtool öffnen"),
        TrayUpdate => ("Update available: {0}", "Update verfügbar: {0}"),
        Tastenkuerzel => ("Shortcuts for {0}: add these lines to {1}", "Tastenkürzel für {0}: diese Zeilen in {1} eintragen"),
        UpdateStatus => ("Update available: {0}. Install with: {1}", "Update verfügbar: {0}. Installieren mit: {1}"),
        TrayNeuVerbinden => ("Reconnect …", "Neu verbinden …"),
        TrayBeenden => ("Quit", "Beenden"),
        BrowserOeffnet => ("Opening your browser. If it does not open, visit:", "Browser öffnet sich. Falls nicht, diese Adresse öffnen:"),
        VerbundenMit => ("Connected to {0}, {1} snippets", "Verbunden mit {0}, {1} Bausteine"),
        VerbindenAbgebrochen => ("Connecting cancelled", "Verbinden abgebrochen"),
        KeineRueckmeldung => ("No response from the browser within 5 minutes", "Keine Rückmeldung vom Browser innerhalb von 5 Minuten"),
        SeiteVerbunden => ("Connected. You can close this window.", "Verbunden. Du kannst dieses Fenster schließen."),
        SeiteAbgebrochen => ("Connecting cancelled. You can close this window.", "Verbinden abgebrochen. Du kannst dieses Fenster schließen."),
        SeiteFehler => ("Connecting failed: {0}", "Verbinden fehlgeschlagen: {0}"),
        SeiteUnbekannt => ("Unknown request.", "Unbekannte Anfrage."),
        TokenWiderrufen => ("Token invalid or revoked", "Token ungültig oder widerrufen"),
        ServerNichtErreichbar => ("Server not reachable: {0}", "Server nicht erreichbar: {0}"),
        BitteHttps => ("Please use https, otherwise the token is sent unencrypted", "Bitte https verwenden, sonst geht der Token unverschlüsselt übers Netz"),
        UrlSchema => ("URL needs http:// or https://", "URL braucht http:// oder https://"),
        Eingerichtet => ("Set up: {0} and tippkraft.service is running.", "Eingerichtet: {0} und tippkraft.service läuft."),
        SchonVerbunden => ("Already connected. Reconnect: “tippkraft verbinden” in the app launcher.", "Schon verbunden. Neu verbinden: „tippkraft verbinden“ im App-Starter."),
        LoginWeg => ("`login` no longer exists. Connecting now works through the browser: tippkraft connect", "`login` gibt es nicht mehr. Verbinden geht jetzt über den Browser: tippkraft verbinden"),
        PopupBaustein => ("Snippet", "Baustein"),
        PopupVorschau => ("Preview", "Vorschau"),
        PopupEinfuegen => ("Insert", "Einfügen"),
        PopupAbbrechen => ("Cancel", "Abbrechen"),
        PopupTasten => ("Enter · Tab · Arrows · Esc", "Enter · Tab · Pfeile · Esc"),
        PopupNichts => ("(nothing)", "(nichts)"),
        UebTitel => ("Snippets", "Bausteine"),
        UebAnzahl => ("{0} snippets", "{0} Bausteine"),
        UebVonN => ("{0} of {1}", "{0} von {1}"),
        UebSuchen => ("Search …", "Suchen …"),
        UebKeineTreffer => ("No matches.", "Keine Treffer."),
        UebLeer => ("No snippets yet. Connect tippkraft via “tippkraft verbinden”.", "Noch keine Bausteine. Verbinde tippkraft über „tippkraft verbinden“."),
        UebLoestNicht => ("does not trigger", "löst nicht aus"),
        AuslSofort => ("Immediately", "Sofort"),
        AuslLeertaste => ("Space", "Leertaste"),
        AuslEnter => ("Enter", "Enter"),
        AuslTab => ("Tab", "Tab"),
    };
    if s == Sprache::De { de } else { en }
}

/// Text mit eingesetzten Werten {0}, {1}, …
pub fn f(s: Sprache, t: T, werte: &[&dyn std::fmt::Display]) -> String {
    let mut out = text(s, t).to_string();
    for (i, w) in werte.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), &w.to_string());
    }
    out
}

/// Kurzform für die aktuelle Sprache
pub fn tr(t: T) -> &'static str {
    text(aktuell(), t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn werte_einsetzen_und_sprache_waehlen() {
        assert_eq!(f(Sprache::De, T::VerbundenMit, &[&"https://x", &3]), "Verbunden mit https://x, 3 Bausteine");
        assert_eq!(f(Sprache::En, T::UebVonN, &[&2, &30]), "2 of 30");
        assert_eq!(Sprache::fuer(Some("de")), Sprache::De);
        assert_eq!(Sprache::fuer(Some("en")), Sprache::En);
    }
}
