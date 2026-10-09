# Rasterwarp

Real-time animation software that recreates the look of 1970s analog video synthesis: artwork warped by oscillator-driven deflection, colorized by brightness level, with CRT glow and feedback trails. It runs on the GPU and is played live, like an instrument.

Developed by Anthony C. Bartman (@rommix0).

## Features

- **Deflection:** stacked oscillators (sine, triangle, ramp, square, noise) with LFOs and envelopes warp the source.
- **Look:** posterized color levels, bloom, scanlines and feedback trails.
- **Motion:** Live editing, A/B Transitions with curves, and a cue Sequence.
- **Sources:** images, video files and cameras (with a looper), plus a background layer.
- **Control:** link any slider or option to MIDI, or to sound (level, bass, mid, treble, beats) from a microphone, system audio or a sound file.
- **Output:** record HEVC (NVENC), ProRes 4444 or FFV1, with alpha and the sound file as a soundtrack; save stills.
- **Projects:** saved to `.rwproject` files and autosaved.

## Requirements

- Windows 10 or later, a GPU with Vulkan (or DX12 / OpenGL via `RASTERWARP_BACKEND=dx12` or `gl`).
- An NVIDIA GPU for HEVC recording (ProRes and FFV1 work everywhere).

## Building

1. Install Rust 1.88 or later (MSVC toolchain) and a shared FFmpeg 9 build.
2. Set `FFMPEG_DIR` (the FFmpeg build) and `LIBCLANG_PATH` in `.cargo/config.toml`.
3. `cargo run --release`

To make a distributable copy, run `powershell -ExecutionPolicy Bypass -File .\dist.ps1`. It builds a release and copies `rasterwarp.exe`, the FFmpeg DLLs and this README into `dist\`.
