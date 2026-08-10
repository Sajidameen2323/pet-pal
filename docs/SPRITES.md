# Making a PetPal creature

A PetPal creature is **two files in a folder**:

```
my-creature\
    creature.png     the artwork: a grid of small pictures
    sprite.toml      a text file saying which pictures are which animation
```

Put that folder in `%APPDATA%\PetPal\sprites\` and it appears in **Tray >
Sprite**. That is the whole system. Everything below is detail.

There are three ways to get those two files. Pick one:

| | Best if | Effort |
|---|---|---|
| **A. Repaint a copy** | You want a sure thing | Low |
| **B. Generate with AI** | You want something new and cannot draw | Medium |
| **C. Draw from scratch** | You can use a pixel editor | High |

---

# A. Repaint a copy

The safest route, because you start from a sheet that already works.

1. **Tray > Sprite > Make a copy to edit...**
   This writes the creature you are using now into
   `%APPDATA%\PetPal\sprites\<name>-copy\` and opens the folder. You get
   `creature.png` and a `sprite.toml` that already matches it.
2. Open `creature.png` in any editor that keeps transparency — Paint.NET, GIMP,
   Krita, Aseprite, Piskel. **Not** MS Paint; it will destroy the transparency.
3. Repaint the pictures. Stay inside the grid squares. Change one or all.
4. Rename the folder. **That name is what shows in the menu.**
5. **Tray > Sprite** and pick it. If PetPal is already running, use **Reload
   config & sprites** first.

You never have to touch `sprite.toml` on this route.

---

# B. Generate with AI

This section is written to be **copied and pasted**. The layout below is exact;
do not change the numbers unless you understand the reference sections at the
end.

## B1. The sheet you are asking for

**4 columns across, 10 rows down. One animation per row. 4 pictures per row.
40 pictures total.**

One animation per row is the point: you never have to count picture numbers,
and the settings file below is the same every time.

```
            col 0     col 1     col 2     col 3
row 0  |  idle    |  idle    |  idle    |  idle    |   standing still
row 1  |  walk    |  walk    |  walk    |  walk    |   walking
row 2  |  run     |  run     |  run     |  run     |   running
row 3  |  fall    |  fall    |  fall    |  fall    |   falling through the air
row 4  |  sleep   |  sleep   |  sleep   |  sleep   |   asleep
row 5  |  annoyed |  annoyed |  annoyed |  annoyed |   cross
row 6  |  drag    |  drag    |  drag    |  drag    |   held up by the mouse
row 7  |  alert   |  alert   |  alert   |  alert   |   surprised
row 8  |  sit     |  sit     |  sit     |  sit     |   sitting down
row 9  |  climb   |  climb   |  climb   |  climb   |   climbing a wall
```

**Every one of the 40 squares must have artwork in it.** An empty square is not
an error — the creature just turns invisible for that whole animation, with no
warning. This is the single most common thing that goes wrong.

## B2. The prompt

Paste this into an image generator. Replace the one line in capitals.

```
Create a pixel-art sprite sheet on a fully transparent background.

THE CREATURE: A SMALL FRIENDLY RED DRAGON.

Layout — follow exactly:
- A grid of 4 columns and 10 rows. 40 equal squares. 64x64 pixels per
  square, so the whole image is 256 pixels wide and 640 pixels tall.
- Do NOT draw the grid lines. Do NOT write any numbers, labels or text.
- Transparent background. No ground, no floor, no platform, no shadow,
  no scenery, no border, no frame.
- Exactly one creature per square, the same creature every time, the
  same size every time.
- The creature FACES RIGHT in every single square.
- In each square the creature's FEET TOUCH THE BOTTOM EDGE of that
  square, except in rows 3, 6 and 9 where it is off the ground.
- Centre the creature left-to-right in its square. A tail may hang off
  to one side; the body must be centred.

The 10 rows, top to bottom. Each row is a 4-step loop that repeats:
1. IDLE - standing still, facing right, breathing gently. Small
   up-and-down movement only. Blink on the last step.
2. WALK - a 4-step walking cycle, legs swapping, body rising slightly
   over the planted foot.
3. RUN - a 4-step run. Body lower and leaning forward, legs further
   apart, faster-looking than the walk.
