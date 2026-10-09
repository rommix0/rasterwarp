# Rasterwarp: Audio Links: Design

Date: 2026-10-09
Status: Approved 2026-10-09 (design reviewed section by section in chat).
Builds on: everything on `main` at `2b2dadd` (video inputs, alpha export, camera looper, MIDI control).

## Goals

- **Mouth control:** sound moves sliders. Loudness, or the bass, mid or treble band, pushes any slider up or down from where the hand left it.
- **Beats fire things:** a detected beat, or one tapped by hand (a MIDI key, a panel button), picks an option or runs an action the way a MIDI key does, and kicks a decaying Pulse that sliders can follow.
- **Music videos in sync:** a sound file plays with the clock, and recordings carry it as their soundtrack, sample-exact in real-time and frame-by-frame recording alike.

**Out of scope** (later):
- Recording live input into captures.
- Adjustable band edges, an FFT spectrum view.
- More than one audio source at a time.
- Audio from video files' own soundtracks.
- Audio links per bank or cue.

## Sources

One audio source at a time, chosen in the Audio section:

- **None** (the default): signals read 0. Hand beats still work.
- **Input:** an input device (microphone, line-in), through `cpal` (WASAPI on Windows), shared mode.
- **Loopback:** an output device's loopback ("what the PC is playing"), through `cpal`.
- **File:** a sound file (WAV, MP3, FLAC, OGG, and anything else FFmpeg decodes), decoded by the linked FFmpeg libraries and resampled with swresample.

## Signals

Every source produces the same signals.

- **Follow signals,** each 0 to 1:
  - **Level:** the whole signal.
  - **Bass:** below 200 Hz (low-pass).
  - **Mid:** 200 Hz to 2 kHz (band-pass).
  - **Treble:** above 2 kHz (high-pass).
  - **Pulse:** jumps to 1 on every Any beat (detected or by hand) and falls back to 0 over the release time. It is computed once per canvas frame from that frame's beats, stepping by the frame period, for every source.
- **Beats,** which fire like key presses:
  - **Any:** an onset in the whole signal, or any hand beat.
  - **Bass:** an onset in the bass band (a kick).
  - **Treble:** an onset in the treble band (hats, snare).

**Analysis** (`src/audio/analysis.rs`, one implementation shared by live input and files):
- Input is mixed to mono at 48 kHz.
- Each band is a second-order IIR filter (biquad); Level is unfiltered.
- Each band is rectified and followed by an envelope follower with the **attack** and **release** times. At gain 1, a full-scale sine inside a band reads 1; louder values clamp at 1. **Gain** multiplies before clamping.
- **Onsets:** a band's fast envelope rising above its slow average (about 1 s) by a ratio set by **sensitivity**, and above a small floor, is an onset. After an onset, that band can't fire again for 100 ms.
- Defaults: gain 1 (range 0 to 8), attack 10 ms (1 to 200 ms), release 150 ms (10 ms to 2 s), sensitivity 0.5 (0 to 1, higher fires more easily).

**Live sources:**
- `cpal`'s audio thread runs the analysis block by block.
- After each block it stores the latest follow values and adds to per-beat counters in a shared slot of atomics, with no locks.
- The app reads the slot once per canvas frame. A beat counter that rose since the last read is a beat this frame.
- Live analysis keeps running while the app is paused.

**Files:**
- On load, a background thread decodes the whole file and runs the analysis over it once.
- It stores Level, Bass, Mid and Treble as tracks at 1,000 values per second, and the onset times of each beat.
- Each canvas frame reads the tracks at the file's playhead time. Every onset between the previous frame's playhead and this one is a beat this frame, including across a loop wrap.
- Because the tracks are fixed, the same moment of the song always gives the same signals: real-time play, frame-by-frame recording, scrubbing, pause and loops agree.

**Hand beats:**
- Three new actions, **Beat**, **Bass beat** and **Treble beat**, fire their beat as if detected.
- A hand Bass or Treble beat is also an Any beat.
- They work with any source, None included, and mix with detected beats.
- Saved names, pinned like the other action names: `beat`, `bass-beat`, `treble-beat`.

## Audio links

Two kinds, stored as two lists in the project.

**Follow links** (`AudioLink`): a follow signal moves a slider.
- **Targets:** every slider in the parameter table (`Target::Slider`). Not the transition duration, which isn't drawn per frame. Not choices.
- **Depth:** in the slider's own units, negative allowed. Defaults to a quarter of the slider's range.
- **The shown value** is `base + depth × signal`, held inside the slider's range.
  - **Base** is the slider's value in the frame's params (set by hand or MIDI, and blended by transitions and sequences).
  - Several follow links on one slider add their `depth × signal` terms.
  - Whole-numbered sliders round after adding.
  - The value is written through the table's setter (`SliderId::set`), so tied-value rules hold.
