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

**One single image. 8 squares across, 6 squares down. 48 squares in total.**

Ask for 8x6 rather than some other shape because 8x6 is a 4:3 picture, which is
a shape image generators are good at. Tall thin sheets come back distorted.

At 128 pixels per square that is a 1024x768 image, a size every generator
handles. The converter in step B3 shrinks it to the 64-pixel squares PetPal
wants, so ask big and let the tool reduce it.

This is the exact map. The numbers inside are square numbers, and **squares
count from 0** — that is how the file format numbers them, and it is what the
settings file in B4 refers to. The rows are counted from 1 in the prompt below,
the way a person would read them.

```
             1st  2nd  3rd  4th  | 5th  6th  7th  8th
    row 1 |    0    1    2    3       4    5    6    7  |  IDLE, all 8
    row 2 |    8    9   10   11      12   13   14   15  |  WALK, all 8
    row 3 |   16   17   18   19      20   21   22   23  |  RUN, all 8
    row 4 |   24   25   26   27   |  28   29   30   31  |  CLIMB 4 | FALL 4
    row 5 |   32   33   34   35   |  36   37   38   39  |  SLEEP 4 | ANNOYED 4
    row 6 |   40   41   42   43   |  44   45 | 46   47  |  ALERT 4 | SIT 2 | HELD 2
```

The first three rows are one animation each. The last three are split, because
ten animations do not divide evenly into six rows — but you never have to work
any of it out, because the settings file in B4 already has the numbers.

**Every one of the 48 squares must have artwork in it.** An empty square is not
an error — the creature just turns invisible for that whole animation, with no
warning. This is the single most common thing that goes wrong.

## B2. The prompt

Paste this into an image generator. Change only the line in capitals.

```
Generate ONE single image: a complete pixel-art sprite sheet.
Do not give me separate images. Everything must be in one picture.

THE CREATURE: A SMALL FRIENDLY RED DRAGON.

SIZE AND GRID - follow exactly:
- One image, 1024 pixels wide and 768 pixels tall.
- Divided into a grid of 8 columns and 6 rows = 48 equal squares,
  each 128x128 pixels.
- Do NOT draw the grid lines. Do NOT write any numbers, labels,
  captions or text anywhere in the image.
- Fully transparent background. No ground, no floor, no platform,
  no shadow, no scenery, no border, no frame, no colour behind
  the creature.
- Exactly one creature in every square. All 48 squares are used.
- It is the SAME creature at the SAME size in every square.
- The creature FACES RIGHT in every single square. Never left.
- Its FEET TOUCH THE BOTTOM EDGE of its square, except in the
  squares listed below as off the ground.
- Centre it left-to-right in its square. A tail may hang to one
  side, but the body is centred.

WHAT GOES IN EACH SQUARE, reading left to right, top to bottom:

Row 1, all 8 squares - IDLE: standing still, breathing gently.
  Only a small up-and-down movement across the 8 steps. Blink near
  the end. This is a loop.

Row 2, all 8 squares - WALKING: a smooth 8-step walk cycle. Legs
  swap over, the body rises slightly over the planted foot. Loops.

Row 3, all 8 squares - RUNNING: an 8-step run. Body lower and
  leaning forward, legs reaching further, clearly faster than the
  walk. Loops.

Row 4, squares 1-4 - CLIMBING a wall that is to its RIGHT: facing
  that wall, head tilted up, limbs reaching up and pushing down,
  body pressed toward the wall, moving hand over hand. Off the
  ground - feet do not touch the bottom of the square.

Row 4, squares 5-8 - FALLING through the air: limbs out, eyes
  wide, alarmed, tumbling slightly. Off the ground.

Row 5, squares 1-4 - ASLEEP: lying down curled up, eyes closed,
  gentle breathing. Small sleepy "z" letters floating above it in
  the later squares.

Row 5, squares 5-8 - ANNOYED: standing, cross, frowning, ears
  back, stamping or shaking with irritation. A small puff of steam
  above the head.

Row 6, squares 1-4 - SURPRISED and pleased: head and ears up,
  looking right, delighted. A small exclamation mark above the
  head.

Row 6, squares 5-6 - SITTING DOWN on its bottom, relaxed and
  settled. Only a tiny movement between the two.

Row 6, squares 7-8 - BEING HELD UP in mid-air by the scruff of
  the neck: arms and legs hanging limp, eyes wide, swinging
  gently. Off the ground.

STYLE: chunky readable pixel art, bold dark outline, flat colours,
no anti-aliasing, no gradients, no soft shadows.
```

**Why each rule is there:** the creature is mirrored automatically when it walks
left, so left-facing art comes out backwards. The bottom edge of each square is
what gets lined up with your taskbar, so feet off the bottom row make it hover
or sink. And the tool in the next step finds the creature by looking for a
transparent gap around it, so a background or a ground plate confuses it.