4. FALL - dropping through the air. Limbs out, eyes wide, slight tumble
   across the 4 steps. Not touching any ground.
5. SLEEP - lying down curled up, eyes closed, gentle breathing. Small
   sleepy "z" letters floating above it in the later steps.
6. ANNOYED - standing, cross, frowning, ears back, stamping or shaking.
   A small puff of steam above the head.
7. DRAG - dangling in mid-air as if held up by the scruff of the neck.
   Arms and legs hanging limp, eyes wide, gently swinging.
8. ALERT - surprised and pleased, head and ears up, looking right. A
   small exclamation mark above the head.
9. SIT - sitting down on its bottom, relaxed and settled. Very small
   movement only.
10. CLIMB - climbing a vertical wall that is to its RIGHT. Facing that
    wall, head tilted up, limbs reaching up and pushing down, body
    pressed toward the wall. Climbing hand over hand across the 4
    steps. Feet not on any ground.

Chunky readable pixel art, bold dark outline, flat colours, no
anti-aliasing, no gradients, no soft shadows.
```

**Why each rule is there:** the creature is mirrored automatically when it walks
left, so left-facing art comes out backwards. The bottom edge of each square is
what gets lined up with your taskbar, so feet off the bottom row make it hover
or sink. And the tool in the next step finds the creature by looking for a
transparent gap around it, so a background or a ground plate confuses it.

## B3. Fix the grid

Image generators almost never produce an exact grid. What comes back usually
has: an opaque background, pictures placed by eye rather than on a grid, a
little ground plate under each pose, and a picture several times larger than
pixel art needs to be.

`sheetconv.exe` fixes all four. It sits next to `petpal.exe`. Open the folder
containing it, type `cmd` in the address bar, press Enter, and run:

```
sheetconv.exe "C:\path\to\what-the-ai-made.png" out
```

It prints what it found. This is a real run on a sheet with all four of the
usual faults:

```
input      1024x2560
keyed      white background removed by border flood
grid       4 columns x 10 rows = 40 cells
trim       26 px off the bottom of each sprite
sprites    40 found, largest 140x119, scaled by 0.443
wrote      out\creature.png  (256x640, 64px cells)
wrote      out\sprite.toml
```

**The line to check is the grid line. It must say
`4 columns x 10 rows = 40 cells`.** If it says anything else, the result is
wrong even though the tool reported success — it does not know what you were
aiming for, so it will not complain.

| The grid line says | What happened | Add |
|---|---|---|
| `1 columns x 1 rows = 1 cells` | The background is solid, so the whole image looks like one big picture | `--key-white` |
| Any other wrong count | Two pictures are touching, so they merged | `--cols 4 --rows 10` |
| `4 columns x 10 rows` but there is a pale sliver under the feet | A ground plate is painted under each pose | `--trim-bottom 12`, then adjust |
| `no sprites found` | The image is blank, or `--key-white` erased everything | Check you passed the right file |

Most AI output needs all of them at once:

```
sheetconv.exe input.png out --cols 4 --rows 10 --key-white --trim-bottom 12
```

For `--trim-bottom`, measure the plate: open the PNG, zoom into one picture, and
count the pixels from the bottom of the plate up to where the feet actually
start. Too small leaves a pale sliver under the feet; too big starts eating the
toes. Run it, look at `out\creature.png`, adjust the number, run it again.

`--key-white` floods in from the edges of the image rather than deleting every
white pixel, so white **inside** your creature — eyes, teeth, a chest patch — is
kept.

## B4. The settings file

`sheetconv` writes a starter `sprite.toml`, but it cannot know which row you
meant as "annoyed". **Replace its contents with exactly this** — it matches the
layout above with nothing to work out:

```toml
image = "creature.png"
frame_width = 64
frame_height = 64

[anims.idle]
row = 0
frame_ms = 240

[anims.walk]
row = 1
frame_ms = 90

[anims.run]
row = 2
frame_ms = 55

[anims.fall]
row = 3
frame_ms = 140

[anims.sleep]
row = 4
frame_ms = 480

[anims.annoyed]
row = 5
frame_ms = 140

[anims.drag]
row = 6
frame_ms = 200

[anims.alert]
row = 7
frame_ms = 160

