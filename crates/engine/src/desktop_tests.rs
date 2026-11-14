//! AppImage desktop integration (ADR 0094): the desktop entry and MIME files it writes, quoting
//! of awkward paths, idempotent install and remove, and what it refuses.

use super::*;
use crate::cmds_desktop::{APP_ID, MARKER_KEY, desktop_entry, entry_appimage, exec_argument, install, owned_files, read_status, uninstall};
use serde_json::json;
use std::path::{Path, PathBuf};

/// A fresh folder with a fake AppImage in a folder whose name has a space, and a data folder.
fn setup(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("anicraft-desktop-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let apps = root.join("My Apps");
    std::fs::create_dir_all(&apps).unwrap();
    let appimage = apps.join("AniCraft-0.1.0-x86_64.AppImage");
    std::fs::write(&appimage, b"\x7fELF fake").unwrap();
    let data = root.join("share");
    (root, appimage, data)
}

fn value<'a>(entry: &'a str, key: &str) -> Option<&'a str> {
    entry.lines().find_map(|l| l.strip_prefix(&format!("{key}=")))
}

#[test]
fn the_entry_starts_the_appimage_and_keeps_the_packaged_fields() {
    let entry = desktop_entry(Path::new("/home/ana/Apps/AniCraft.AppImage")).unwrap();
    assert!(entry.contains("[Desktop Entry]\n"));
    assert_eq!(value(&entry, "Exec"), Some("\"/home/ana/Apps/AniCraft.AppImage\" %f"));
    assert_eq!(value(&entry, "TryExec"), Some("/home/ana/Apps/AniCraft.AppImage"));
    assert_eq!(value(&entry, MARKER_KEY), Some("/home/ana/Apps/AniCraft.AppImage"));
    // Everything else is the packaged entry, so menus, docks and file types match the packages.
    for key in ["Type", "Name", "Icon", "StartupWMClass", "Categories", "MimeType", "Terminal"] {
        let packaged = include_str!("../../../packaging/linux/ai.storyteller.anicraft.desktop");
        assert_eq!(value(&entry, key), value(packaged, key), "{key}");
    }
    assert_eq!(value(&entry, "Icon"), Some(APP_ID));
    assert_eq!(value(&entry, "MimeType"), Some("application/x-anicraft;application/x-fla;application/x-xfl;"));
    assert_eq!(entry.matches("Exec=").count(), 2, "one Exec and one TryExec");
}

