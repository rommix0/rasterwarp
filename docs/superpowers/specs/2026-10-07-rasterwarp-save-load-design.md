# Rasterwarp: Save and Load: Design

Date: 2026-10-07
Status: Approved 2026-10-07 (design reviewed section by section in chat)
Builds on: `docs/superpowers/specs/2026-10-07-rasterwarp-motion-output-look-design.md` (Plans 1–3, now on `main`), which listed "persisting curves, cues and presets to disk" under Later.

## Goals

- **Presets:** save one look (a full parameter set) under a name, browse the presets folder in the panel, and load a preset into whatever is being edited.
- **Projects:** save and open the whole session as a named file.
- **Autosave:** the session survives quitting and relaunching without pressing Save.
- **Durable files:** files saved now keep loading after later plans add parameters; a damaged or hand-edited file can't put the app into an invalid state.

**Out of scope:** preset thumbnails, embedding images in projects, undo/redo, MIDI mappings (no MIDI yet), sharing presets between machines beyond copying files.

## File format

- **Library:** `serde` (with `derive`) and `serde_json`. These are the first new crates since the prototype; the user approved them for this feature. Files are pretty-printed JSON.
- **Every file** is a JSON object with `"format"` and `"version"` fields:
  - Preset: `"format": "rasterwarp-preset"`, extension `.rwpreset`.
  - Project: `"format": "rasterwarp-project"`, extension `.rwproject`.
  - App settings: `"format": "rasterwarp-settings"`, file `settings.json`.
  - `"version": 1` for all three. A later plan bumps the version only when a change can't be expressed by the tolerant rules below.
- **Tolerant reading** (applies to every file):
  - A missing field takes its default (`Params::default()`'s value, or the type's default).
  - An unknown field is ignored.
  - An unrecognised enum value (e.g. a waveform from a later version) falls back to that field's default; the rest of the file still loads. A small helper deserializes the field generically, tries the real type, and falls back to the default.
  - After reading, every numeric parameter is clamped to its range with `Params::clamped()`, which lives in `params.rs` next to the existing `ranges` so files and sliders share one definition. Discrete fields are clamped the same way (e.g. `levels` to `ranges::LEVELS`, raster `lines` to `ranges::RASTER_LINES`).
  - A `version` greater than 1 still loads; the panel shows "Saved by a newer version of rasterwarp; some settings may not have loaded."
- **Safe writing:** every write (preset, project, autosave, settings) goes to `<name>.tmp` in the same folder and is then renamed over the target, so an interrupted write never leaves a truncated file. No `.tmp` file remains after a successful write.

## What each file holds

**Preset (`name.rwpreset`):** `params`, one full `Params` (warp and oscillators, raster, colorize with palette, thresholds and fringing, keying, feedback, glow). Params never refer to user curves, so a preset is self-contained and works in any project.

**Project (`name.rwproject`):**
- `mode`: Live, Transition or Sequence.
- `transition`: both banks' `Params`, which bank is on air, the transition duration and its curve.
- `sequence` (if one exists): its cues (each with `params`, start frame, ramp length in frames and curve), the selected cue and the loop setting.
- `curves`: the user curves (id, name and points), at most `curve::MAX_CUSTOM` (8).
- `canvas`: the canvas preset choice and custom size; `frame_rate`: the program frame rate.
- `source`: the source image path, or none for the built-in test card. `background`: the background image path, or none.

**Not saved (state in motion):** transition progress, the sequence's running state and position, oscillator clocks and phases, the animation frame count, and the trails. An opened project starts at rest: no transition running, the sequence stopped on its selected cue, trails cleared.

**App settings (`settings.json`, not per project):** the presets folder, the captures folder, the capture codec and options (real-time/offline, stop after), and "Show preview". (Which named project the session belongs to is recorded in the autosave, below, not here.)

**Images are stored as paths.** A path is stored as written by the OS (absolute). Images are never embedded.

## Where presets load and save

Load and Save preset act on whatever the panel is editing:
- **Live:** loading replaces the on-screen look immediately (a cut). Gliding into a preset is done in Transition mode: load into the off-air bank, then Transition.
- **Transition:** loads into the off-air bank being edited. The on-air picture doesn't change until a transition or cut.
- **Sequence:** loads into the selected cue's `params`; the cue keeps its start frame, ramp length and curve.

Save preset saves the same look that Load would replace.

## The panel

