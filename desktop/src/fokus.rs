//! Welcher Compositor läuft, und dessen Fokus-Meldungen mit Neuverbindung (F22)

use std::time::Duration;

use crate::{hyprland, kde, niri, sway};

#[derive(Debug, PartialEq)]
pub enum Quelle {
    Niri(String),
    Sway(String),
    Hyprland(std::path::PathBuf),
    /// F11: KWin-Skript über D-Bus
    Kde,
}

/// Niri hat Vorrang, danach Sway, dann Hyprland (AC-3)
pub fn erkennen(env: impl Fn(&str) -> Option<String>) -> Option<Quelle> {
    if let Some(p) = env("NIRI_SOCKET") {
        return Some(Quelle::Niri(p));
    }
    if let Some(p) = env("SWAYSOCK") {
        return Some(Quelle::Sway(p));
    }
    if let Some(sig) = env("HYPRLAND_INSTANCE_SIGNATURE") {
        return Some(Quelle::Hyprland(hyprland::verzeichnis(&sig, env("XDG_RUNTIME_DIR").as_deref())));
    }
    (crate::uinput::sitzung(&env) == crate::uinput::Sitzung::Kde).then_some(Quelle::Kde)
}

pub fn beobachten(q: &Quelle, melden: &mut dyn FnMut(Option<String>)) -> Result<(), String> {
    match q {
        Quelle::Niri(p) => niri::beobachten(p, melden),
        Quelle::Sway(p) => sway::beobachten(p, melden),
        Quelle::Hyprland(v) => hyprland::beobachten(v, melden),
        Quelle::Kde => kde::beobachten(melden),
    }
}

/// Läuft dauerhaft: bei Abbruch alle 5 Sekunden neu verbinden (AC-4). Ohne Compositor einmal loggen (AC-3).
pub fn dauerhaft(mut melden: impl FnMut(Option<String>)) {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let Some(q) = erkennen(env) else {
        eprintln!("tippkraft: kein unterstützter Compositor (Niri, Sway, Hyprland, KDE) erkannt, Ausschlussliste ist ohne Wirkung");
        return;
    };
    let mut gemeldet = false;
    loop {
        if let Err(e) = beobachten(&q, &mut melden) {
            if !gemeldet {
                eprintln!("tippkraft: {e}, neuer Versuch alle 5 Sekunden");
                gemeldet = true;
            }
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

/// Zeilen für Pause (Mod+Alt+T) und Übersicht (Mod+Alt+Leertaste) im Format des Compositors (AC-5)
/// `hypr_lua`: Hyprland ab 0.57 legt `hyprland.lua` statt `hyprland.conf` an, mit eigener Schreibweise
pub fn tastenkuerzel(q: &Quelle, bin: &str, hypr_lua: bool) -> (&'static str, &'static str, String) {
    match q {
        Quelle::Niri(_) => ("Niri", "~/.config/niri/config.kdl (binds { … })", format!("Mod+Alt+T {{ spawn \"{bin}\" \"pause\"; }}\nMod+Alt+Space {{ spawn \"{bin}\" \"overview\"; }}")),
        Quelle::Sway(_) => ("Sway", "~/.config/sway/config", format!("bindsym $mod+Alt+t exec {bin} pause\nbindsym $mod+Alt+space exec {bin} overview")),
        Quelle::Hyprland(_) if hypr_lua => ("Hyprland", "~/.config/hypr/hyprland.lua", format!("hl.bind(\"SUPER + ALT + T\", hl.dsp.exec_cmd(\"{bin} pause\"))\nhl.bind(\"SUPER + ALT + SPACE\", hl.dsp.exec_cmd(\"{bin} overview\"))")),
        Quelle::Hyprland(_) => ("Hyprland", "~/.config/hypr/hyprland.conf", format!("bind = SUPER ALT, T, exec, {bin} pause\nbind = SUPER ALT, SPACE, exec, {bin} overview")),
        Quelle::Kde => ("KDE Plasma", "", format!("Meta+Alt+T: {bin} pause\nMeta+Alt+Space: {bin} overview")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(paare: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = paare.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn erkennt_den_compositor_ac3() {
        assert_eq!(erkennen(env(&[("SWAYSOCK", "/run/s"), ("NIRI_SOCKET", "/run/n")])), Some(Quelle::Niri("/run/n".into())));
        assert_eq!(erkennen(env(&[("SWAYSOCK", "/run/s")])), Some(Quelle::Sway("/run/s".into())));
        assert_eq!(erkennen(env(&[("HYPRLAND_INSTANCE_SIGNATURE", "abc")])), Some(Quelle::Hyprland("/tmp/hypr/abc".into())));
        assert_eq!(erkennen(env(&[])), None);
        assert_eq!(erkennen(env(&[("XDG_CURRENT_DESKTOP", "KDE")])), Some(Quelle::Kde));
        assert_eq!(erkennen(env(&[("XDG_CURRENT_DESKTOP", "GNOME")])), None);
    }

    #[test]
    fn zeigt_tastenkuerzel_je_compositor_ac5() {
        let (name, datei, zeilen) = tastenkuerzel(&Quelle::Sway("/s".into()), "/home/a/.local/bin/tippkraft", false);
        assert_eq!((name, datei), ("Sway", "~/.config/sway/config"));
        assert_eq!(zeilen, "bindsym $mod+Alt+t exec /home/a/.local/bin/tippkraft pause\nbindsym $mod+Alt+space exec /home/a/.local/bin/tippkraft overview");
        assert!(tastenkuerzel(&Quelle::Hyprland("/h".into()), "tk", false).2.starts_with("bind = SUPER ALT, T, exec, tk pause"));
        let (_, datei, zeilen) = tastenkuerzel(&Quelle::Hyprland("/h".into()), "tk", true);
        assert_eq!(datei, "~/.config/hypr/hyprland.lua");
        assert_eq!(zeilen, "hl.bind(\"SUPER + ALT + T\", hl.dsp.exec_cmd(\"tk pause\"))\nhl.bind(\"SUPER + ALT + SPACE\", hl.dsp.exec_cmd(\"tk overview\"))");
        assert!(tastenkuerzel(&Quelle::Niri("/n".into()), "tk", false).2.contains("Mod+Alt+Space { spawn \"tk\" \"overview\"; }"));
    }
}
