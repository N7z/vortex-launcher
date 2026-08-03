# vortex-launcher

Unofficial Linux launcher for [Vortex](https://playvortex.io). Native GUI, no webview, no terminal. It downloads the Windows build and Proton for you on first run and starts the game under a private wine prefix.

Not affiliated with or endorsed by the Vortex developers. No game files are redistributed here; everything is fetched from the official download URL at runtime.

## Why another one

| | vortex-launcher | Riko | Tempest |
|---|---|---|---|
| UI | native (egui) | webview | none, CLI only |
| binary | 9.0 MB | bundles a browser engine | small |
| memory idle | ~65 MiB PSS | webview-class | n/a |
| first run | click Install, done | | edit configs by hand |

Measured on this machine (Arch, Wayland, Mesa): 9.0 MB stripped binary, window up in ~40 ms, 65 MiB PSS idle (149 MiB RSS, most of it shared Mesa pages).

## Requirements

- glibc, libGL/libEGL, and either Wayland or X11 client libraries. These are on any desktop install already.
- `python3` on PATH. Proton's launch script is Python; the launcher says so plainly if it is missing.
- Vulkan drivers for your GPU, for DXVK/vkd3d inside Proton.
- ~2.5 GB free disk for Proton plus the game.

No Steam required.

## Build

```sh
cargo build --release
./target/release/vortex-launcher
```

## What it does on first run

1. Fetches the latest GE-Proton release from GitHub, verifies its sha512, unpacks it.
2. Downloads `Vortex-Windows.zip` (resumable) and extracts `Vortex.exe`.
3. Asks you to sign in.
4. Runs the game with `proton run`, with `STEAM_COMPAT_DATA_PATH` pointing at a prefix owned by this launcher.

An existing Steam or `compatibilitytools.d` Proton is picked up and reused instead of downloading, when one is present.

## Where things go

```
$XDG_DATA_HOME/vortex-launcher/
    game/           extracted game, Vortex.exe
    proton/         GE-Proton builds this launcher downloaded
    prefix/         the wine prefix (pfx/ inside)
    compat-client/  STEAM_COMPAT_CLIENT_INSTALL_PATH stand-in
    downloads/      partial downloads, resumed on the next run
    session.json    your session token, mode 0600
    logs/game.log       last session's game output
    logs/launcher.log   what the launcher itself did
$XDG_CONFIG_HOME/vortex-launcher/config.json
```

Updates are detected with a one-byte ranged GET against the download URL, comparing `ETag`/`Last-Modified` (HEAD answers 404 there, so a ranged GET is the cheap probe).

By default the game is allowed to update itself on start; the checkbox turns that off by setting `VORTEX_NO_UPDATE=1`.

## License

AGPL-3.0-or-later, see [LICENSE](LICENSE).
