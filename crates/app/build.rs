//! Embeds the TStudio logo as the .exe icon (Explorer, taskbar, shortcuts).
//! Uses rc.exe from the Windows SDK; if it is missing the build just goes on without an exe icon.

#[path = "src/logo.rs"]
mod logo;

use std::path::{Path, PathBuf};

fn find_rc() -> Option<PathBuf> {
    if let Ok(rc) = std::env::var("RC") {
        return Some(PathBuf::from(rc));
    }
    let base = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(base).ok()?.flatten().map(|e| e.path()).filter(|p| p.join("x64").join("rc.exe").exists()).collect();
    versions.sort();
    versions.pop().map(|p| p.join("x64").join("rc.exe"))
}

fn main() {
    println!("cargo:rerun-if-changed=src/logo.rs");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") || std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("tstudio.ico");
    std::fs::write(&ico, logo::ico(&[16, 24, 32, 48, 64, 256])).unwrap();
    let rc_file = out.join("tstudio.rc");
    std::fs::write(&rc_file, format!("1 ICON \"{}\"\n", ico.display().to_string().replace('\\', "\\\\"))).unwrap();
    let res = out.join("tstudio.res");
    let Some(rc) = find_rc() else {
        println!("cargo:warning=rc.exe nicht gefunden – tstudio.exe bekommt kein Datei-Icon");
        return;
    };
    let ok = std::process::Command::new(rc).arg("/nologo").arg("/fo").arg(&res).arg(&rc_file).status().map(|s| s.success()).unwrap_or(false);
    if ok {
        println!("cargo:rustc-link-arg-bins={}", res.display());
    } else {
        println!("cargo:warning=rc.exe fehlgeschlagen – tstudio.exe bekommt kein Datei-Icon");
    }
}