**Project row** (under the header's Pause / Show preview line; replaces "Reset all"):
- **Open project…**, **Save project**, **Save as…**, **New**.
- The header shows `Project: <name>` with ` •` appended while there are unsaved changes, or `Project: Untitled`.
- **Save project** on an Untitled session behaves like Save as… (a save dialog filtered to `.rwproject`).
- **New** resets everything to defaults (what "Reset all" did) and makes the session Untitled.
- Keyboard: Ctrl+S Save project, Ctrl+Shift+S Save as…, Ctrl+O Open project…. These fire only when no text field has keyboard focus.
- **Unsaved changes:** Open project, New, and dropping a project file first check for unsaved changes. If there are any, a system message dialog (`rfd::MessageDialog`) asks "Save changes to <name>?" with Save / Don't save / Cancel. Save on an Untitled session goes through Save as…; cancelling that dialog cancels the whole action. Quitting never asks, because the autosave keeps the session.
- **Unsaved changes are detected** by comparing a snapshot of the current project (the project file's contents, in memory) with the snapshot last saved or opened. No dirty flags.

**Presets section** (collapsible, directly above Curves):
- A name text box and a **Save preset** button. Saving to a name that already exists asks "Replace preset <name>?" (Yes / No) first.
- The presets in the folder, as a list of names sorted case-insensitively. Clicking a name loads it (see "Where presets load and save").
- Right-click on a name: **Rename** (inline text box; Enter confirms, Escape cancels; an existing name is refused with a message) and **Delete** ("Delete preset <name>?" Yes / No).
- "Folder: <folder> — **Choose…**", like the captures folder. The default is `presets`, a relative path next to the default `captures`.
- Preset names must be non-empty after trimming and contain none of `\ / : * ? " < > |`; the Save button is disabled with a hint otherwise.
- The list is rescanned when the section is opened, after any save, rename or delete, and when the folder changes. A missing folder lists nothing; saving creates it.

**Drag and drop** (the window already accepts PNG/JPG sources): a dropped `.rwpreset` loads like a click in the list; a dropped `.rwproject` opens like Open project… (with the unsaved-changes check). Other extensions keep their current behaviour.

## Autosave and restore

- **Location:** `%APPDATA%\rasterwarp\` (from the `APPDATA` environment variable; if it isn't set, a `rasterwarp-data` folder in the current working directory). It holds `autosave.rwproject` and `settings.json`, created on first write.
- **What the autosave holds:** the current project snapshot, plus the named project path it belongs to (if any) and whether it had unsaved changes. After a relaunch the header shows the same `Project: <name> •`, and Save project writes to that named file.
- **When it's written:** when the window closes, and every 60 seconds of wall time if the snapshot changed since the last autosave. Settings are written when they change and on close.
- **Restore on launch:** if `autosave.rwproject` exists, it is restored, at rest. An image path on the command line still overrides the restored source image. If the autosave can't be read, the app starts fresh with defaults, renames the file to `autosave.bad.rwproject` (replacing any older one), and shows a one-line error in the panel.

## Error handling

- **Opening a project or preset that can't be used** (unreadable, not JSON, wrong `format`, e.g. a preset opened as a project): an error line in the panel names the file and the problem; nothing else changes.
- **A cue referring to a user curve that isn't in the file:** the cue's curve becomes Linear. The same for the transition curve.
- **More than 8 user curves in the file:** the first 8 are kept.
- **Canvas size larger than the GPU allows:** clamped, as a custom canvas size is today.
- **A missing source or background image:** the project still opens. The panel shows "Source image not found: <path>" (or "Background image not found: <path>") and uses the test card (or no background). The missing path is kept in the project so saving doesn't silently drop it; choosing a new image replaces it.
- **Save failures** (folder missing and can't be created, permission denied, disk full): an error line in the panel; the in-memory state and the "last saved" snapshot don't change. A failing autosave is shown once, not every 60 seconds, until an autosave succeeds again.

## Code structure

- **`src/save.rs`, file formats:** the `PresetFile`, `ProjectFile` and `Settings` types with their format names and version; JSON read/write with the tolerant rules; the enum-fallback helper; the safe write.
- **`src/session.rs`, the project in memory:** builds a project snapshot from the running app (mode, banks, sequence, curves, canvas, frame rate, image paths) and applies one back (at rest). Snapshot comparison gives unsaved changes.
- **`src/presets.rs`, the presets folder:** list (sorted), save, rename, delete, and name validation.
- **`src/files_ui.rs`, the project row and the Presets section:** returns button presses as actions, like the existing panel code, so `ui.rs` (674 lines) doesn't grow much.
- **`src/app.rs`:** the dialogs, the autosave timer and save on close, keyboard shortcuts, and routing dropped files by extension.
- **Existing types:** `serde` derives on the settings types (`Params` and its parts, enums), curves, cues and the frame rate. `Params::clamped()` in `params.rs`. Small accessors on `Motion` and `Sequence` to take out and put back the saved parts; session-only state (clocks, ramp progress, running position) stays private.

## Testing

**Unit tests (fast, no GPU):**
- Preset round trip with `Params::default()` and with every field changed from its default: what's read equals what was written.
- Project round trip: both banks, on-air bank, duration and curve, a sequence with cues using a user curve, the curves, canvas, frame rate and both paths.
- Tolerant reading: a missing field takes its default; an unknown field is ignored; out-of-range values are clamped (one numeric and one discrete field); an unknown enum value falls back to its default while the rest loads; `version: 2` loads and is flagged as newer; a preset read as a project is an error naming the format.
- A cue with a missing user curve becomes Linear; 9 curves in a file keep the first 8.
- Unsaved-changes tracking: a fresh snapshot is clean; changing a parameter makes it dirty; saving makes it clean again.
- Presets folder (in a temporary directory): list sorted case-insensitively; save, rename (including refusing an existing name) and delete; name validation.
- Safe write: the target has the new contents and no `.tmp` file remains.

**Compatibility guard:** `tests/fixtures/v1.rwpreset` and `tests/fixtures/v1.rwproject`, written by this version and checked in. A test loads both and checks key values. These files are never regenerated; they must keep loading in every later version.

**Manual check (release build):**
- Save a preset and load it in each of Live, Transition (into the off-air bank) and Sequence (into the selected cue).
- Rename and delete a preset; refuse a bad name.
- Save a project, change things (header shows `•`), Save, Save as…, New, Open: the unsaved-changes prompt appears when expected and Cancel cancels.
- Quit and relaunch: the session, project name and unsaved marker come back.
- Drop a preset and a project onto the window.
- Open a project whose source image was moved: it opens with the test card and an error line.
