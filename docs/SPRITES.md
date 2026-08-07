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
* Any cell size works. The built-ins use 32x32; a long creature is happier in
  something like 48x32.
* Any number of frames. The built-ins use 44, but two is a valid sheet.
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


## The animations

Nine names are recognised. **Every one is optional: anything you leave out falls
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

Frame counts are yours to choose — those are just what the built-ins do. Walk
and run read much better with 6-8 frames than with 4.

Animations loop, and switching animation restarts it at frame 0.


## Two rules that catch people out

**1. Draw facing right.** PetPal mirrors the whole cell horizontally when the
creature moves left. Left-facing art comes out backwards. This also means any
asymmetric detail — an eyepatch, a satchel, lettering — swaps sides when it
turns around. Keep such details central, or accept the flip.

**2. Put the feet on the bottom row of the cell.** The bottom edge of the cell
is what gets aligned with the taskbar or window edge. Empty rows below the feet
make the creature hover; feet hanging past the bottom make it sink.

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
