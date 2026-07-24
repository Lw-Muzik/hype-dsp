# HypeMuzik APO

The free Windows system-wide EQ Audio Processing Object.

`hm_apo.dll` is dropped here (not committed) by the Windows release build:

```
cargo build -p hm-apo --release --target x86_64-pc-windows-msvc
copy ..\..\target\x86_64-pc-windows-msvc\release\hm_apo.dll .\hm_apo.dll
```

`tauri.conf.json` bundles `apo/*`, so the installer resolves the DLL at
`resource_dir()/apo/hm_apo.dll`. This README keeps the resource glob valid when
the DLL isn't staged. See `docs/windows-apo.md`.
