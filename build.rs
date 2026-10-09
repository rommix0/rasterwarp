//! Copies the FFmpeg DLLs that capture, clips and cameras link against (and the ones those
//! load in turn, such as avdevice's avfilter) next to the built executables, so
//! `cargo run`, `cargo test` and the exe in `target/<profile>/` all find them at startup.
//! Without them Windows exits the program with 0xC0000135 and no message. Also links
//! `rasterwarp.rc` (the icon) into the app.

use std::path::{Path, PathBuf};
use std::{env, fs};

const DLLS: [&str; 7] = [
    "avcodec-63.dll",
    "avdevice-63.dll",
    "avfilter-12.dll",
    "avformat-63.dll",
    "avutil-61.dll",
    "swscale-10.dll",
    "swresample-7.dll",
];

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=rasterwarp.rc");
    println!("cargo:rerun-if-changed=icon/rasterwarp.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // The icon (rasterwarp.rc) goes into the app only, not the test binaries.
    embed_resource::compile_for("rasterwarp.rc", ["rasterwarp"], embed_resource::NONE)
        .manifest_optional()
        .expect("compile rasterwarp.rc");
    let Ok(ffmpeg_dir) = env::var("FFMPEG_DIR") else {
        panic!("FFMPEG_DIR is not set; see .cargo/config.toml");
    };
    let bin = Path::new(&ffmpeg_dir).join("bin");
    // OUT_DIR is target/<profile>/build/<crate>-<hash>/out.
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR is inside target/<profile>")
        .to_path_buf();
    for dll in DLLS {
        let src = bin.join(dll);
        assert!(
            src.is_file(),
            "{} is missing; FFMPEG_DIR must point at a shared FFmpeg 9 build",
            src.display()
        );
        for dir in [profile_dir.clone(), profile_dir.join("deps")] {
            copy_if_changed(&src, &dir.join(dll));
        }
    }
}

fn copy_if_changed(src: &Path, dst: &Path) {
    let same = match (fs::metadata(src), fs::metadata(dst)) {
        (Ok(a), Ok(b)) => a.len() == b.len(),
        _ => false,
    };
    if !same {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).expect("create target directory");
        }
        fs::copy(src, dst).unwrap_or_else(|e| panic!("copy {} failed: {e}", src.display()));
    }
}
