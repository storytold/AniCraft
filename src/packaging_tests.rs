//! The installers in `packaging/` agree with the app (ADR 0026): one app id everywhere, every
//! document type the Open dialog accepts is registered on each OS, the Windows registry
//! inventory the installer-policy check guards matches the WiX source, and every icon file the
//! packaging names exists. These run in `cargo test`, so a change on one side that forgets the
//! other fails on any OS, not only in the platform's release job.

use crate::platform::{APP_ID, filters};
use std::path::Path;

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).replace("\r\n", "\n")
}

/// The value of `<key>name</key>` followed by `<string>value</string>` in a plist.
fn plist_string(plist: &str, key: &str) -> Option<String> {
    let rest = &plist[plist.find(&format!("<key>{key}</key>"))?..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")?;
    Some(rest[start..start + end].to_string())
}

/// `Key=value` from a desktop entry.
fn desktop_value(entry: &str, key: &str) -> Option<String> {
    entry.lines().find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_string))
}

#[test]
fn one_app_id_everywhere() {
    let linux = format!("packaging/linux/{APP_ID}");
    let desktop = read(&format!("{linux}.desktop"));
    assert_eq!(desktop_value(&desktop, "StartupWMClass").as_deref(), Some(APP_ID), "the window's app id must match the desktop entry");
    assert_eq!(desktop_value(&desktop, "Icon").as_deref(), Some(APP_ID));
    let metainfo = read(&format!("{linux}.metainfo.xml.in"));
    assert!(metainfo.contains(&format!("<id>{APP_ID}</id>")));
    assert!(metainfo.contains(&format!("<launchable type=\"desktop-id\">{APP_ID}.desktop</launchable>")));
    assert!(read(&format!("{linux}.mime.xml")).contains(&format!("<icon name=\"{APP_ID}\"/>")));
    for manifest in [format!("packaging/linux/flatpak/{APP_ID}.yml"), format!("packaging/linux/flatpak/{APP_ID}.bundle.yml")] {
        assert!(read(&manifest).lines().any(|l| l == format!("id: {APP_ID}")), "{manifest}");
    }
    assert!(read("packaging/env.sh").contains(&format!("export APP_ID={APP_ID}\n")));
    let plist = read("packaging/macos/Info.plist.in");
    assert_eq!(plist_string(&plist, "CFBundleIdentifier").as_deref(), Some(APP_ID));
    assert!(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("assets/app-icon/hicolor/256x256/apps/{APP_ID}.png")).is_file());
}

#[test]
fn every_openable_document_type_is_registered() {
    let (_, extensions) = filters("file.open");
    assert!(extensions.contains(&"anicraft"));
    let wxs = read("packaging/windows/anicraft.wxs");
    let plist = read("packaging/macos/Info.plist.in");
    let mime = read(&format!("packaging/linux/{APP_ID}.mime.xml"));
    let desktop_types = desktop_value(&read(&format!("packaging/linux/{APP_ID}.desktop")), "MimeType").unwrap_or_default();
    for ext in extensions {
        assert!(wxs.contains(&format!("SupportedTypes\" Name=\".{ext}\"")), "Windows: .{ext} is not in Open with");
        assert!(plist.contains(&format!("<array><string>{ext}</string></array>")), "macOS: .{ext} has no type declaration");
        let glob = format!("<glob pattern=\"*.{ext}\"");
        let at = mime.find(&glob).unwrap_or_else(|| panic!("Linux: no MIME type has *.{ext}"));
        let open = mime[..at].rfind("<mime-type type=\"").map(|i| i + "<mime-type type=\"".len()).unwrap_or(0);
        let mime_type = &mime[open..open + mime[open..].find('"').unwrap_or(0)];
        assert!(desktop_types.split(';').any(|t| t == mime_type), "Linux: the desktop entry doesn't list {mime_type} (.{ext})");
    }
    // .anicraft is AniCraft's own: the default app on each OS, never only an alternative.
    assert!(wxs.contains("<Extension Id=\"anicraft\" ContentType=\"application/x-anicraft\">"));
    assert!(plist.contains("<string>Owner</string>"));
}

#[test]
fn windows_registry_inventory_matches_the_installer() {
    let wxs = read("packaging/windows/anicraft.wxs");
    let policy: serde_json::Value = serde_json::from_str(&read("packaging/windows/install-policy.json")).unwrap();
    let mut from_wxs = Vec::new();
    for line in wxs.lines().map(str::trim).filter(|l| l.starts_with("<RegistryValue Root=\"HKMU\"")) {
        let attr = |name: &str| {
            let key = format!(" {name}=\"");
            line.find(&key).map(|i| {
                let v = &line[i + key.len()..];
                v[..v.find('"').unwrap_or(0)].replace("&quot;", "\"")
            })
        };
        from_wxs.push(vec!["-1".to_string(), attr("Key").unwrap_or_default(), attr("Name").unwrap_or_default(), attr("Value").unwrap_or_default()]);
    }
    let from_policy: Vec<Vec<String>> = policy["registry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect())
        .collect();
    assert!(!from_wxs.is_empty());
    assert_eq!(from_wxs, from_policy, "update packaging/windows/install-policy.json with anicraft.wxs");
    assert_eq!(policy["name"], "AniCraft");
    assert!(wxs.contains(&format!("UpgradeCode=\"{}\"", policy["upgradeCode"].as_str().unwrap())));
}

#[test]
fn packaging_icons_exist() {
    let icons = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app-icon");
    for f in ["anicraft.ico", "anicraft-padded-1024.png"] {
        assert!(icons.join(f).is_file(), "{f}");
    }
    for s in [16, 24, 32, 48, 64, 128, 256, 512] {
        assert!(icons.join(format!("hicolor/{s}x{s}/apps/{APP_ID}.png")).is_file(), "{s}");
    }
    assert!(icons.join(format!("hicolor/scalable/apps/{APP_ID}.svg")).is_file());
    // The DMG window's background and layout are committed (packaging/macos/dmg/README.md).
    let dmg = Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/macos/dmg");
    assert!(dmg.join("background.tiff").is_file() && dmg.join("dmg-layout.DS_Store").is_file());
}
