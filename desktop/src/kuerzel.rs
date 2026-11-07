//! F43: Tastenkürzel für Pause (Mod+Alt+T) und Übersicht (Mod+Alt+Leertaste) beim Einrichten anlegen.
//! Niri, Sway, Hyprland: eigene Datei plus Include-Zeile. GNOME, Cinnamon, Xfce, KDE: über deren Werkzeuge.
//! Belegte Kombinationen werden ausgelassen, nie überschrieben.

/// Die beiden Kürzel: (Bezeichnung, Unterbefehl)
pub const KUERZEL: [(&str, &str); 2] = [("pause", "pause"), ("overview", "overview")];

/// Compositor mit Textkonfiguration
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Text {
    Niri,
    Sway,
    Hyprland,
}

/// Zeile für ein Kürzel in der Schreibweise des Compositors
fn zeile(t: Text, welches: usize, bin: &str) -> String {
    let befehl = KUERZEL[welches].1;
    match (t, welches) {
        (Text::Niri, 0) => format!("    Mod+Alt+T hotkey-overlay-title=\"tippkraft: pause\" {{ spawn \"{bin}\" \"{befehl}\"; }}"),
        (Text::Niri, _) => format!("    Mod+Alt+Space hotkey-overlay-title=\"tippkraft: snippets\" {{ spawn \"{bin}\" \"{befehl}\"; }}"),
        (Text::Sway, 0) => format!("bindsym Mod4+Mod1+t exec {bin} {befehl}"),
        (Text::Sway, _) => format!("bindsym Mod4+Mod1+space exec {bin} {befehl}"),
        (Text::Hyprland, 0) => format!("bind = SUPER ALT, T, exec, {bin} {befehl}"),
        (Text::Hyprland, _) => format!("bind = SUPER ALT, SPACE, exec, {bin} {befehl}"),
    }
}

/// Ist die Kombination in der Konfiguration schon belegt? (Text ohne Groß/Klein und Leerzeichen verglichen)
fn belegt(t: Text, welches: usize, config: &str) -> bool {
    let norm = |s: &str| s.to_lowercase().replace([' ', '\t'], "");
    let c = norm(config);
    let muster: &[&str] = match (t, welches) {
        (Text::Niri, 0) => &["mod+alt+t{", "mod+alt+thotkey", "super+alt+t{", "super+alt+thotkey"],
        (Text::Niri, _) => &["mod+alt+space{", "mod+alt+spacehotkey", "super+alt+space{", "super+alt+spacehotkey"],
        (Text::Sway, 0) => &["bindsym$mod+alt+t", "bindsym$mod+mod1+t", "bindsymmod4+mod1+t", "bindsymmod4+alt+t"],
        (Text::Sway, _) => &["bindsym$mod+alt+space", "bindsym$mod+mod1+space", "bindsymmod4+mod1+space", "bindsymmod4+alt+space"],
        (Text::Hyprland, 0) => &["bind=superalt,t,", "bind=$mainmodalt,t,"],
        (Text::Hyprland, _) => &["bind=superalt,space,", "bind=$mainmodalt,space,"],
    };
    c.lines().any(|l| muster.iter().any(|m| l.contains(m)))
}

/// Dateiname der eigenen Datei und Include-Zeile für die Hauptkonfiguration
pub fn datei(t: Text) -> (&'static str, &'static str) {
    match t {
        Text::Niri => ("tippkraft-tasten.kdl", "include \"tippkraft-tasten.kdl\""),
        Text::Sway => ("tippkraft-tasten.conf", "include ~/.config/sway/tippkraft-tasten.conf"),
        Text::Hyprland => ("tippkraft-tasten.conf", "source = ~/.config/hypr/tippkraft-tasten.conf"),
    }
}

