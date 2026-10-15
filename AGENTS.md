# AGENTS.md

Rasterwarp is a real-time, Scanimate-style video synthesizer for Windows: Rust 2024, wgpu, egui, winit, cpal, ffmpeg-next and midir. See README.md for what it does.

## Build and check

- Needs the MSVC toolchain, Rust 1.88+, and a shared FFmpeg 9 build in `ffmpeg-9.0.2-full_build-shared/` (gitignored). `.cargo/config.toml` points `FFMPEG_DIR` at it relative to the project. A git worktree has no copy, so set `FFMPEG_DIR` to the main checkout's folder there.
- `build.rs` copies the 7 FFmpeg DLLs next to every built exe and links the icon (`rasterwarp.rc`). If you link a new FFmpeg library, add its DLL to `DLLS`.
- Before committing, run `cargo fmt`, `cargo clippy --all-targets`, and `cargo test` (the lib tests plus `tests/`).
- `dist.ps1` builds a release and copies the exe and DLLs into `dist/` (gitignored).

## Layout

- `src/passes/` holds the render passes (source → warp or raster → colorize → feedback → bloom → composite), and `shaders/` holds their WGSL, embedded with `include_str!`.
- `src/params.rs` holds every live parameter. `src/params/table.rs` is the slider table that MIDI and audio links, transitions and cues all go through.
- `src/blend.rs` turns two parameter banks into one frame's values.
- `src/save.rs` and `src/session.rs` handle settings, presets, projects and the autosave.

## Conventions

- **New parameter:** add the field, range and clamp in `params.rs`; a `SliderId` entry (`float_field`, `all`, `id`, `label`, `range`, and the pinned-id test) in `table.rs`; the blend in `blend.rs`; and the slider in `ui.rs`. Old files must still load, so the field gets its value from the default through `#[serde(default)]`. Slider ids are saved in files and must never change.
- **Uniform structs:** each Rust uniform struct mirrors its WGSL struct field for field, and each has a size test. Update both sides together.
- **Testable maths:** keep geometry and maths in plain Rust with unit tests (as in `tilt.rs`). The shaders should only apply the results.
- **Line endings:** files use LF. Don't let editing tools rewrite them as CRLF, and check `git diff --stat` before committing.
- **Comments:** match the surrounding style, which uses full sentences and explains why the code does what it does. Use plain words in UI text.
- **Commit messages:** `feat:`, `fix:`, `build:`, `docs:` or `chore:`, followed by a short lowercase summary.

## Tests and running the app

- Tests may list MIDI ports, audio devices and cameras, but must never open MIDI ports or audio streams.
- GPU tests skip themselves when no adapter is available (`gpu::test_device`).
- When running the app by hand, set `APPDATA` to a scratch folder so the real autosave and settings stay untouched. Close the app afterwards.
