//! Embed the application icon into the Windows exe (RT_ICON + RT_GROUP_ICON).
//!
//! Prefers the real resource compiler (rc.exe from the Visual Studio
//! installation, found via PATH or vswhere) and falls back to llvm-rc/windres.
//! If no compiler is available the build succeeds without an icon. Other
//! targets skip resource embedding entirely.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn find_rc() -> Option<PathBuf> {
    // 1. plain PATH lookup
    for cand in ["rc.exe", "llvm-rc.exe", "llvm-rc"] {
        if let Ok(out) = Command::new("where").arg(cand).output() {
            if out.status.success() {
                let line = String::from_utf8_lossy(&out.stdout);
                if let Some(p) = line.lines().next() {
                    let p = PathBuf::from(p.trim());
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    // 2. vswhere -> newest VC tools rc.exe (windows-latest runners)
    let vswhere = PathBuf::from(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
    if vswhere.exists() {
        if let Ok(out) = Command::new(&vswhere)
            .args([
                "-latest",
                "-products",
                "*",
                "-requires",
                "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                "-property",
                "installationPath",
            ])
            .output()
        {
            if out.status.success() {
                let vs = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let tools = PathBuf::from(&vs).join("VC").join("Tools").join("MSVC");
                if let Ok(entries) = std::fs::read_dir(&tools) {
                    let mut vers: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
                    vers.sort();
                    for v in vers.into_iter().rev() {
                        let rc = v.join("bin").join("Hostx64").join("x64").join("rc.exe");
                        if rc.exists() {
                            return Some(rc);
                        }
                    }
                }
            }
        }
    }
    // 3. windres (mingw builds)
    for cand in ["windres", "x86_64-w64-mingw32-windres"] {
        if let Ok(out) = Command::new("where").arg(cand).output() {
            if out.status.success() {
                let line = String::from_utf8_lossy(&out.stdout);
                if let Some(p) = line.lines().next() {
                    let p = PathBuf::from(p.trim());
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let ico = manifest.join("../app/icons/icon.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    if !ico.exists() {
        println!("cargo:warning=icon.ico not found — exe will have no icon");
        return;
    }
    let Some(rc) = find_rc() else {
        println!("cargo:warning=no rc.exe/llvm-rc/windres found — exe will have no icon");
        return;
    };

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc_path = out_dir.join("app.rc");
    let res_path = out_dir.join("app.res");
    // canonicalize() yields a UNC \\?\ path which rc.exe refuses to open —
    // strip the prefix (the reason the v0.1.6 embed silently produced an
    // icon-less exe). Failures below are FATAL: the icon is a hard
    // requirement and a silent skip shipped icon-less builds twice already.
    let ico_abs = ico.canonicalize().unwrap_or(ico.clone());
    let ico_disp = ico_abs.to_string_lossy().into_owned();
    let ico_disp = ico_disp
        .strip_prefix("\\\\?\\")
        .unwrap_or(&ico_disp)
        .replace('\\', "\\\\");
    std::fs::write(&rc_path, format!("1 ICON \"{ico_disp}\"\n")).expect("write app.rc");

    let name = rc.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    let ok = if name.contains("windres") {
        Command::new(&rc)
            .args(["-i", "app.rc", "-o", "app.res", "-O", "coff"])
            .current_dir(&out_dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        Command::new(&rc)
            .args(["/nologo", "/fo", "app.res", "app.rc"])
            .current_dir(&out_dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if !ok || !res_path.exists() {
        println!(
            "::error::build.rs: resource compiler {} failed to build app.rc — exe would have no icon",
            rc.display()
        );
        panic!("resource compiler failed (rc={})", rc.display());
    }
    // Diagnostics that survive into the CI annotations (readable via API).
    println!(
        "::notice::build.rs: icon embedded via {} from {}",
        rc.display(),
        ico_abs.display()
    );
    // `rustc-link-arg-bin` is supported on every cargo we care about; the
    // package's only bin target is `tsn-native`
    println!("cargo:rustc-link-arg-bin=tsn-native={}", res_path.display());
}