**Generators are bad at exact grids.** Expect the first attempt to be off. The
next step fixes placement, background and size — what it cannot fix is a missing
pose or a creature that changes size between squares, so check for those and ask
again if you see them.

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

On typical generator output that plain run prints this, and it is **wrong**:

```
input      1024x768
grid       1 columns x 1 rows = 1 cells
sprites    1 found, largest 1024x768, scaled by 0.061
wrote      out\creature.png  (64x64, 64px cells)
wrote      out\sprite.toml
```

It says `wrote`, so it looks like it worked. It did not. The background was
solid, so the whole image read as one enormous creature, and the entire sheet
has been squashed into a single 64x64 square.

**The line to check is the grid line. It must say
`8 columns x 6 rows = 48 cells`.** Nothing else matters until it does. The tool
has no idea what you were aiming for, so it will never complain — a wrong result
still ends in `wrote`.

| The grid line says | What happened | Add |
|---|---|---|
| `1 columns x 1 rows = 1 cells` | The background is solid, so the whole image looks like one big picture | `--key-white` |
| Any other wrong count | Two pictures are touching, so they merged | `--cols 8 --rows 6` |
| `8 columns x 6 rows` but there is a pale sliver under the feet | A ground plate is painted under each pose | `--trim-bottom 13`, then adjust |
| `no sprites found` | The image is blank, or `--key-white` erased everything | Check you passed the right file |

Most AI output needs all of them at once. Here is the same image again, done
properly:

```
sheetconv.exe input.png out --cols 8 --rows 6 --key-white --trim-bottom 13
```

```
input      1024x768
keyed      white background removed by border flood
grid       8 columns x 6 rows = 48 cells
trim       13 px off the bottom of each sprite
sprites    48 found, largest 72x61, scaled by 0.862
wrote      out\creature.png  (512x384, 64px cells)
wrote      out\sprite.toml
```

`48 found` and a `512x384` sheet — that is the one you want.

For `--trim-bottom`, measure the plate: open the PNG, zoom into one picture, and
count the pixels from the bottom of the plate up to where the feet actually
start. Too small leaves a pale sliver under the feet; too big starts eating the
toes. Run it, look at `out\creature.png`, adjust the number, run it again.

`--key-white` floods in from the edges of the image rather than deleting every
white pixel, so white **inside** your creature — eyes, teeth, a chest patch — is
kept.

## B4. The settings file

`sheetconv` writes a starter `sprite.toml`, but it cannot know which squares you
meant as "annoyed". **Delete what it wrote and paste exactly this** — it matches
the map in B1, so there is nothing to work out:

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

[anims.climb]
frames = [24, 25, 26, 27]
frame_ms = 110

[anims.fall]
frames = [28, 29, 30, 31]
frame_ms = 140

[anims.sleep]
frames = [32, 33, 34, 35]
frame_ms = 480

[anims.annoyed]
frames = [36, 37, 38, 39]
frame_ms = 140

[anims.alert]
frames = [40, 41, 42, 43]
frame_ms = 160

[anims.sit]
frames = [44, 45]
frame_ms = 420

[anims.drag]
frames = [46, 47]
frame_ms = 200
```

`row = 1` means "the whole of row 1", which is why the first three animations
need no numbers. The rest name their squares directly, and between them they use
all 48 exactly once.

If `sheetconv` reported a cell size other than 64, change **both**
`frame_width` and `frame_height` to that number, and nothing else. Do not change
the frame numbers — they are positions in the grid, not pixel sizes.

`frame_ms` is how long each square is shown, in thousandths of a second. Lower
is faster. Adjust to taste; nothing breaks.

## B5. Install and check

Copy the `out` folder into `%APPDATA%\PetPal\sprites\`, rename it to whatever
you want the creature called, then **Tray > Sprite** and pick it.

Then walk this list:

- [ ] It appears in the menu — if not, the folder has no `sprite.toml` in it
- [ ] Every pose is visible — if one vanishes, those squares of the sheet are
      empty. Watch it for a minute, or drag it about to force the poses.
- [ ] It faces the way it is walking — if not, the art was drawn facing left
- [ ] Its feet are on the taskbar, not floating above or sunk into it
- [ ] It sits in the middle of where it stands, not off to one side

---

# C. Draw from scratch

You control the grid, so nothing needs converting. Two options:

* **Use the B1 map** — 8 across, 6 down, 64x64 squares, image 512x384 — and the
  `sprite.toml` from B4 works unchanged.
* **Make up your own layout** and write the manifest yourself. See the manifest
  reference below; you can put any animation on any squares.

Two pictures per animation is enough to start. Walk and run look much better
with 6 or 8.

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
