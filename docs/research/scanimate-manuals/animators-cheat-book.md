# Scanimate Animator's Cheat Book - design notes (14 pages, handwritten patch sheets)

General: a collection of hand-lettered "patch sheets", one effect per page: a CRT sketch, a "Direction of Scan" arrow, and a patch diagram. The "raster" is the scanned camera frame. Effects are made by deflecting/modulating the raster's *sections* (Section Vertical / Horizontal / Width / Length / Depth), not by warping the artwork. Pages 1-13 contain content; p014 is a blank template (empty CRT frame on graph paper).

## 0. Patch notation (p001)
- Signal source (circle, usually an oscillator): shows oscillator # (e.g. #2) and waveform (sine/square/triangle); inputs: Freq. Cont. (left), Amplitude Control (top); output arrow to a destination. Special details noted beneath (phase lock, free run, etc.).
- Amplifier: summing, inverting etc. (triangle, labelled SUM); inputs include any bias/offsets.
- Multiplier: box with X; output = X*Y (voltage at one input multiplied by voltage at other).
- Processing circuits (not limited to): rectifiers, filters, comparators, digital inverters; INV block.
- Gain pot (input -> wiper, grounded end) = attenuator/scaler; Bias pot (+V ... -V, wiper out) = adjustable bipolar DC offset.
- Emulation takeaway: a node graph with sources (osc/ramp), sum amps with bias pots, multipliers, gain pots, inverters is exactly the authentic model.

## 1. Effect recipes
### Raster Pinch / "Door Swing" (p002)
- Look: raster becomes a trapezoid; modifies raster LENGTH at a horizontal rate (scan direction horizontal). Combined with a "turn to a line" it is called a "door swing".
- Patch: Vert. Ramp + Bias1 -> SUM A; Horiz. Ramp + Bias2 -> SUM B; A x B (multiplier) -> Gain pot -> "Final Section Vert."
- Bias1 sets apparent vertical angle (tilt of the trapezoid); Bias2 sets horizontal position of the pinch point (where the apex is); Gain sets amount of effect on raster.
- Note: this shaping does NOT track through depth; must be multiplied "on"/"off" proportionally as depth changes.
- Sketched raster lines converge (fan) toward the narrow end.

### "Perspective" Pinch (p003)
- Look: trapezoid; width modification at a vertical rate (scan horizontal). This one DOES track a section through depth.
- Patch A: any LOW-frequency oscillator, triangle wave, phase locked to vertical (choose "top" or "bottom" lock for pinch direction); output -> Section Width. Amplitude = amount of pinch.
- Patch B (also): Vert. Ramp + Bias -> SUM -> Gain control -> Section Width. Invert to change pinch direction.

### Width Distortions (hourglass pinches, side bulges) (p004)
- Oscillator (sine or triangle) -> Section Width. Amplitude = amount of distortion; Frequency = number of width changes down the frame; waveshape = sharpness of angles/curves. "Experiment!"

### Vertical Roll / "Coke Roll" (p005)
- Look: text wrapped on a cylinder rotating (barrel with letters wrapping around). Scan direction horizontal.
- Set Section LENGTH to "0" (raster collapses to a line; the sin/cos supply the extent).
- Osc #3 (SIN) -> Section Vertical (either Final Vert., or Initial Vert. if you want to "resolve out" of it; text partly cut off at right edge of scan, illegible). Osc #4 (COSINE) -> Section Horizontal (same options).
- Amplitude of both oscillators = size and angular view of roll; Frequency = speed of rolling (when in free run).

### Horizontal Roll (also "Coke Roll") (p006)
- Same patch as vertical roll, but raster oriented at 90 deg (scan direction vertical). Use CPU "90 deg" switch, or rotation if more than one section is being animated. Section Length at 0.

### Figure 8 Rolls (p007; vertically or horizontally)
- Scan direction vertical in sketch. Section Length at 0. Osc #3 (SINE) -> Section Vertical, and also -> a multiplier with Osc #4 (COSINE); multiplier output -> Section Horizontal. (sin and sin*cos = Lissajous figure-8 ribbon.)

### Oscillator Stretch (p008)
- Aliases: "Reasoner Stretch", "Multiple Image stretch", "Hor. Line Phase Lock", "Stairstepping". At higher freq. the effect has an interlaced, scissor-like quality (sketch: image repeated as stacked, vertically offset copies, 'NBN' shown 3-4x stacked).
- Patch: HIGH-frequency osc #2 -> Final Vertical (can also go to Horizontal, Depth, Length; "various effects possible, experiment"). Phase locked to a horizontal line (see "Programmed Phase Lock & Vert. Reset Driver", which affects osc #1 and #2 in the Animation Controller).
- Emulation mechanism: oscillator synced to line rate, so each scan line receives a stepped vertical offset -> lines displaced; image appears as multiple offset copies / interlaced scissor.

### Static Star Fields / "Night Sky" background (p009)
- Take raster to a dot (depth to zero, or length and width to zero). Patch: osc #3 (SIN) -> HORIZ; osc #4 (COS) -> VERT; an additional any-HIGH-freq oscillator feeds into both (diagram shows it joined to #3 and #4 inputs). #3 & #4 both phase locked to vertical; high-freq osc phase-locked with frequency adjusted for MAXIMUM STABILITY (use an exact osc in a trigger mode if possible).
- Adjust blanking with Horiz. segment #1 so only the END of each raster line is seen. => stars are the line ends of a collapsed, spinning raster; field is stationary due to lock.