/// Plan für einen Text-Compositor. `config`: Hauptkonfiguration, `andere`: Text aller übrigen Konfigurationsdateien
/// (ohne die eigene Datei), dort belegte Kombinationen zählen auch.
/// Ergebnis: Inhalt der eigenen Datei, neue Hauptkonfiguration (None = Include schon da), eingerichtete und belegte Kürzel
pub struct TextPlan {
    pub datei: String,
    pub config: Option<String>,
    pub eingerichtet: Vec<&'static str>,
    pub belegt: Vec<&'static str>,
}

pub fn text_plan(t: Text, config: &str, andere: &str, bin: &str) -> TextPlan {
    let (mut eingerichtet, mut belegt_liste, mut zeilen) = (vec![], vec![], vec![]);
    for (i, (name, _)) in KUERZEL.iter().enumerate() {
        if belegt(t, i, config) || belegt(t, i, andere) {
            belegt_liste.push(*name);
        } else {
            eingerichtet.push(*name);
            zeilen.push(zeile(t, i, bin));
        }
    }
    let kopf = "# tippkraft: Tastenkürzel (von `tippkraft setup` angelegt, darf geändert werden)\n";
    let datei = match t {
        Text::Niri => format!("// {}binds {{\n{}\n}}\n", &kopf[2..], zeilen.join("\n")),
        _ => format!("{kopf}{}\n", zeilen.join("\n")),
    };
    let include = datei_include(t);
    let schon = config.lines().any(|l| l.trim() == include || (t == Text::Niri && l.contains("\"tippkraft-tasten.kdl\"")));
    let config = (!schon).then(|| format!("{}{}\n# tippkraft\n{include}\n", config, if config.ends_with('\n') || config.is_empty() { "" } else { "\n" }).replace("# tippkraft\n", if t == Text::Niri { "// tippkraft\n" } else { "# tippkraft\n" }));
    TextPlan { datei, config, eingerichtet, belegt: belegt_liste }
}

fn datei_include(t: Text) -> &'static str {
    datei(t).1
}

/// Führt ein Programm aus, liefert stdout bei Erfolg (im Test eine Attrappe)
pub trait Lauf {
    fn lauf(&mut self, prog: &str, args: &[&str]) -> Option<String>;
}

pub struct System;
impl Lauf for System {
    fn lauf(&mut self, prog: &str, args: &[&str]) -> Option<String> {
        let o = std::process::Command::new(prog).args(args).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    }
}

/// Ergebnis: eingerichtet bzw. schon vorhanden, und belegte (ausgelassene) Kürzel
#[derive(Debug, Default, PartialEq)]
pub struct Ergebnis {
    pub eingerichtet: Vec<&'static str>,
    pub belegt: Vec<&'static str>,
}

/// GVariant-Liste von Strings ("@as []", "['a', 'b']") zerlegen
fn liste(s: &str) -> Vec<String> {
    s.split('\'').skip(1).step_by(2).map(String::from).collect()
}
fn gv_liste(v: &[String]) -> String {
    format!("[{}]", v.iter().map(|x| format!("'{x}'")).collect::<Vec<_>>().join(", "))
}

