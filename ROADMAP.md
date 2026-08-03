# Roadmap

The MVP is deliberately small: install, play, update, and clear errors. Everything
below is out of scope until that is solid.

## Next

- join a specific server, not just a game (`/games/{id}/play?instance=`, the ids are
  already in `/api/games`)
- game thumbnails and friends, both behind the session
- packaging: AppImage first, then AUR and Flatpak
- gamemode / MangoHud toggles

## Later

- multiple game instances
- mod integration
- theming

## Known gaps in the project

- Proton runs on the host, not in the sniper container. Works in practice, but a distro with unusual library versions can still trip it.
- No cancel for the extraction step, only for downloads.
- Update checks only cover the game, not the installed Proton build.
