# Rasterwarp: MIDI Control: Design

Date: 2026-10-09
Status: Approved 2026-10-09 (design reviewed section by section in chat).
Builds on: everything on `main` at `6c763f4` (video inputs, alpha export, camera looper).

## Goals

- **Play Rasterwarp from a MIDI controller:** a wheel, fader or knob moves a slider; a key runs an action or picks an option.
- **Learn links by touching things:** right-click a control, move a wheel or press a key, done.
- **A parameter table** that names every linkable slider and choice once, so the panel, MIDI and later the audio links (project B) all reach parameters the same way.

The controller used for testing is a keyboard with a pitch wheel and a mod wheel: keys trigger actions and choices, and the two wheels stand in for knobs and faders.

**Out of scope** (later):
- **OSC.** The link model leaves room for it as another source.
- **Audio-driven amplitude** (project B), which reuses the parameter table.
- **Checkboxes** (Raster, Bypass, keying, keyed levels, Flip) as link targets.
- **Endless encoders** (relative CCs), **soft takeover**, and a **pitch-bend mode** that bends around the current value.
- **MIDI output** (feedback to motorised faders or LEDs), MIDI clock, and note velocity as a value.
- **Several MIDI maps** to switch between; links live in the app settings.

## What can be linked

Three kinds of target:

- **Sliders.**
  - Each oscillator (1 to 4): amplitude, frequency, phase, phase speed, LFO rate, LFO depth.
  - Deflection: zoom, rotation, offset x, offset y, analog drift, axis wander, line jitter.
  - Raster: lines, beam width, area compensation, speed compensation.
  - Colorize: levels, each "level N from" threshold, softness, edge fringe, ringing, cycle speed.
  - Feedback: amount, zoom, rotation, offset x, offset y.
  - Glow and CRT: bloom, bloom threshold, scanlines, scanline count, chroma bleed, noise.
  - Source and Background: speed, position, delay, slit-scan depth.
  - The transition duration.
- **Choices**, where a key selects one option.
  - The mode: Live, Transition or Sequence.
  - Each oscillator's waveform, input, sync, envelope and axis (→ X or → Y).
  - Each input's play mode, between-frames setting and slit-scan mode.
- **Actions**, where a key runs the action.
  - Transition (or Reverse/Resume, as the Space key does).
  - Cut.
  - Sequence Run/Stop.
  - Sequence Reset.
  - Source camera loop toggle (F9).
  - Background camera loop toggle (F10).
  - Record/Stop recording.
  - Save still.
  - Clear trails.
  - Pause/Resume.

Palette colours, file pickers, the canvas size and capture settings are not linkable.

## Where values go

A link moves its target the way a hand on the panel would. Bank settings go into the bank the panel edits (`Motion::editable()`): the screen in Live, the off-air bank in Transition, and the selected cue in Sequence. The panel's slider moves as the wheel turns.

- **Unsaved changes:** a change through MIDI marks the project unsaved, as an edit does.
- **Hidden panel:** links work whether the panel is open, collapsed or hidden (Tab), because they don't go through the panel's drawing.
- **Actions** do exactly what their button or key does, including doing nothing where the button would be disabled. For example, Cut does nothing outside Transition mode, and a loop toggle does nothing when that input isn't a camera.

## The parameter table (`params.rs`)

One list describes every linkable slider and choice. Each entry has:

- **An id:** a permanent name, lowercase, dot-separated, with hyphenated words. Examples are `warp.osc1.amplitude`, `warp.zoom`, `colorize.level2-from`, `glow.scanline-count`, `source.speed`, `background.slit-depth`, `transition.duration` and `warp.osc3.waveform`. Ids are saved in the settings, so once released they never change; a test pins them, as `saved_names_never_change` does for enum names.
- **A label** for the MIDI section, such as "Osc 1 amplitude" or "Source speed".
- **For a slider:** its range (from `params::ranges`), whether it is whole-numbered, and functions that read and write it in a `Params`.
- **For a choice:** its options, by the same saved names the enums use (`sine`, `ping-pong` and so on), and functions that read and write it.

**Rules that tie values together live in the setters,** so they hold however a value changes:
- Setting the level count resets the thresholds to even spacing and drops keyed levels that no longer exist, as the panel does now.
- Setting a threshold holds it between its neighbours.

**The panel draws these sliders and choices through one helper** that takes the id. The helper draws the egui control, writes through the table's setter, and attaches the right-click menu. That's how the panel and MIDI stay identical.

**Outside the bank:** the transition duration (on `AbState`) and the mode (on `Motion`) are entries whose functions work on `Motion`. Every other entry works on the edited `Params`.

## Links (`control.rs`)

- **Source.** One of:
  - a control change (`cc`): channel 1–16 and controller 0–127. The mod wheel is CC 1.
  - the pitch wheel (`pitch`): channel 1–16.
  - a note (`note`): channel 1–16 and note 0–127.
- **Target.** One of:
  - a slider id;
  - a choice id and an option;
  - an action.
- **Range,** for a slider target: `min` and `max`, defaulting to the slider's whole range.
  - Setting `min` above `max` inverts the control.
  - Both are held inside the slider's range.

**Matching:**
- One source can drive several targets, and one target can have several sources.
- Learning the same source onto the same target twice keeps one link.
- Note sources only link to choices and actions; CC and pitch sources only link to sliders. While learning a slider, notes are ignored, and while learning a choice option or action, wheels are ignored.

**Values:**
- A CC's 0–127, or the pitch wheel's 0–16383, maps linearly from `min` to `max`. The pitch wheel's centre (8192) lands halfway, so letting go of the wheel returns the slider to the middle of the link's range.
- Whole-numbered sliders round to the nearest whole number.
- There's no soft takeover: the first movement sets the value.

