//! Abstimmung mit der tippkraft-Browser-Erweiterung: Ist sie in einem Browser auf diesem Rechner installiert,
//! ersetzt in dessen Fenstern nur die Erweiterung, die Desktop-App hält sich dort heraus (sonst doppelter Text).
//! Erkannt wird an den Profilordnern, die Erweiterung selbst bleibt unverändert.
use std::fs;
use std::path::{Path, PathBuf};

/// ID der Firefox-Erweiterung (browser/manifest.json, browser_specific_settings.gecko.id)
const FIREFOX_ID: &str = "browser@tippkraft.com";

/// Firefox-Profilwurzeln: klassisch, XDG (ab Firefox 147), Snap, Flatpak
const FIREFOX_WURZELN: &[&str] = &[".mozilla/firefox", ".config/mozilla/firefox", "snap/firefox/common/.mozilla/firefox", ".var/app/org.mozilla.firefox/.mozilla/firefox", ".var/app/org.mozilla.firefox/config/mozilla/firefox"];
const FIREFOX_APPS: &[&str] = &["firefox", "org.mozilla.firefox", "firefox-esr", "firefox-developer-edition", "firefox-nightly", "navigator"];

/// Chromium-Familie: Konfigurationsordner (normal und Flatpak) und App-IDs der Fenster
const CHROMIUM: &[(&[&str], &[&str])] = &[
    (&[".config/google-chrome", ".config/google-chrome-beta", ".var/app/com.google.Chrome/config/google-chrome"], &["google-chrome", "google-chrome-beta", "com.google.chrome"]),
    (&[".config/chromium", "snap/chromium/common/chromium", ".var/app/org.chromium.Chromium/config/chromium"], &["chromium", "chromium-browser", "org.chromium.chromium"]),
    (&[".config/BraveSoftware/Brave-Browser", ".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser"], &["brave-browser", "com.brave.browser"]),
    (&[".config/microsoft-edge", ".var/app/com.microsoft.Edge/config/microsoft-edge"], &["microsoft-edge", "com.microsoft.edge"]),
    (&[".config/vivaldi", ".var/app/com.vivaldi.Vivaldi/config/vivaldi"], &["vivaldi", "vivaldi-stable", "com.vivaldi.vivaldi"]),
];

fn unterordner(p: &Path) -> Vec<PathBuf> {
    fs::read_dir(p).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default()
}

fn firefox_hat(home: &Path) -> bool {
    FIREFOX_WURZELN.iter().flat_map(|w| unterordner(&home.join(w))).any(|profil| {
        let ext = profil.join("extensions");
        ext.join(format!("{FIREFOX_ID}.xpi")).exists() || ext.join(FIREFOX_ID).is_dir()
    })
}

/// Chromium speichert Erweiterungen unter <Profil>/Extensions/<id>/<version>/manifest.json.
/// Die Store-ID kennen wir vorab nicht, deshalb erkennen wir tippkraft an der Adresse im Manifest.
fn chromium_hat(home: &Path, wurzeln: &[&str]) -> bool {
    wurzeln.iter().flat_map(|w| unterordner(&home.join(w))).any(|profil| {
        unterordner(&profil.join("Extensions")).iter().flat_map(|id| unterordner(id)).any(|version| {
            fs::read_to_string(version.join("manifest.json")).is_ok_and(|m| m.contains("https://tippkraft.com"))
        })
    })
}

/// App-IDs (klein geschrieben) der Browser, in denen die Erweiterung installiert ist
pub fn mit_erweiterung(home: &Path) -> Vec<String> {
    let mut apps: Vec<&str> = vec![];
    if firefox_hat(home) {
        apps.extend(FIREFOX_APPS);
    }
    for (wurzeln, ids) in CHROMIUM {
        if chromium_hat(home, wurzeln) {
            apps.extend(*ids);
        }
    }
    apps.into_iter().map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heim(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("tippkraft-browser-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn ohne_erweiterung_leer() {
        let h = heim("leer");
        fs::create_dir_all(h.join(".mozilla/firefox/abc.default/extensions")).unwrap();
        fs::create_dir_all(h.join(".config/google-chrome/Default/Extensions/xyz/1.0")).unwrap();
        fs::write(h.join(".config/google-chrome/Default/Extensions/xyz/1.0/manifest.json"), r#"{"homepage_url":"https://example.com"}"#).unwrap();
        assert!(mit_erweiterung(&h).is_empty());
    }

    #[test]
    fn firefox_auch_als_snap() {
        let h = heim("snap");
        let ext = h.join("snap/firefox/common/.mozilla/firefox/q1.default-release/extensions");
        fs::create_dir_all(&ext).unwrap();
        fs::write(ext.join("browser@tippkraft.com.xpi"), b"").unwrap();
        let a = mit_erweiterung(&h);
        assert!(a.contains(&"firefox".to_string()) && a.contains(&"org.mozilla.firefox".to_string()));
        assert!(!a.contains(&"google-chrome".to_string()));
    }

    #[test]
    fn chrome_an_der_adresse_im_manifest() {
        let h = heim("chrome");
        let v = h.join(".config/google-chrome/Profile 1/Extensions/abcdefghijklmnopabcdefghijklmnop/0.1.1_0");
        fs::create_dir_all(&v).unwrap();
        fs::write(v.join("manifest.json"), r#"{"homepage_url":"https://tippkraft.com/help/browser-extension.html"}"#).unwrap();
        let a = mit_erweiterung(&h);
        assert!(a.contains(&"google-chrome".to_string()));
        assert!(!a.contains(&"firefox".to_string()) && !a.contains(&"chromium".to_string()));
    }
}
