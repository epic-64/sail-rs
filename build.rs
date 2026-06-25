//! Build script — prepares the web (wasm) audio at build time.
//!
//! The native build embeds the full-quality clips from `assets/sounds/`. The web
//! build downloads the whole wasm up front, so it embeds *re-encoded* (smaller)
//! copies instead — but we don't want those copies committed to the repo. So this
//! script generates them into `OUT_DIR/sounds-web/` during the wasm build, and
//! `sound.rs` `include_bytes!`s them from there (see the `snd!` macro).
//!
//! Re-encoding uses `ffmpeg` if it can be found (on PATH or vendored under
//! `.tools/`). It is best-effort and NEVER fails the build: if ffmpeg is missing,
//! a conversion errors, or the re-encode comes out larger than the original, the
//! original clip is copied through unchanged and a `cargo:warning` is emitted.
//! Worst case the web build just ships the full-size audio.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

// (filename, channels, bitrate) — ambience/SFX to mono 80k, the sailing music to
// stereo 112k. Per file we keep whichever is smaller (original vs re-encoded), so
// clips already below target stay as-is.
const CLIPS: &[(&str, u8, &str)] = &[
    ("dammafra-sailing-435998.mp3", 2, "112k"),
    ("calm.mp3", 1, "80k"),
    ("thunderstorm-cut.mp3", 1, "80k"),
    ("universfield-transition-02-141076.mp3", 1, "80k"),
    ("flap1.mp3", 1, "80k"),
    ("flap2.mp3", 1, "80k"),
    ("collect-coin.mp3", 1, "80k"),
    ("pw23check-winning-218995.mp3", 1, "80k"),
    ("lightyeartraxx-kl-peach-game-over-iii-142453.mp3", 1, "80k"),
    ("invalid-input.mp3", 1, "80k"),
];

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    println!("cargo:rerun-if-changed=build.rs");
    for (name, _, _) in CLIPS {
        println!("cargo:rerun-if-changed=assets/sounds/{name}");
    }

    // Only the web build needs the re-encoded copies; native embeds the
    // originals directly and never references OUT_DIR.
    if env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("wasm32") {
        return;
    }

    let src_dir = manifest.join("assets/sounds");
    let web_dir = PathBuf::from(env::var("OUT_DIR").unwrap()).join("sounds-web");
    fs::create_dir_all(&web_dir).expect("create OUT_DIR/sounds-web");

    let ffmpeg = find_ffmpeg(&manifest);
    if ffmpeg.is_none() {
        println!(
            "cargo:warning=ffmpeg not found (PATH or .tools/) — web build will embed \
             full-quality audio (larger wasm). Install ffmpeg to shrink it."
        );
    }

    for (name, channels, bitrate) in CLIPS {
        let src = src_dir.join(name);
        let dst = web_dir.join(name);
        let mut staged = false;

        if let Some(ff) = &ffmpeg {
            let tmp = web_dir.join(format!("__tmp_{name}"));
            let result = Command::new(ff)
                .args(["-y", "-loglevel", "error", "-i"])
                .arg(&src)
                .args(["-ac", &channels.to_string(), "-c:a", "libmp3lame", "-b:a", bitrate])
                .arg(&tmp)
                .status();
            match result {
                Ok(s) if s.success() => {
                    let orig = fs::metadata(&src).map(|m| m.len()).unwrap_or(u64::MAX);
                    let new = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(u64::MAX);
                    if new <= orig {
                        // Re-encode is smaller — use it.
                        if fs::rename(&tmp, &dst).is_ok() {
                            staged = true;
                        }
                    } else {
                        // Original already smaller — drop the re-encode.
                        let _ = fs::remove_file(&tmp);
                    }
                }
                _ => {
                    let _ = fs::remove_file(&tmp);
                    println!("cargo:warning=ffmpeg could not convert {name} — embedding original");
                }
            }
        }

        if !staged {
            // Fall back to the original so include_bytes! always resolves.
            if let Err(e) = fs::copy(&src, &dst) {
                println!("cargo:warning=could not stage web audio {name}: {e}");
            }
        }
    }
}

/// Locate an ffmpeg binary: first on PATH, then any vendored copy under `.tools/`.
fn find_ffmpeg(manifest: &PathBuf) -> Option<PathBuf> {
    let on_path = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if on_path {
        return Some(PathBuf::from("ffmpeg"));
    }

    let tools = manifest.join(".tools");
    if let Ok(entries) = fs::read_dir(&tools) {
        for entry in entries.flatten() {
            for exe in ["ffmpeg.exe", "ffmpeg"] {
                let p = entry.path().join("bin").join(exe);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}