[anims.sit]
row = 8
frame_ms = 420

[anims.climb]
row = 9
frame_ms = 110
```

`row = 5` means "use the whole of row 5". Because every row is full, that is all
you need — no picture numbers anywhere.

If `sheetconv` gave you a cell size other than 64, change **both** `frame_width`
and `frame_height` to match, and nothing else.

## B5. Install and check

Copy the `out` folder into `%APPDATA%\PetPal\sprites\`, rename it to whatever
you want the creature called, then **Tray > Sprite** and pick it.

Then walk this list:

- [ ] It appears in the menu — if not, the folder has no `sprite.toml` in it
- [ ] It is visible — if a pose vanishes, that row of the sheet is empty
- [ ] It faces the way it is walking — if not, the art was drawn facing left
- [ ] Its feet are on the taskbar, not floating above or sunk into it
- [ ] It sits in the middle of where it stands, not off to one side

---

# C. Draw from scratch

Same layout as section B — 4 columns, 10 rows, one animation per row, every
square filled — but you control the grid, so no converting is needed. Use the
same `sprite.toml` from B4.

Two frames per animation is enough to start; walk and run look much better with
6 or 8. If you want different frame counts per row, see the reference below.

---

# Reference

## The three rules that decide whether it looks right

**1. Draw facing right.** PetPal flips the whole square horizontally when the
creature moves left. This also means asymmetric details — an eyepatch, a
satchel, lettering — swap sides when it turns around. Keep them central, or
accept the flip.

**2. Feet on the bottom row of the square.** The bottom edge is what gets lined
up with the taskbar or a window's edge. Empty rows below the feet make the
creature hover; feet drawn past the bottom make it sink. This applies to
`idle`, `walk`, `run`, `sit`, `sleep`, `annoyed` and `alert`. It does **not**
apply to `fall`, `drag` or `climb` — the creature is off the ground in those
and can sit anywhere in the square.

**3. Centre the body left-to-right.** The middle of the square is where the
creature actually *is*. Empty space on one side shifts it off its own standing
point, which shows up as it hugging one end of a ledge. Centre the body and let
a tail or a trailing effect be the thing that overhangs.

## The ten animations

Every one is optional. Anything you leave out falls back to `idle`, so a sheet
with one idle picture already works, and `idle` + `walk` is enough to feel
alive. The exception is `climb`, which falls back to `walk`.

| Name | When it plays |
|---|---|
| `idle` | Standing still. Also the fallback for every other animation. |
| `walk` | Wandering along a surface. |
| `run` | Chasing the cursor, and sprinting the length of a ledge. |
| `fall` | Falling **and** jumping — it is the whole airborne state. |
| `sleep` | Curled up after the idle timer, or on **Go to sleep**. |
| `annoyed` | Whole-machine CPU above `cpu_annoy_percent`. |
| `drag` | While you are holding it with the mouse. |
| `alert` | Reminders, an app opening, and clicking the pet. |
| `sit` | Resting. Alternates with `idle`; **Roam** sets how often. |
| `climb` | Going up the side of a window too tall to jump onto. |

Animations loop, and switching animation restarts at the first picture.

### Drawing `climb`

The least obvious one. It plays when the pet scales the vertical edge of a
window — the only way it can reach a window whose top is more than 280 pixels
up. Pictures are never rotated, and the pet keeps facing the window it is
holding, so:

* Draw the wall as being **in front** of the creature — to its right, since you
  draw facing right.
* Head up toward the top edge, limbs reaching and pushing, body pressed in.
* Feet are not on the ground, so rule 2 does not apply.
* Two pictures alternating is enough.

If your creature's shape leaves no room for a raised limb, do not force one — a
stagger works. The built-ins each solved it differently: Pal reaches with both
forelimbs, Vader staggers his boots because there is nowhere to raise one clear
of the cape, the mouse bunches all four legs forward, and the monkey goes hand
over hand.

## Sheet layout rules

Pictures are numbered **left to right, top to bottom, starting at 0**:

```
+----+----+----+----+
|  0 |  1 |  2 |  3 |
+----+----+----+----+
|  4 |  5 |  6 |  7 |
+----+----+----+----+
|  8 |  9 | ...
```

* The number of columns is `image width / frame_width`, rounded **down**. A
  stray pixel of padding on the right costs you a whole column.
* Any square size works, and squares need not be square. Three built-ins use
  32x32; the monkey uses 36x32 because its arms and tail need the width.
* Any number of pictures. The built-ins use 52; two is a valid sheet.
* Must be an **RGBA PNG**. Indexed-colour PNGs are rejected — if your editor
  saves indexed by default, choose 32-bit / "RGBA" / "true colour + alpha".

## The manifest, in full

```toml
image = "creature.png"      # required - filename, same folder
frame_width = 32            # required
frame_height = 32           # required

