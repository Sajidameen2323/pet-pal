# Kept sprite sheets

Source copies of custom creatures, kept here so they live alongside the code
instead of only in `%APPDATA%`.

**Nothing in this folder is a build input.** `build.rs` only reads
`assets/petpal.rc` and `assets/petpal.ico`; these sheets are never compiled in.
PetPal loads sprites at runtime from `%APPDATA%\PetPal\sprites\`, so to actually
use one, copy its folder there:

```
xcopy /E /I assets\sprites\cat "%APPDATA%\PetPal\sprites\cat"
```

Then pick it from **Tray > Sprite**.

See [../../docs/SPRITES.md](../../docs/SPRITES.md) for the sheet format.

| Folder | |
|---|---|
| `cat/` | Tabby cat, 64x64 cells, 8x5 grid. Converted from a 1586x992 white-background contact sheet: background keyed out by flood fill from the borders, sprites re-fitted to an exact grid, feet bottom-aligned. |
