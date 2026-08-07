# sheetconv

Fits an arbitrary contact sheet onto a PetPal sprite grid.

Sheets that come out of an image generator are almost never usable as-is. This
fixes the four things that are usually wrong with them:

| Problem | What sheetconv does |
|---|---|
| Opaque background instead of alpha | `--key-white` floods in from the image borders. Flooding from the edge rather than keying by colour means white *inside* the sprite — paws, chest, teeth — is kept. |
| Sprites placed freehand, no real grid | Detects each sprite, then re-lays them on an exact grid. |
| A ground plate or shadow under each pose | `--trim-bottom N` drops N pixels off the bottom of every sprite. |
| Rendered far larger than the art it depicts | `--cell N` sets the output cell size and downsamples with an area filter. |

Every frame is scaled by **one shared factor**, so the creature does not pulse
as it animates, and each is anchored with its **feet on the bottom row** and
centred on the ink under the body — not the whole bounding box, so a swinging
tail does not slide the sprite sideways.

## Building

```bash
cd tools/sheetconv
cargo build --release
```

It is its own workspace, so `cargo build` in the repo root still builds only the
app. The binary lands at `tools/sheetconv/target/release/sheetconv.exe`.

## Usage

```
sheetconv <input.png> <output-dir> [options]

  --cell N           output cell size in pixels (default 64)
  --trim-bottom N    drop N source pixels from the bottom of every sprite
  --key-white        remove a white background by flooding from the borders
  --cols N           force the column count instead of detecting it
  --rows N           force the row count instead of detecting it
  -h, --help
```

It writes `creature.png` and a starter `sprite.toml` into the output directory.

## Working out the options

Run it once with no options and read what it prints:

```
input      1536x1024
grid       7 columns x 5 rows = 35 cells
sprites    35 found, largest 376x133, scaled by 0.165
```

**If the grid count is wrong**, two sprites are touching and their bands merged
— the giveaway is a "largest sprite" about twice the width of the others. Force
it with `--cols` / `--rows`.

**If the art has a ground plate**, find its height in the source image: open the
PNG, zoom to one sprite, and measure from the bottom of the plate to where the
feet actually start. Try that number, look at the result, adjust. Around 10-20
px is typical for a 1500px-wide sheet. Too small leaves a pale sliver under the
feet; too large starts eating the toes.

**If nothing is found at all**, the background is probably opaque — add
`--key-white`.

## Example

The `mouse_v2` sheet in this project: 1536x1024, transparent background already,
but with a plate painted under every pose and two sprites touching.

```bash
sheetconv mouse_v2.png out --cell 64 --cols 8 --rows 5 --trim-bottom 14
```

Then copy `out/` into `%APPDATA%\PetPal\sprites\<name>\` and pick it from
**Tray > Sprite**.

## What it will not do

It does not guess which pose means "annoyed". The generated `sprite.toml` maps
rows 0, 1 and 2 to idle, walk and run — which is how these sheets are nearly
always laid out — and lists the remaining frame numbers as comments for you to
assign. See [../../docs/SPRITES.md](../../docs/SPRITES.md) for the animation
names.

Automatic plate detection is deliberately absent. The plate is opaque artwork
sitting flush against the feet, and every heuristic tried for separating the two
mis-fired on the lying-down and tumbling poses. An explicit number you can see
the effect of is more honest than a clever guess that quietly eats a foot.
