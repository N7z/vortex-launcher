<div align="center">

# vortex-launcher

<img
  width="128"
  height="128"
  alt="icon128"
  src="https://github.com/user-attachments/assets/da4038ea-3b8c-49f5-8c5b-ae6a3b2db7f1"
/>

</div>

Unofficial Linux launcher for [Vortex](https://playvortex.io). Native GUI, no webview, no terminal. It downloads the Windows build and Proton for you on first run and starts the game under a private wine prefix.

Not affiliated with or endorsed by the Vortex developers. No game files are redistributed here; everything is fetched from the official download URL at runtime.

<div align="center">
<img width="575" height="709" alt="image" src="https://github.com/user-attachments/assets/bdae428e-b50a-4469-8072-3bb2d0617c0c" />
</div>

## Why another one

| | vortex-launcher | Riko | Tempest |
|---|---|---|---|
| UI | native (egui) | webview | none, CLI only |
| binary | 9.0 MB | bundles a browser engine | small |
| memory idle | ~65 MiB PSS | webview-class | n/a |
| first run | click Install, done | | edit configs by hand |

Measured on my machine (Arch, Wayland, Mesa): 9.0 MB stripped binary, window up in ~40 ms, 65 MiB PSS idle (149 MiB RSS, most of it shared Mesa pages).

## Requirements

- glibc, libGL/libEGL, and either Wayland or X11 client libraries. These are on any desktop install already.
- `python3` on PATH. Proton's launch script is Python; the launcher says so plainly if it is missing.
- Vulkan drivers for your GPU, for DXVK/vkd3d inside Proton.
- ~2.5 GB free disk for Proton plus the game.

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

## Links from the browser

On start the launcher writes `~/.local/share/applications/vortex-launcher.desktop` and claims `x-scheme-handler/vortex`, so pressing Play on the website opens the game here. It is rewritten only when missing or out of date, e.g after the binary moves.

Only one launcher runs at a time: a second start hands its link to the first over `$XDG_RUNTIME_DIR/vortex-launcher.sock` and exits. A link that arrives before anything is installed waits for the install instead of being dropped.

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
    logs/launcher.log   what the launcher itself did, rewritten each run
$XDG_CONFIG_HOME/vortex-launcher/config.json
```

By default the game is allowed to update itself on start; the checkbox turns that off by setting `VORTEX_NO_UPDATE=1`.

## Black screen, but the sound is playing

If the game window is black (or shows only the void) while audio and input keep working, tick **Use Microsoft's shader compiler** in the launcher and start the game again.

Vortex compiles its shaders at runtime. Under Proton that goes through vkd3d-shader, which on some GPUs rejects shaders the real compiler accepts, the game then keeps presenting frames it never drew into, so you get a black screen on a game that's running fine. The checkbox installs Microsoft's `d3dcompiler_47` into the prefix (via [winetricks](https://github.com/Winetricks/winetricks), which must be installed) and tells wine to use it.

You can confirm it is this and not something else in `logs/game.log`, which will hold both of:

```
vkd3d:warn:vkd3d:hresult_from_vkd3d_result Invalid shader.
warn:vkd3d-proton:dxgi_vk_swap_chain_record_render_pass:
     Application is presenting user index 0, but it has never been rendered to.
```

The option is off by default because it changes the shader compiler for everyone who turns it on, and most GPUs never hit the bug.

Special thanks to KitKat for the Logo!

## License

AGPL-3.0-or-later, see [LICENSE](LICENSE).