[anims.idle]
frames = [0, 1, 2, 1]       # explicit numbers; repeats and any order allowed
frame_ms = 240              # optional, default 120

[anims.walk]
row = 1                     # shorthand: a whole row...
count = 6                   # ...or the first `count` squares of it
frame_ms = 90
```

Each animation takes **either** `frames` **or** `row`/`count`. If you give both,
`frames` wins. `count` defaults to the full row width and is capped at it.

Numbers pointing outside the grid are dropped silently, so trimming a sheet will
not crash anything — the animation just gets shorter.

**A number pointing at an empty square is not an error, and that is the one to
watch for.** Nothing warns you; the creature becomes invisible for that whole
animation. A 8x6 grid is 48 squares whether or not you drew in all of them, and
it is easy to write `38` when the art you meant is at `40`.

## The built-in layout

**Make a copy to edit...** writes 52 pictures on an 8-wide sheet. Three
animations share row 4, so this layout needs counting — it exists to match what
the built-in creatures actually contain, not because it is a good starting
point. Prefer the one-animation-per-row layout in section B.

```toml
image = "creature.png"
frame_width = 32
frame_height = 32

[anims.idle]
frames = [0, 1, 2, 3, 4, 5, 6, 7]
frame_ms = 240

[anims.walk]
frames = [8, 9, 10, 11, 12, 13, 14, 15]
frame_ms = 65

[anims.run]
frames = [16, 17, 18, 19, 20, 21, 22, 23]
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

## Installing

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

## If something is wrong

A sheet that fails to load never stops PetPal — it falls back to a built-in and
reports why in a tray balloon.

| Message | Cause |
|---|---|
| `sprite.toml: ...` | Typo in the settings file, or a missing required line. |
| `<path>: The system cannot find the file` | `image =` does not match the PNG's filename. |
| `frame_width and frame_height must be non-zero` | Missing or zero size lines. |
| `... is NxM, smaller than one WxH frame` | Square size is larger than the image. |
| `unsupported indexed PNG` | Re-save as RGBA / 32-bit. |
| `Unknown sprite "..."` | `sprite` in config.toml names a folder that is not there. |

Faults that produce **no message at all**:

| Symptom | Cause |
|---|---|
| One animation shows nothing | Those squares of the sheet are empty. |
| Nothing appears in the Sprite menu | No `sprite.toml` inside the folder, or the folder is nested one level too deep. |
| Hovers above every surface | Empty rows below the feet in the grounded poses. |
| Sinks into the taskbar | Feet drawn past the bottom of the square. |
| Hugs one end of a ledge | Body off-centre in its square. |
| Faces the wrong way | Art drawn facing left. Everything must face right. |
| Pulses bigger and smaller as it moves | The creature is not the same size in every square. |
| Walks up windows instead of climbing | You left out `climb`, so it fell back to `walk`. |

If it loads but the grid is visibly wrong, check that the PNG's width divides
exactly by `frame_width`.

## Notes and limits

* **Size on screen** is `square size x scale`, and `scale` (1-6, **Tray >
  Size**) is global. Design at a size that looks right at 3x.
* **Speed** is the `speed` setting in pixels per second. The built-ins each
  scale it to suit themselves; custom sheets all run at 1.0x.
* **Turning around at ledge ends** assumes the creature fills roughly the middle
  three quarters of its square. A sheet with a lot of empty margin turns earlier
  than it looks like it should.
* **Recolouring** via `[colors]` in `config.toml` only affects the built-in
  creatures. A PNG sheet is used exactly as drawn.
* Pictures are decoded once at load and kept in memory: a sheet costs about
  `squares x frame_width x frame_height x 4` bytes.
