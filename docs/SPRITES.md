# Making a PetPal sprite

A PetPal creature is one PNG holding a grid of animation frames, plus a
`sprite.toml` describing which frames belong to which animation. Drop the two
into a folder, put the folder in `%APPDATA%\PetPal\sprites\`, and it appears in
**Tray > Sprite**.


## The quick way: start from a working copy

**Tray > Sprite > Make a copy to edit...**

That writes the creature you are currently using out as a real sprite sheet and
opens the folder. You get `creature.png` and a `sprite.toml` that already
matches it exactly, so you are repainting a finished sheet instead of guessing a
layout.

1. Export a copy.
2. Open `creature.png` in any pixel editor that keeps transparency — Aseprite,
   Piskel, Paint.NET, GIMP, Krita.
3. Repaint the cells. Stay on the grid; you can change one cell or all of them.
4. Rename the folder to whatever you want the creature called. That name is what
   shows up in the menu.
5. **Tray > Sprite** and pick it. If PetPal is already running, use
   **Reload config & sprites**.

Everything below is for building a sheet from scratch instead.


## If your sheet came out of an image generator

It almost certainly needs processing first. The usual faults are an opaque
background instead of alpha, sprites placed freehand so there is no real grid,
a ground plate painted under each pose, and a render several times larger than
the pixel art it depicts.

[`tools/sheetconv`](../tools/sheetconv/) fixes all four:

```bash
cd tools/sheetconv && cargo build --release
./target/release/sheetconv my-sheet.png out --cell 64 --cols 8 --rows 5 --trim-bottom 14
```

Run it with no options first and read what it prints — it reports the grid it
detected and the largest sprite it found, which is how you tell whether you need
`--cols`/`--rows` or `--key-white`. Full notes in its
[README](../tools/sheetconv/README.md).

When generating a sheet, you will save yourself the `--trim-bottom` step by
asking for **no ground, no platform, no shadow** and a **transparent
background** up front.


## Sheet layout

The PNG is sliced into a grid of equal cells. Frames are numbered
**left to right, top to bottom, starting at 0**:

```
+----+----+----+----+----+----+----+----+
|  0 |  1 |  2 |  3 |  4 |  5 |  6 |  7 |
+----+----+----+----+----+----+----+----+
|  8 |  9 | 10 | 11 | 12 | 13 | 14 | 15 |
+----+----+----+----+----+----+----+----+
| 16 | 17 | ...
```

* Columns are `image_width / frame_width`, rounded **down**. A partial column on
  the right is ignored, so a stray pixel of padding costs you a whole column.
* Any cell size works, and cells need not be square. Three of the built-ins use
  32x32; the monkey uses 36x32 because its arms and tail need the width. A long
  creature is happier in something like 48x32.
* Any number of frames. The built-ins use 52, but two is a valid sheet.
* Must be an **RGBA PNG**. Indexed-colour PNGs are rejected — if your editor
  saves indexed by default, export as 32-bit / "RGBA" / "true colour + alpha".


## The manifest

`sprite.toml`, in the same folder as the PNG:

```toml
image = "creature.png"      # required — filename, same folder
frame_width = 32            # required
frame_height = 32           # required

[anims.idle]
frames = [0, 1, 2, 1]       # explicit indices; repeats and any order allowed
frame_ms = 240              # optional, default 120

[anims.walk]
row = 1                     # shorthand: a whole row...
count = 6                   # ...or the first `count` cells of it
frame_ms = 90
```

Each animation takes **either** `frames` **or** `row`/`count`. If you give both,
`frames` wins. `count` defaults to the full row width and is capped at it.

Indices pointing outside the grid are dropped silently, so trimming your sheet
will not crash anything — the animation just gets shorter.

**An index pointing at an empty cell is not an error, and that is the one to
watch for.** Nothing warns you; the creature simply becomes invisible for the
whole of that animation. If a pose disappears in use, count your cells — an 8x6
grid is 48 cells whether or not you drew in all of them, and it is easy to write
`38` when the art you meant is at `40`.

### A complete manifest

All ten animations on a 52-frame, 8-wide sheet — copy this and renumber:

```toml
image = "creature.png"
frame_width = 32
frame_height = 32

[anims.idle]
frames = [0, 1, 2, 3, 4, 5, 6, 7]
frame_ms = 240

[anims.walk]
row = 1
frame_ms = 65

[anims.run]
row = 2
frame_ms = 38

[anims.fall]
frames = [24, 25]
frame_ms = 140

[anims.sleep]
frames = [26, 27, 28, 29, 30, 31]
frame_ms = 480

[anims.annoyed]
frames = [32, 33, 34, 35]
frame_ms = 110

[anims.drag]
frames = [36, 37]
frame_ms = 200

[anims.alert]
frames = [38, 39, 40, 41]
frame_ms = 150

[anims.sit]
frames = [42, 43]
frame_ms = 420

