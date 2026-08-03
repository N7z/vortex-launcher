# Roadmap

The MVP is deliberately small: install, play, update, and clear errors. Everything
below is out of scope until that is solid.

## Next

- umu-launcher support, so Proton runs inside the Steam Linux Runtime container the
  way Valve intends, instead of straight on the host
- pick and pin a Proton build from the UI, keep more than one installed
- prefix maintenance: reset prefix, open a winetricks-free repair path
- packaging: AppImage first, then AUR and Flatpak
- launcher self-update
- gamemode / MangoHud toggles
- desktop entry and icon installed on first run

## Later

- account login in the launcher (the client already handles its own auth)
- multiple game instances
- mod integration, hooking up the tooling from VortexStuff
- theming

## Known gaps in the MVP

- Proton runs on the host, not in the sniper container. Works in practice, but a
  distro with unusual library versions can still trip it.
- No cancel for the extraction step, only for downloads.
- Update checks only cover the game, not the installed Proton build.
