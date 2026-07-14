# Bundled mpv (native TV player)

The **Stations → TV** feature plays channels in a native [mpv](https://mpv.io)
window (ffmpeg-backed, plays every format; VLC-class). To ship a self-contained
app that needs no system install, the mpv binary + its dylibs live here and are
copied into the app bundle via the `bundle.resources` `"mpv/"` mapping in
`src-tauri/tauri.conf.json`.

## Populate this directory

Run once before `pnpm build:app` / `tauri build`:

```sh
scripts/get_mpv.sh
```

That fetches a **relocatable** mpv into this folder so the app finds it at
`<resource_dir>/mpv/mpv` (see `resolve_mpv` in `src-tauri/src/commands/video.rs`).

## Development

In `pnpm tauri dev` there is no bundle, so `resolve_mpv` falls back to an `mpv`
on your `PATH` (`brew install mpv` / `apt install mpv` / winget). The feature
works either way — bundling only matters for shipped builds.

> This placeholder file keeps the resource glob valid before `get_mpv.sh` runs.
> The real binary + dylibs are git-ignored (see `.gitignore`).