#[test]
fn awkward_paths_are_quoted_as_the_desktop_entry_spec_says() {
    // Spaces and quotes go inside double quotes; ", `, $ and \ are escaped there; % is doubled;
    // then the value's own backslashes are doubled.
    assert_eq!(exec_argument("/home/a b/x.AppImage"), r#""/home/a b/x.AppImage""#);
    assert_eq!(exec_argument(r#"/o'k/"q"/$HOME/`x`/100%"#), r#""/o'k/\\"q\\"/\\$HOME/\\`x\\`/100%%""#);
    assert_eq!(exec_argument(r"/back\slash"), r#""/back\\\\slash""#);
    // The marker reads back exactly, whatever the path.
    for p in ["/home/a b/Ani Craft.AppImage", r"/weird\name/$x%.AppImage", "/ünï/çødé/日本.AppImage"] {
        assert_eq!(entry_appimage(&desktop_entry(Path::new(p)).unwrap()), Some(PathBuf::from(p)), "{p}");
    }
    // Line breaks, relative paths and nothing at all can't be in a desktop entry.
    for bad in ["/a\nExec=evil", "relative/x.AppImage", ""] {
        assert!(desktop_entry(Path::new(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn install_writes_the_entry_mime_types_and_icons_and_is_idempotent() {
    let (root, appimage, data) = setup("install");
    let r = install(&appimage, &data).unwrap();
    assert_eq!(r.written.len(), owned_files().len());
    assert!(r.unchanged.is_empty());
    let entry = std::fs::read_to_string(data.join("applications").join(format!("{APP_ID}.desktop"))).unwrap();
    assert_eq!(entry_appimage(&entry).as_deref(), Some(appimage.as_path()));
    let mime = std::fs::read_to_string(data.join("mime/packages").join(format!("{APP_ID}.xml"))).unwrap();
    for ty in ["application/x-anicraft", "application/x-fla", "application/x-xfl"] {
        assert!(mime.contains(&format!("<mime-type type=\"{ty}\">")), "{ty}");
    }
    for size in ["16x16", "48x48", "256x256"] {
        let icon = std::fs::read(data.join("icons/hicolor").join(size).join("apps").join(format!("{APP_ID}.png"))).unwrap();
        assert_eq!(&icon[..8], b"\x89PNG\r\n\x1a\n", "{size}");
    }
    assert!(data.join("icons/hicolor/scalable/apps").join(format!("{APP_ID}.svg")).is_file());
    // Again: nothing changes.
    let again = install(&appimage, &data).unwrap();
    assert!(again.written.is_empty());
    assert_eq!(again.unchanged.len(), owned_files().len());
    // Moved to another folder: only the entry is rewritten.
    let moved = root.join("AniCraft.AppImage");
    std::fs::copy(&appimage, &moved).unwrap();
    let r = install(&moved, &data).unwrap();
    assert_eq!(r.written.len(), 1);
    assert_eq!(read_status(&data).appimage.as_deref(), Some(moved.as_path()));
    // No temporary files left behind.
    let mut stray = Vec::new();
    for rel in owned_files() {
        let dir = data.join(rel).parent().unwrap().to_path_buf();
        stray.extend(std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".anicraft-tmp")).map(|e| e.path()));
    }
    assert!(stray.is_empty(), "{stray:?}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn remove_deletes_exactly_what_install_wrote_and_is_idempotent() {
    let (root, appimage, data) = setup("remove");
    // Someone else's files beside ours stay.
    let other = data.join("applications").join("org.example.other.desktop");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    std::fs::write(&other, "[Desktop Entry]\nName=Other\n").unwrap();
    install(&appimage, &data).unwrap();
    let removed = uninstall(&data).unwrap();
    assert_eq!(removed.len(), owned_files().len());
    for rel in owned_files() {
        assert!(!data.join(rel).exists());
    }
    assert!(other.is_file());
    assert_eq!(read_status(&data), Default::default());
    // Again: nothing to do, and no error.
    assert!(uninstall(&data).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_entry_anicraft_did_not_write_is_never_replaced_or_removed() {
    let (root, appimage, data) = setup("foreign");
    let entry = data.join("applications").join(format!("{APP_ID}.desktop"));
    std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
    std::fs::write(&entry, "[Desktop Entry]\nName=My own AniCraft\nExec=/opt/mine/anicraft\n").unwrap();
    assert!(install(&appimage, &data).is_err());
    assert!(uninstall(&data).is_err());
    assert!(std::fs::read_to_string(&entry).unwrap().contains("My own AniCraft"));
    assert!(read_status(&data).foreign);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn system_folders_and_missing_appimages_are_refused() {
    let (root, appimage, data) = setup("refuse");
    for system in ["/usr/share", "/usr/local/share", "/etc/xdg", "/opt", "/", "/var/lib/flatpak/exports/share", "/var/lib/snapd/desktop"] {
        assert!(install(&appimage, Path::new(system)).is_err(), "{system}");
        assert!(uninstall(Path::new(system)).is_err(), "{system}");
    }
    assert!(install(&root.join("gone.AppImage"), &data).is_err());
    assert!(install(&appimage, Path::new("relative/share")).is_err());
    assert!(!data.exists(), "nothing was written");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_commands_add_report_and_remove() {
    let (root, appimage, data) = setup("commands");
    let mut s = Session::default();
    let (a, d) = (appimage.display().to_string(), data.display().to_string());
    let st = s.execute("desktop.status", &json!({"data_home": d, "appimage": a})).unwrap();
    assert_eq!((st["added"].as_bool(), st["this_copy"].as_bool()), (Some(false), Some(false)));
    let r = s.execute("desktop.integrate", &json!({"appimage": a, "data_home": d, "refresh": false})).unwrap();
    assert_eq!(r["written"].as_array().map(Vec::len), Some(owned_files().len()));
    let st = s.execute("desktop.status", &json!({"data_home": d, "appimage": a})).unwrap();
    assert_eq!((st["added"].as_bool(), st["this_copy"].as_bool(), st["target_exists"].as_bool()), (Some(true), Some(true), Some(true)));
    assert!(!s.is_dirty(), "the document is untouched");
    let r = s.execute("desktop.remove", &json!({"data_home": d, "refresh": false})).unwrap();
    assert_eq!(r["removed"].as_array().map(Vec::len), Some(owned_files().len()));
    for bad in [json!({}), json!({"appimage": a}), json!({"data_home": ""}), json!({"appimage": "", "data_home": d}), json!({"appimage": 3, "data_home": d})] {
        assert!(s.execute("desktop.integrate", &bad).is_err(), "{bad}");
    }
    assert!(crate::cmds_plugin::plugin_may_call("desktop.integrate").is_err());
    let _ = std::fs::remove_dir_all(root);
}
