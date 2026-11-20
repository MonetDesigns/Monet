# Monet app icon

A **blue pen mark** on a white tile: the nib points down-left, with a lighter highlight along one
edge. It is Monet's own mark, distinct from the upstream project's icon.

## Palette

| Colour | Hex | Used for |
|---|---|---|
| Paper | `#FFFFFF` | the full-bleed field |
| Monet blue | `#0B63CE` | the pen mark |
| Highlight | `#6FA8FF` | the mark's light edge |

## Geometry

A 512-unit tile (`viewBox="0 0 512 512"`), rounded square with `rx=112`, no border. The macOS
renders pad it onto Apple's 824/1024 icon grid; Windows and Linux use the tile edge to edge with
its rounded corners.

## Provenance

Derived from the Monet brand mark in [`docs/brand/monet.svg`](../../docs/brand/monet.svg), drawn by
the project owner. License: see `LICENSE.txt`.

## Files

- `monet.svg`: canonical master (white tile, blue pen mark).
- `monet-small.svg`: lighter trace; also the hicolor scalable icon.
- `monet-<size>.png` (16–1024): renders on the macOS grid; `packaging/macos/package.sh` builds
  the `.icns` from them.
- `monet.ico`: Windows icon, 16–256 px, embedded in the `.exe` by `apps/figcraft/build.rs`.
- `hicolor/<size>/apps/app.monet.design.png` (16–512) and `hicolor/scalable/...svg`:
  Linux icon theme; the 256 px one is also the runtime window icon off macOS.

## Regenerate

Replace `monet.svg` (and `monet-small.svg`), then run `cargo xtask icon`. It needs `resvg`
(`cargo install resvg`).