/// GNOME und Cinnamon: eigene Tastenkürzel in dconf über gsettings
pub fn gsettings(l: &mut dyn Lauf, cinnamon: bool, bin: &str) -> Option<Ergebnis> {
    let (schema, schluessel, basis) = if cinnamon {
        ("org.cinnamon.desktop.keybindings", "custom-list", "/org/cinnamon/desktop/keybindings/custom-keybindings/")
    } else {
        ("org.gnome.settings-daemon.plugins.media-keys", "custom-keybindings", "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/")
    };
    let einzel = format!("{schema}.custom-keybinding");
    // GNOME speichert Pfade, Cinnamon nur Namen
    let pfad = |e: &str| if cinnamon { format!("{basis}{e}/") } else { e.to_string() };
    let mut eintraege = liste(&l.lauf("gsettings", &["get", schema, schluessel])?);
    let kombis: Vec<String> = eintraege
        .iter()
        .filter_map(|e| l.lauf("gsettings", &["get", &format!("{einzel}:{}", pfad(e)), "binding"]))
        .map(|b| b.to_lowercase())
        .collect();
    let mut erg = Ergebnis::default();
    for (i, (name, befehl)) in KUERZEL.iter().enumerate() {
        let kombi = ["<super><alt>t", "<super><alt>space"][i];
        let eigener = if cinnamon { format!("tippkraft-{name}") } else { format!("{basis}tippkraft-{name}/") };
        if eintraege.contains(&eigener) {
            erg.eingerichtet.push(name);
            continue;
        }
        if kombis.iter().any(|b| b.contains(kombi)) {
            erg.belegt.push(name);
            continue;
        }
        let ort = format!("{einzel}:{}", pfad(&eigener));
        let binding = if cinnamon { format!("['{}']", ["<Super><Alt>t", "<Super><Alt>space"][i]) } else { ["<Super><Alt>t", "<Super><Alt>space"][i].to_string() };
        l.lauf("gsettings", &["set", &ort, "name", &format!("tippkraft {name}")])?;
        l.lauf("gsettings", &["set", &ort, "command", &format!("{bin} {befehl}")])?;
        l.lauf("gsettings", &["set", &ort, "binding", &binding])?;
        eintraege.push(eigener);
        erg.eingerichtet.push(name);
    }
    l.lauf("gsettings", &["set", schema, schluessel, &gv_liste(&eintraege)])?;
    Some(erg)
}

/// Xfce: Anwendungskürzel in xfconf (Kanal xfce4-keyboard-shortcuts)
pub fn xfce(l: &mut dyn Lauf, bin: &str) -> Option<Ergebnis> {
    let mut erg = Ergebnis::default();
    for (i, (name, befehl)) in KUERZEL.iter().enumerate() {
        let p = format!("/commands/custom/{}", ["<Super><Alt>t", "<Super><Alt>space"][i]);
        match l.lauf("xfconf-query", &["-c", "xfce4-keyboard-shortcuts", "-p", &p]) {
            Some(v) if v.contains("tippkraft") => erg.eingerichtet.push(name),
            Some(_) => erg.belegt.push(name),
            None => {
                l.lauf("xfconf-query", &["-c", "xfce4-keyboard-shortcuts", "-p", &p, "-n", "-t", "string", "-s", &format!("{bin} {befehl}")])?;
                erg.eingerichtet.push(name);
            }
        }
    }
    Some(erg)
}

/// KDE Plasma: je Kürzel eine .desktop-Datei mit X-KDE-Shortcuts, danach kbuildsycoca6, dann gilt es sofort
/// (in der Test-VM gemessen; eine Registrierung nur über D-Bus löst bei Tastendruck nichts aus)
pub fn kde(l: &mut dyn Lauf, home: &std::path::Path, bin: &str) -> Option<Ergebnis> {
    let mut erg = Ergebnis::default();
    let ordner = home.join(".local/share/applications");
    std::fs::create_dir_all(&ordner).ok()?;
    for (i, (name, befehl)) in KUERZEL.iter().enumerate() {
        let datei = ordner.join(format!("tippkraft-{name}.desktop"));
        if datei.exists() {
            erg.eingerichtet.push(name);
            continue;
        }
        // Qt-Tastencode: Meta | Alt | Taste
        let code = 0x1000_0000 | 0x0800_0000 | [0x54, 0x20][i];
        let frei = l
            .lauf("busctl", &["--user", "call", "org.kde.kglobalaccel", "/kglobalaccel", "org.kde.KGlobalAccel", "isGlobalShortcutAvailable", "is", &code.to_string(), &format!("tippkraft-{name}.desktop")])
            .is_none_or(|a| a.contains("true"));
        if !frei {
            erg.belegt.push(name);
            continue;
        }
        let kombi = ["Meta+Alt+T", "Meta+Alt+Space"][i];
        std::fs::write(&datei, format!("[Desktop Entry]\nType=Application\nName=tippkraft {name}\nExec={bin} {befehl}\nIcon=input-keyboard\nNoDisplay=true\nX-KDE-Shortcuts={kombi}\n")).ok()?;
        erg.eingerichtet.push(name);
    }
    l.lauf("kbuildsycoca6", &[]);
    Some(erg)
}

