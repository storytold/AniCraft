fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    // `--screenshot` is a desktop-app feature: hand over to the `anicraft` binary next to us.
    if argv.iter().any(|a| a == "--screenshot") {
        std::process::exit(delegate_to_desktop(&argv));
    }
    match anicraft_cli::run(&argv) {
        Ok(out) => {
            if !out.is_empty() {
                println!("{out}");
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn delegate_to_desktop(argv: &[String]) -> i32 {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(format!("anicraft{}", std::env::consts::EXE_SUFFIX))));
    let Some(exe) = exe.filter(|p| p.exists()) else {
        eprintln!("error: --screenshot needs the AniCraft desktop app (`anicraft`) next to anicraft-cli; build it with `cargo build`");
        return 1;
    };
    match std::process::Command::new(&exe).args(argv).status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("error: could not start {}: {e}", exe.display());
            1
        }
    }
}