**Notes:**
- A note-on with velocity above 0 fires its targets.
- Note-off, and note-on with velocity 0, do nothing.

## MIDI input (`midi.rs`)

- **Library:** the `midir` crate (WinMM on Windows).
- **Ports:** Rasterwarp listens to every MIDI input port at once; there is no device picker. Each port's callback runs on midir's thread and sends parsed messages over a channel; anything that isn't a CC, pitch wheel or note is dropped there.
- **No waiting:** once per screen refresh, the app drains the channel and applies the messages before drawing the panel. Neither rendering nor the panel ever waits on MIDI.
- **New devices:** Windows doesn't announce them, so the port list is rescanned every 3 seconds and when **Refresh** is pressed. New ports open, ports that have gone are dropped, and a rescan must not reopen ports that are already open. Links don't depend on which port a message came from, so a device that comes back works with its old links.

## Learning and the panel

- **Linking:** right-clicking a linkable slider, choice option or action button opens a menu with **Link to MIDI**.
  - Choosing it starts learning that target. The MIDI section shows "Learning *target*: move a wheel or press a key…" with **Cancel**; Esc cancels too.
  - The first fitting message (see Matching) makes the link and ends learning.
  - Learning another target replaces the one waiting.
- **Unlinking:** for a target that has links, the menu also lists them, for example "Unlink CC 1 (ch 1)". A linked control's hover text names its links.
- **The MIDI section** (new, collapsed by default):
  - The devices that are open, or "No MIDI input devices", with **Refresh**.
  - Any port that failed to open, with the reason.
  - The learning line, while learning.
  - Every link: its source ("CC 1 ch 1", "Pitch ch 1", "Note C4 ch 1"), its target's label, and for sliders the min and max as drag values. Each link has a delete button.
- **Action buttons** whose right-click offers linking: Transition, Cut, Run/Stop, Reset, Loop last N s and Back to live, Record and Stop recording, Save still, Clear trails, and the Pause checkbox. Pause is an action even though it is drawn as a checkbox.

## Saving

Links are saved in the app settings (`Settings`, beside the capture settings) and apply in every project.

- **Format:** a list of `{ "source": ..., "target": ..., "min": ..., "max": ... }`, where:
  - a source is a string: `"cc:1:1"`, `"pitch:1"` or `"note:1:60"` (kind, channel, number);
  - a target is a string: `"slider:warp.osc1.amplitude"`, `"choice:warp.osc1.waveform=square"` or `"action:transition"`.
- **Action names** are pinned like the ids: `transition`, `cut`, `sequence-run`, `sequence-reset`, `source-loop`, `background-loop`, `record`, `save-still`, `clear-trails`, `pause`.
- **Unknown entries:** a link whose source, id, option or action the app doesn't know is dropped with a log line, and the rest load. A file from a newer version still loads.
- **Projects and presets** are unchanged.

## Errors

- **No devices:** "No MIDI input devices" in the MIDI section. MIDI never stops the app from starting.
- **A port that won't open** (for example, another program holds it exclusively): its name and reason appear in the MIDI section, and it's tried again at the next rescan.
- **A device unplugged mid-performance:** it disappears at the next rescan; its links stay.

## Code structure

- `src/params.rs`: the parameter table (sliders and choices, ids, labels, getters and setters), with the tied-value rules moved into setters.
- `src/control.rs` (new):
  - `Source`, `Target`, `Action` and `Link`, with their saved string forms;
  - value mapping;
  - the learning state;
  - applying a message to `Motion` and returning the actions to run.
- `src/midi.rs` (new): the ports, the rescanning, message parsing, and the channel to the app.
- `src/ui.rs`, `src/inputs_ui.rs`: sliders and choices drawn through the table helper; right-click menus; the MIDI section; action buttons report link requests.
- `src/app.rs`: owns the MIDI input; drains it each refresh; runs actions through the same code as their buttons and keys.
- `src/save.rs`: `Settings` gains the links.
- `Cargo.toml`: adds `midir`.

## Testing

- **Unit tests:**
  - **Parsing:** CC, the 14-bit pitch wheel, note-on, note-on with velocity 0 as note-off, and other messages dropped.
  - **Value mapping:** ends and middle, the pitch wheel's centre, inverted ranges, whole-number rounding.
  - **The table:**
    - ids are unique and pinned;
    - every default lies inside its range;
    - get/set round trips;
    - choices cover every enum value;
    - the levels and thresholds rules behave the same through the setter as through the panel.
  - **Learning:**
    - a wheel links a slider;
    - a note links a choice option or action;
    - a mismatched message is ignored while learning;
    - relearning the same pair keeps one link;
    - cancel;
    - unlink.
  - **Applying:**
    - values go into the edited bank in each mode (on air in Live, off-air in Transition, the selected cue in Sequence);
    - choices select their option;
    - actions are reported.
  - **Settings:** round trip with links; unknown sources, ids, options and actions are dropped while the rest load.
- **MIDI:** a test that lists the input ports, which passes with none.
- **By hand, with the keyboard:**
  - the mod wheel on oscillator 1's amplitude;
  - the pitch wheel on zoom, with a narrowed range;
  - keys for Transition, Cut, the camera loop and a waveform choice;
  - unplugging and replugging the keyboard;
  - links surviving a restart.

## Later

OSC as another source. Audio-driven amplitude (project B) on the same table. Checkbox targets. Endless encoders and soft takeover. A bend mode for the pitch wheel. MIDI output for controller feedback. Several MIDI maps.
