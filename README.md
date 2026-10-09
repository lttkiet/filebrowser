# File Browser

A Rust desktop file browser with a keyboard-driven terminal interface for headless systems.

See [PRODUCT_SPEC.md](PRODUCT_SPEC.md) for the detailed product scope, behavior, UI design, platform support, and roadmap.

## Run

```powershell
cargo run
cargo run -- --gui
cargo run -- --tui
```

The app opens at the current user's home folder. It chooses the GUI when a graphical session is available and otherwise uses the TUI when attached to an interactive terminal.

The desktop layout combines Explorer-style tabs and sortable Details columns with a persistent, user-managed Favorites/Locations sidebar and Finder-style navigation controls. Pin the current folder with the star beside the address path, or pin and remove folders from their context menus. Open folders in a new tab from the file list or sidebar. Layout, hidden-item visibility, Details pane state, and sort order persist across launches.

In list and Details views, the whole row is clickable, including its metadata columns. Single-click an item to select it and double-click to open it. Dragging starts only after pressing and holding an item and moving it; hovering over a name does not start a drag. Drag selected items onto folders, sidebar locations, or empty folder space to move them, or drop files from another app into a folder. These pointer interactions are available in the desktop GUI; the TUI uses its keyboard copy/cut/paste queue.

## TUI keys

| Key | Action |
| --- | --- |
| Arrows / `j` / `k` | Move through entries |
| Enter / Right | Open folder or launch file |
| Backspace / Left | Go to parent folder |
| `/` | Search recursively in the current folder tree |
| Space | Add/remove current entry from selection |
| `n` / F2 | Create folder / rename |
| `c` / `x` / `p` | Copy / cut / paste |
| `d` | Move selection to trash |
| `h` / `s` | Toggle hidden files / cycle sort |
| `q` | Quit |

The Linux milestone targets Ubuntu (`.deb`) and openSUSE (`.rpm`).