- One signal can drive several sliders; one slider can follow several signals. Linking the same signal to the same slider twice keeps one link.

**Beat links** (`BeatLink`): a beat fires a target.
- **Targets:** everything a MIDI note can fire: a choice option, a mode, or an action (Record and Pause included).
- They fire through the same code that applies MIDI note targets, factored out of `Links::handle` so both share it: choices into `Motion::editable()`, the mode through `Motion::set_mode`, actions through the app's `run_action`.
- Linking the same beat to the same target twice keeps one link.

**Where modulation applies:**
- On the frame that is drawn and recorded: the params from `Motion::frame()`, after the transition or sequence blend. The banks, the cues and the panel's values are never changed by modulation.
- It works the same in Live, Transition and Sequence. Turning a slider or a MIDI wheel moves the base, and modulation stays on top of it.
- The pulsing is not an edit: it never marks the project unsaved.
- The off-air preview is modulated by the same links, so it shows what will go on air.
- When the source is None, stopped or failing, follow signals read 0 (Pulse still follows hand beats), and every slider sits at its base.

## The sound file

**Playhead:** a loaded file has a position in seconds, separate from transitions and sequences.
- Each canvas frame advances it by one frame period while it is playing and the app isn't paused, as animation time advances. So the app's Pause freezes it, and frame-by-frame recording steps it one frame at a time.
- **Controls:** Play/Pause, Restart, a position slider (scrubbing), and Loop.
- **At the end:** with Loop, it wraps to 0; without, it stops at the end and goes silent.

**Hearing it:**
- A `cpal` output stream plays it on an output device: the system default, or one picked in the section.
- A **volume** slider: 0 to 1, default 1.
- The output follows the playhead. When the sound being played is more than 40 ms from the playhead (after a scrub, a hitch, Pause or Restart), the output jumps to the playhead.
- While a frame-by-frame recording runs, the speakers are muted.

**Memory and loading:**
- The file is decoded to 48 kHz stereo 16-bit (about 11 MB a minute), plus the analysis tracks.
- This is charged to the same memory budget as video clips and the camera looper. A file that doesn't fit is refused with "This file needs X of memory and only Y is free".
- Loading runs in the background with a progress line ("Loading drums.wav… 40%"), as video clips do. The previous source keeps working until the new file is ready.

## Recording

When the source is a sound file, recordings get it as their soundtrack.

- **Where the sound comes from:** the decoded file, for each recorded frame's time span on the playhead (silence for frames where the file is paused or has ended). It is never captured from the speakers. So it is sample-exact in real-time and frame-by-frame recording.
- **Codecs:**
  - HEVC (`.mp4`): AAC.
  - ProRes 4444 (`.mov`): PCM (signed 16-bit).
  - FFV1 (`.mkv`): PCM (signed 16-bit).
- **"Start the sound with the recording"** (on by default): Record restarts the file from 0.
- **End of the file:** if the file isn't looping, a recording stops when the file ends, unless "Stop after" ends it sooner.
- **Not recorded:** live input and hand beats.

## The panel

**Right-click menus** (`link_ui.rs`). Next to "Link to MIDI":
- Sliders (except the duration): "Audio: Level", "Audio: Bass", "Audio: Mid", "Audio: Treble", "Audio: Pulse".
- Choice options, mode labels and action buttons: "Beat: Any", "Beat: Bass", "Beat: Treble".
- Picking one makes the link at once; no learning step.
- For a linked target, the menu also lists "Unlink Audio: Bass" or "Unlink Beat: Bass" entries.
- The hover text names audio links along with MIDI links, for example "Audio: Bass +0.50".

**The Audio section** (new, collapsed by default, after the MIDI section):
- **Source:** None / Input / Loopback / File.
  - Input and Loopback: a device drop-down and **Refresh**.
  - File: **Open…**, the file name, the loading line, Play/Pause, Restart, position, Loop, volume, the output device, and "Start the sound with the recording".
- **Shaping:** gain, attack, release, sensitivity. These are app sliders, not table entries, so they aren't linkable.
- **Meter:** five bars (Level, Bass, Mid, Treble, Pulse) and three beat lights that flash when their beat fires.
- **Tap buttons:** Beat, Bass beat, Treble beat. Each is linkable to MIDI (they are the three hand-beat actions).
- **Links:** every follow link with its signal, the target's label, a depth drag value, the live shown value, and Remove. Every beat link with its beat, the target's label, and Remove.
- **Errors:** the source's error line, if any.

## Saving

Audio belongs to the project (`Project` in `src/session.rs`), saved and autosaved with it. A project from before this has no `audio` block and loads with the source None and no links.

```json
"audio": {
  "source": "file:C:/music/drums.wav",
  "gain": 1.0, "attack": 0.01, "release": 0.15, "sensitivity": 0.5,
  "volume": 1.0, "loop": true, "start_with_recording": true,
  "output": "",
  "follow": [ { "signal": "bass", "target": "slider:warp.zoom", "depth": 0.5 } ],
  "beats":  [ { "beat": "bass", "target": "action:cut" } ]
}
```