### Moving Star Fields (p010)
- Begin with static starfield. Exact 505 oscillator in FREE-RUN: symmetry set to ramp (sawtooth), offset "plus" or "minus" (not "bipolar"). RAMP OUT feeds both inputs of a multiplier (squared ramp "Ramp2"); that goes to a second multiplier with the pattern-modulation (a triggered Exact out) -> to the Sin/Cos making the pattern.
- Squared ramp sweeps stars out nonlinearly (they speed up as they move out) = added "perspective" effect.
- Square-wave output (with symmetry/offset set so the pulse coincides with the ramp fall-back) -> Horiz segment blanking input #2 (to unblanked portion of raster) to blank returning lines, so stars move one direction only ("out" or "in", depending on frequency setting).
- Sketch: stars radiating from center with arrows (warp-speed), dense/small at center, larger at edges.

### Graded Color Backgrounds (p011)
- Look: dark at top to light at bottom, streaky raster texture.
- Patch: oscillator (sine or triangle) + Bias -> amp -> external background colors (the Red, Green, Blue patch points on the panel above the blanking controller); OR Vert. Ramp + Bias -> amp -> same. Colorizer "Ped.", "Luminance" and "Mix" controls affect this (color level, blacks, saturation).

### Raster Ball (p012)
- Look: sphere built from curved meridian-like raster strips (globe). Scan direction vertical (use 90 deg switch @ CPU or rotation).
- Patch: four oscillators each with a "Mult. In" (amplitude-multiplication input): #1 lock Bottom, #2 lock Top (phase-locked to vertical); their outputs + Bias -> SUM -> Ext. Width. Osc #3 (SIN) -> Final Vert, and also fed to the Mult. In of #1/#2 (routing partly ambiguous in sketch). Osc #4 (COS) -> gain pot -> Final Horizontal, and same signal + Bias via SUM -> Blanking section (turns off back of ball).

### Basic Sin/Cos Patterns (p013)
- Osc 3 (sin) and osc 4 (cos) to Horizontal/Vertical (either order) -> a circle on the CRT. Oscillators set to sine: patterns round or elliptical; set to triangle: patterns square (diamond). Additional patterns developed with Osc 1, 2, 5 multiplied through #3 & #4. Motion is provided with ramps.

## 2. Animation over time / transitions
- Essentially none in this book (static patch sheets). Relevant hints:
  - Oscillators have "free run" vs "phase lock" modes; frequency "controls speed of rolling (when in free run)" (p005). Phase lock to vertical with "top"/"bottom" lock options (p003, p012).
  - "Animation Controller" houses oscillators #1 and #2 with "Programmed Phase Lock & Vert. Reset Driver" (p008). Exact oscillator (505) can be triggered or free-run (p009, p010).
  - Motion provided with ramps (p013); moving starfield = free-run ramp, squared, with return blanked (p010).
  - Depth tracking: shaping must be multiplied on/off proportionally as depth changes (p002); perspective pinch tracks through depth (p003).
  - Not covered: ramp durations, A/B presets, hold/reset curves.

## 3. Raster / scanlines
- "Direction of scan" arrow on each sketch (horizontal or vertical): raster can be rotated 90 deg (CPU "90 deg" switch or rotation) (p006, p012). Pinch (length at horizontal rate) vs perspective pinch (width at vertical rate) differ by scan direction.
- Raster is addressed as "sections" with Length, Width, Depth, Vertical, Horizontal; collapsing length to 0 gives line-based figures (rolls p005-p007); collapse to a dot gives star fields from line ends (p009).
- High-frequency vertical modulation phase-locked to a horizontal line gives "stairstepping" with interlaced scissor-like artifacts (p008).
- Sketched pinched rasters show visible scan lines converging (p002) and widening/narrowing (p003). No line counts or beam size given.
- Blanking is a per-section signal added to the raster: Horiz segment blanking inputs #1, #2 (p009-p010), "Blanking section" hides the back of the ball (p012), square-wave pulse blanks ramp return lines (p010).

## 4. Colorizing
- Only p011: external background colors via R, G, B patch points (above the blanking controller) fed by oscillator+bias or vertical ramp+bias; Colorizer "Ped.", "Luminance", "Mix" affect color level, blacks, saturation. Vertical ramp gives dark-to-light gradient background.
- No level-to-color assignment tables.

## 5. Other authenticity notes
- Terminology: section, segment, final vert/horiz vs initial vert, ramp, bias, gain, mult in, ext. width, phase lock top/bottom, free run, trigger mode, CPU, "Exact 505" oscillator (symmetry and offset knobs; offset plus/minus/bipolar), blanking controller, Animation Controller.
- Each oscillator has amplitude and frequency controls plus a multiplier input (amplitude-modulation chain, p012).
- Operator workflow: "Experiment!" stressed repeatedly; waveform shape sets sharpness; sine = round, triangle = diamond (p013).
- Recurring building blocks: sin/cos pair for circles; sin and sin*cos for figure-8; ramp*ramp for perspective acceleration; triangle phase-locked to vertical for linear width taper; high-freq line-locked oscillator for stairstep.
- Imperfection hint: stability depends on phase-locking ("adjust for maximum stability"); otherwise star fields crawl.
