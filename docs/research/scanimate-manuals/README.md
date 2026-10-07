# Scanimate manual research notes

Notes taken from the three scanned manuals in `manuals/`, read page by page on 2026-10-07. The notes paraphrase the manuals, quote only short phrases, and cite page numbers. Page numbers refer to positions in the PDF file unless a note says otherwise.

Confidence varies. Some pages were faint or only partly legible, and each file says where. Treat parts lists and schematics as lightly verified.

| File | Source | Pages | Most useful for |
|---|---|---|---|
| [scanimate-manual-1969.md](scanimate-manual-1969.md) | Scanimate Manual (1969) | 63 | INITIAL/FINAL ramp workflow; 600-line 48 Hz raster; area intensity compensation; oscillator sync modes |
| [animators-cheat-book.md](animators-cheat-book.md) | Scanimate Animator's Cheat Book | 14 | Patch recipes: rolls, pinches, stretch, star fields, raster ball, sin/cos figures |
| [technical-manuals-p001-090.md](technical-manuals-p001-090.md) | Scanimate Technical Manuals | 1–90 | Animation Aid (5 sequence ramps, linear/sine shapes, frame thumbwheels); colorizer (4 thresholds → 5 levels); sections and blanking |
| [technical-manuals-p091-180.md](technical-manuals-p091-180.md) | Scanimate Technical Manuals | 91–180 | Sequence ramp generator; INITIAL + ramp × (FINAL − INITIAL); raster integrators; speed and area intensity compensation; rotation controller modes |
| [technical-manuals-p181-267.md](technical-manuals-p181-267.md) | Scanimate Technical Manuals | 181–267 | Animation controller module list (HF/LF oscillators, multipliers, summers, patch panel); colorizer and keyer details; timing |

## Findings used in the design

These feed into `docs/superpowers/specs/2026-10-07-rasterwarp-motion-output-look-design.md`.

- **The Scanimate is a two-keyframe machine.** Every animatable value is INITIAL + ramp × (FINAL − INITIAL). Ramps are linear or sine (S-curve), and resetting one is near-instant. Up to 5 sequence ramps start on frame-count thumbwheels (0–999, 24 fps in the examples).
- **The raster itself is deflected.** It is about 600 lines, non-interlaced, and has up to 5 sections. Enlarging spreads the lines into visible gaps. An intensity compensator boosts brightness with area, and also with animation speed in later models.
- **Oscillators** are triangle cores with a diode sine shaper. LF runs up to about 500 Hz and HF up to about 90 kHz (line-locked "raster bending").
  - Sync is FRAME (stationary) or free (drifting). Phase can reset per frame or at sequence start.
  - Oscillator 4 can be slaved to oscillator 3 with a 90° offset.
- **The colorizer** has 4 threshold pots giving 5 hard levels, with per-level R/G/B. Any level can be keyed over background video. Vertical edges show colored fringing.
- **Imperfections:** rotation-axis wander from imperfect multiplier nulling; run-to-run variation in the final angle from ramp tolerances.
