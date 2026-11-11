//! Files commands read are size-checked before they are read, and saves leave no predictable temp file.
#![cfg(not(target_arch = "wasm32"))]

use anicraft_engine::Session;
use serde_json::json;

#[test]
fn import_to_library_checks_an_image_size_before_reading() {
    let dir = std::env::temp_dir().join(format!("anicraft-readbounds-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let big = dir.join("big.png");
    // A sparse file: the size is claimed without being stored (or read, if the check works).
    std::fs::File::create(&big).unwrap().set_len(256 * 1024 * 1024 + 1).unwrap();
    let mut s = Session::default();
    let e = s.execute("file.import_to_library", &json!({"path": big})).unwrap_err().to_string();
    assert!(e.contains("too large"), "{e}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn save_does_not_write_through_a_planted_predictable_temp_path() {
    let dir = std::env::temp_dir().join(format!("anicraft-savetmp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("a.anicraft");
    let victim = dir.join("victim.txt");
    std::fs::write(&victim, "keep").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&victim, dir.join("a.anicraft.tmp")).unwrap();
    anicraft_engine::format::save(&Session::default().doc, &target).unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
    assert!(target.is_file());
    std::fs::remove_dir_all(&dir).ok();
}