/// Text-Compositor einrichten: eigene Datei schreiben, Include ergänzen. `endung`: Dateien, die als Konfiguration gelten.
fn text_einrichten(t: Text, ordner: &std::path::Path, haupt: &str, endung: &str, bin: &str) -> Result<Ergebnis, String> {
    let haupt_pfad = ordner.join(haupt);
    let config = std::fs::read_to_string(&haupt_pfad).map_err(|_| format!("{} fehlt", haupt_pfad.display()))?;
    let (eigen, _) = datei(t);
    // belegte Kombinationen auch in anderen Dateien suchen (z. B. eine von Hand angelegte tippkraft.kdl)
    let andere: String = std::fs::read_dir(ordner)
        .map(|it| {
            it.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == endung) && p.file_name().is_some_and(|n| n != eigen && n != haupt))
                .filter_map(|p| std::fs::read_to_string(p).ok())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let plan = text_plan(t, &config, &andere, bin);
    if plan.eingerichtet.is_empty() {
        return Ok(Ergebnis { eingerichtet: vec![], belegt: plan.belegt });
    }
    std::fs::write(ordner.join(eigen), &plan.datei).map_err(|e| e.to_string())?;
    if let Some(neu) = plan.config {
        std::fs::write(&haupt_pfad, neu).map_err(|e| e.to_string())?;
    }
    Ok(Ergebnis { eingerichtet: plan.eingerichtet, belegt: plan.belegt })
}