[anims.climb]
frames = [44, 45, 46, 47, 48, 49, 50, 51]
frame_ms = 85
```

That is exactly the layout **Make a copy to edit...** writes out, so you can
export a built-in and compare against this.


## The animations

Ten names are recognised. **Every one is optional: anything you leave out falls
back to `idle`**, so a sheet with a single idle frame already works, and
`idle` + `walk` is enough to feel alive.

| Name | When it plays | Frames used by the built-ins | ms |
|---|---|---|---|
| `idle` | Standing still. Also the fallback for every other animation. | 8 | 240 |
| `walk` | Wandering along a surface. | 8 | 65 |
| `run` | Chasing the cursor, and sprinting the length of a ledge. | 8 | 38 |
| `fall` | Falling **and** jumping — it is the whole airborne state. | 2 | 140 |
| `sleep` | Curled up after `sleep_after_idle_secs` of no input. | 6 | 480 |
| `annoyed` | Whole-machine CPU above `cpu_annoy_percent`. | 4 | 110 |
| `drag` | While you are holding it with the mouse. | 2 | 200 |
| `alert` | Reminders, an app opening, and clicking the pet. | 4 | 150 |
| `sit` | Resting. Alternates with `idle`; `roam` sets how often. | 2 | 420 |
| `climb` | Going up the side of a window too tall to jump onto. | 8 | 85 |

Frame counts are yours to choose — those are just what the built-ins do. Walk
and run read much better with 6-8 frames than with 4.

### Drawing `climb`

It is the newest animation and the least obvious, so it is worth its own note.

`climb` plays when the pet scales the vertical edge of a window — the only way
it can reach a window whose top is more than 280px up. It is the one exception
to the fallback rule: leave it out and you get **`walk`**, not `idle`, because a
creature going up a wall should at least be moving its legs.

Frames are never rotated. The pet keeps facing the window it is holding, so:

* Draw the wall as being **in front** of the creature — to its right, since you
  draw facing right.
* Head up toward the lip, limbs reaching and pushing, body pressed in.
* The feet are **not** on the ground, so ignore the bottom-row rule here.
* Two frames alternating is enough. The built-ins use eight at 85ms.

If your creature's silhouette leaves no room for a raised limb, do not force
one — a stagger works. The built-ins each solved this differently: Pal reaches
with both forelimbs, Vader staggers his boots because there is nowhere to raise
one clear of the cape, the mouse bunches all four legs forward, and the monkey
goes hand over hand because it is the one drawn with its near-side limbs on top
of its body.

Animations loop, and switching animation restarts it at frame 0.


## Two rules that catch people out

**1. Draw facing right.** PetPal mirrors the whole cell horizontally when the
creature moves left. Left-facing art comes out backwards. This also means any
asymmetric detail — an eyepatch, a satchel, lettering — swaps sides when it
turns around. Keep such details central, or accept the flip.

**2. Put the feet on the bottom row of the cell.** The bottom edge of the cell
is what gets aligned with the taskbar or window edge. Empty rows below the feet
make the creature hover; feet hanging past the bottom make it sink. This applies
to every grounded pose — `idle`, `walk`, `run`, `sit`, `sleep`, `annoyed`,
`alert`. It does **not** apply to `fall`, `drag` or `climb`, where the creature
is off the ground and free to sit anywhere in the cell.

And a third worth knowing: **the horizontal centre of the cell is the
creature's position.** Empty space on one side shifts the creature off its own
standing point, which shows up as it hugging one end of a ledge. Centre the
body, and let a tail or trailing effect be the thing that sits off-centre.


## Installing it

```
%APPDATA%\PetPal\sprites\
    my-creature\
        creature.png
        sprite.toml
```

The folder name is the menu label and the value written to `sprite` in
`config.toml`. Any immediate subfolder containing a `sprite.toml` is picked up —
loose files at the top level are ignored, which is why this guide can live here.

**Tray > Sprite > Open sprites folder...** takes you there.

The menu list is rebuilt each time you open it, so a newly added folder appears
without restarting. To re-read a sheet you have just edited, use **Reload
config & sprites**.


## When something is wrong

A sheet that fails to load never stops PetPal — it falls back to the built-in
creature and reports the reason in a tray balloon.

| Message | Cause |
|---|---|
| `sprite.toml: ...` | TOML syntax error, or a missing required key. |
| `<path>: The system cannot find the file` | `image =` does not match the PNG's filename. |
| `frame_width and frame_height must be non-zero` | Missing or zero size keys. |
| `... is NxM, smaller than one WxH frame` | Cell size larger than the image. |
| `unsupported indexed PNG` | Re-export as RGBA / 32-bit. |
| `Unknown sprite "..."` | `sprite` in config.toml names a folder that is not there. |

If the creature loads but looks wrong, it is almost always the grid: check that
the PNG's width is an exact multiple of `frame_width`.

Silent faults, which produce no message at all:

| Symptom | Cause |
|---|---|
| One animation shows nothing | Its `frames` point at cells you left empty. |
| Hovers above every surface | Empty rows below the feet in the grounded poses. |
| Sinks into the taskbar | Feet drawn past the bottom row. |
| Hugs one end of a ledge | Body off-centre in the cell; centre it and let the tail overhang. |
| Faces the wrong way | Art drawn facing left. Everything must face right. |
| Walks up windows | You left out `climb`, so it fell back to `walk`. |


## Notes and limits

* **Size on screen** is `frame size x scale`, and `scale` (1-6, **Tray > Size**)
  is global. Design at a cell size that looks right at 3x, or expect users to
  change it.
* **Speed** is the `speed` setting in pixels per second, the same for every
  custom sheet. The built-ins each scale it a little to suit themselves; custom
  sheets run at 1.0x.
* **Turning around at ledge ends** assumes the creature occupies roughly the
  middle three quarters of its cell. A sheet with a lot of empty margin will
  turn earlier than it looks like it should.
* **Recolouring** via the `[colors]` block in `config.toml` only affects the
  built-in creatures. A PNG sheet is used exactly as drawn.
* Frames are decoded once at load and kept in memory: a sheet costs roughly
  `cells x frame_width x frame_height x 4` bytes.