- **Source:** `"none"`, `"input:<device name>"`, `"loopback:<device name>"` or `"file:<path>"`.
- **Output:** the output device's name; empty means the system default.
- **Signal names:** `level`, `bass`, `mid`, `treble`, `pulse`. **Beat names:** `any`, `bass`, `treble`. Pinned by tests.
- **Targets:** the same strings as MIDI links (`slider:<id>`, `choice:<id>=<option>`, `action:<name>`). A follow link's target must be a slider; a beat link's must not be.
- **Unknown entries:** a link whose signal, beat, target, id, option or action isn't known, or whose kind doesn't fit, is dropped with a log line, and the rest load.
- **Missing source:** a file or device that isn't there on load shows an error line in the section. The links and settings are kept.
- **Unsaved changes:** changing the source, a shaping setting, the volume, a link or a depth marks the project unsaved. The playhead position isn't saved (nothing in motion is).
- Presets are unchanged.

## Errors

- **No devices:** the device list is empty, with a line saying so.
- **Device in use or gone:**
  - The error shows in the section.
  - Follow signals read 0.
  - **Refresh** tries again.
- **No loopback** on a device: "This device can't loop back what it plays".
- **A file that won't decode:** "Couldn't read <file>: <reason>". The previous source stays.
- **No output device:** the file still drives links and recordings, silently, with a line saying there's no output.
- Audio never stops the app from starting.

## Code structure

- `src/audio/mod.rs` (new): `Signal`, `Beat`, `Signals` (five follow values and the beats this frame), and `Audio`. `Audio` owns the current source and gives `signals(frame)` each canvas frame.
- `src/audio/analysis.rs` (new): biquads, followers, the onset detector, Pulse. Pure, sample-rate aware, tested on synthetic signals.
- `src/audio/input.rs` (new): live input and loopback through `cpal`, the atomic slot, device listing.
- `src/audio/file.rs` (new): background decoding and resampling, analysis tracks, the playhead, the soundtrack slice for a time span.
- `src/audio/output.rs` (new): speaker playback, following the playhead.
- `src/audio_links.rs` (new): `AudioLink`, `BeatLink`, their saved forms, `modulate(&mut Params, &[AudioLink], &Signals)`, and which beat targets fire.
- `src/audio_ui.rs` (new): the Audio section.
- `src/control.rs`: the three beat actions; applying a note-style target factored out for beat links.
- `src/link_ui.rs`: the Audio and Beat menu items and hover text.
- `src/session.rs`: the `audio` block.
- `src/capture/encode.rs`, `src/capture/recorder.rs`: the soundtrack stream.
- `src/app.rs`: owns `Audio`; each canvas frame reads the signals, fires beat links, modulates `frame_params` and the preview's params; hands the soundtrack to the recorder; restarts the file and stops at its end when recording.
- `Cargo.toml`: adds `cpal`, and FFmpeg's `software-resampling` feature.
- `build.rs`: copies the swresample DLL next to the app.

## Testing

- **Unit tests:**
  - **Analysis:**
    - a 60 Hz sine lights Bass and not Treble, and an 8 kHz sine the reverse;
    - a full-scale in-band sine reads about 1 at gain 1;
    - attack and release times;
    - a click train gives one beat per click, and the 100 ms hold-off;
    - Pulse jumps on a beat and decays over the release.
  - **Tracks:**
    - lookup between samples;
    - beats in a frame span, including across a loop wrap and with the playhead paused.
  - **Modulation:**
    - depth and negative depth;
    - clamping to the range;
    - rounding of whole-numbered sliders;
    - links on one slider adding;
    - the tied-value rules through the setter;
    - the bank untouched.
  - **Beats:**
    - a hand Bass beat is also Any and kicks Pulse;
    - beat links fire choices into `editable()` and report actions.
  - **Saving:** a project round trip with audio; an old project without `audio`; unknown entries and mismatched kinds dropped while the rest load; names pinned.
- **File test:** decode `samples/drums.wav`: its length, and that beats were found.
- **Capture test:** encode frames plus a soundtrack to `.mp4`, `.mov` and `.mkv`, decode them back, and check the audio codec and that the audio and video lengths match.
- **Devices:** a test that lists input and output devices, which passes with none.
- **By hand:**
  - loopback while music plays;
  - `drums.wav` playing in sync, with Pause, Loop and scrubbing;
  - a frame-by-frame render of `drums.wav` to `.mp4`, played back to check sync;
  - Q25 keys tapping beats onto Cut, and Pulse onto zoom.

## Later

Recording live input. Adjustable band edges or an FFT spectrum view. Several sources at once. Audio from video files' own soundtracks. Audio links per bank or cue. OSC (still pending from MIDI control).