/// Richtet die Kürzel für die laufende Sitzung ein. Err: hier nicht automatisch möglich (dann Zeilen ausgeben).
pub fn einrichten(env: impl Fn(&str) -> Option<String>, home: &std::path::Path, bin: &str, l: &mut dyn Lauf) -> Result<Ergebnis, String> {
    use crate::fokus::Quelle;
    let cfg = home.join(".config");
    match crate::fokus::erkennen(&env) {
        Some(Quelle::Niri(_)) => text_einrichten(Text::Niri, &cfg.join("niri"), "config.kdl", "kdl", bin),
        Some(Quelle::Sway(_)) => {
            let e = text_einrichten(Text::Sway, &cfg.join("sway"), "config", "conf", bin)?;
            l.lauf("swaymsg", &["reload"]);
            Ok(e)
        }
        Some(Quelle::Hyprland(_)) if cfg.join("hypr/hyprland.lua").exists() => Err("hyprland.lua".into()),
        Some(Quelle::Hyprland(_)) => text_einrichten(Text::Hyprland, &cfg.join("hypr"), "hyprland.conf", "conf", bin),
        Some(Quelle::Kde) => kde(l, home, bin).ok_or_else(|| "KDE".into()),
        Some(Quelle::X11) => {
            let d = env("XDG_CURRENT_DESKTOP").unwrap_or_default().to_uppercase();
            if d.contains("XFCE") {
                xfce(l, bin).ok_or_else(|| "xfconf-query".into())
            } else if d.contains("CINNAMON") {
                gsettings(l, true, bin).ok_or_else(|| "gsettings".into())
            } else {
                Err(d)
            }
        }
        None if crate::uinput::sitzung(&env) == crate::uinput::Sitzung::Gnome => gsettings(l, false, bin).ok_or_else(|| "gsettings".into()),
        None => Err("unbekannter Desktop".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn niri_eigene_datei_und_include() {
        let p = text_plan(Text::Niri, "input {\n}\nbinds {\n    Mod+Q { close-window; }\n}\n", "", "/home/x/.local/bin/tippkraft");
        assert!(p.datei.contains("binds {\n    Mod+Alt+T hotkey-overlay-title=\"tippkraft: pause\" { spawn \"/home/x/.local/bin/tippkraft\" \"pause\"; }"));
        assert!(p.datei.contains("Mod+Alt+Space"));
        assert!(p.config.unwrap().ends_with("// tippkraft\ninclude \"tippkraft-tasten.kdl\"\n"));
        assert_eq!(p.eingerichtet, ["pause", "overview"]);
    }

    #[test]
    fn include_nicht_doppelt_und_belegte_kombination_auslassen() {
        let cfg = "binds {\n    Mod+Alt+T { spawn \"foot\"; }\n}\ninclude optional=true \"tippkraft-tasten.kdl\"\n";
        let p = text_plan(Text::Niri, cfg, "", "tk");
        assert!(p.config.is_none());
        assert_eq!(p.belegt, ["pause"]);
        assert!(!p.datei.contains("Mod+Alt+T"));
        assert!(p.datei.contains("Mod+Alt+Space"));
    }

    #[test]
    fn sway_und_hyprland() {
        let s = text_plan(Text::Sway, "set $mod Mod4\nbindsym $mod+Alt+space exec rofi\n", "", "tk");
        assert_eq!(s.belegt, ["overview"]);
        assert_eq!(s.datei.lines().last(), Some("bindsym Mod4+Mod1+t exec tk pause"));
        assert!(s.config.unwrap().ends_with("include ~/.config/sway/tippkraft-tasten.conf\n"));
        let h = text_plan(Text::Hyprland, "bind = SUPER, Q, killactive\n", "", "tk");
        assert!(h.datei.contains("bind = SUPER ALT, T, exec, tk pause") && h.datei.contains("bind = SUPER ALT, SPACE, exec, tk overview"));
        assert!(h.config.unwrap().ends_with("source = ~/.config/hypr/tippkraft-tasten.conf\n"));
        // zweiter Lauf: Include schon da, die eigene Datei liest der Aufrufer nicht mit
        let h2 = text_plan(Text::Hyprland, "bind = SUPER, Q, killactive\n# tippkraft\nsource = ~/.config/hypr/tippkraft-tasten.conf\n", "", "tk");
        assert!(h2.config.is_none() && h2.belegt.is_empty());
    }

    #[test]
    fn kuerzel_in_anderer_datei_sind_belegt() {
        // wie beim Nutzer: Kürzel von Hand in ~/.config/niri/tippkraft.kdl
        let andere = "binds {\n    Mod+Alt+T hotkey-overlay-title=\"tippkraft pausieren\" { spawn \"/home/mg/.local/bin/tippkraft\" \"pause\"; }\n    Mod+Alt+Space { spawn \"tk\" \"uebersicht\"; }\n}\n";
        let p = text_plan(Text::Niri, "binds {\n}\n", andere, "tk");
        assert_eq!(p.belegt, ["pause", "overview"]);
        assert!(p.eingerichtet.is_empty());
    }

    /// Attrappe für gsettings und xfconf-query mit Zustand
    #[derive(Default)]
    struct Attrappe {
        werte: std::collections::HashMap<String, String>,
        log: Vec<String>,
    }
    impl Lauf for Attrappe {
        fn lauf(&mut self, prog: &str, args: &[&str]) -> Option<String> {
            self.log.push(format!("{prog} {}", args.join(" ")));
            match (prog, args) {
                ("gsettings", ["get", schema, key]) => Some(self.werte.get(&format!("{schema} {key}")).cloned().unwrap_or_else(|| "@as []".into())),
                ("gsettings", ["set", schema, key, wert]) => {
                    self.werte.insert(format!("{schema} {key}"), wert.to_string());
                    Some(String::new())
                }
                ("xfconf-query", [_, _, _, p]) => self.werte.get(*p).cloned(),
                ("xfconf-query", [_, _, _, p, .., wert]) => {
                    self.werte.insert(p.to_string(), wert.to_string());
                    Some(String::new())
                }
                _ => None,
            }
        }
    }

    #[test]
    fn gnome_legt_beide_an_laesst_belegte_aus_und_ist_wiederholbar() {
        let mut a = Attrappe::default();
        let s = "org.gnome.settings-daemon.plugins.media-keys";
        a.werte.insert(format!("{s} custom-keybindings"), "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/']".into());
        a.werte.insert(format!("{s}.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/ binding"), "'<Super><Alt>space'".into());
        let e = gsettings(&mut a, false, "tk").unwrap();
        assert_eq!(e, Ergebnis { eingerichtet: vec!["pause"], belegt: vec!["overview"] });
        let ort = format!("{s}.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/tippkraft-pause/");
        assert_eq!(a.werte[&format!("{ort} command")], "tk pause");
        assert_eq!(a.werte[&format!("{ort} binding")], "<Super><Alt>t");
        assert_eq!(a.werte[&format!("{s} custom-keybindings")], "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/', '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/tippkraft-pause/']");
        // zweiter Lauf: nichts doppelt
        let e2 = gsettings(&mut a, false, "tk").unwrap();
        assert_eq!(e2.eingerichtet, ["pause"]);
        assert_eq!(liste(&a.werte[&format!("{s} custom-keybindings")]).len(), 2);
    }

    #[test]
    fn cinnamon_mit_namen_und_binding_als_liste() {
        let mut a = Attrappe::default();
        let e = gsettings(&mut a, true, "tk").unwrap();
        assert_eq!(e.eingerichtet, ["pause", "overview"]);
        assert_eq!(a.werte["org.cinnamon.desktop.keybindings custom-list"], "['tippkraft-pause', 'tippkraft-overview']");
        assert_eq!(a.werte["org.cinnamon.desktop.keybindings.custom-keybinding:/org/cinnamon/desktop/keybindings/custom-keybindings/tippkraft-overview/ binding"], "['<Super><Alt>space']");
    }

    #[test]
    fn xfce_legt_an_und_laesst_fremde_belegung_stehen() {
        let mut a = Attrappe::default();
        a.werte.insert("/commands/custom/<Super><Alt>t".into(), "xfce4-terminal".into());
        let e = xfce(&mut a, "tk").unwrap();
        assert_eq!(e, Ergebnis { eingerichtet: vec!["overview"], belegt: vec!["pause"] });
        assert_eq!(a.werte["/commands/custom/<Super><Alt>space"], "tk overview");
        assert_eq!(a.werte["/commands/custom/<Super><Alt>t"], "xfce4-terminal");
    }

    #[test]
    fn kde_desktop_dateien_mit_kuerzel_belegte_auslassen() {
        let home = std::env::temp_dir().join(format!("tk-kde-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        struct K(Vec<String>);
        impl Lauf for K {
            fn lauf(&mut self, prog: &str, args: &[&str]) -> Option<String> {
                self.0.push(format!("{prog} {}", args.join(" ")));
                // Meta+Alt+Leertaste ist belegt
                Some(if args.contains(&"402653216") { "b false".into() } else { "b true".into() })
            }
        }
        let mut l = K(vec![]);
        let e = kde(&mut l, &home, "tk").unwrap();
        assert_eq!(e, Ergebnis { eingerichtet: vec!["pause"], belegt: vec!["overview"] });
        let d = std::fs::read_to_string(home.join(".local/share/applications/tippkraft-pause.desktop")).unwrap();
        assert!(d.contains("Exec=tk pause") && d.contains("X-KDE-Shortcuts=Meta+Alt+T"));
        assert!(l.0.iter().any(|c| c.contains("402653268")) && l.0.last().unwrap().starts_with("kbuildsycoca6"));
        // zweiter Lauf: vorhanden
        assert_eq!(kde(&mut l, &home, "tk").unwrap().eingerichtet, ["pause"]);
        let _ = std::fs::remove_dir_all(&home);
    }
}
