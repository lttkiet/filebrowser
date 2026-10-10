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

Preview and edit UTF-8 text files up to 1 MiB. The desktop GUI also previews BMP, GIF, JPEG, PNG, and WebP images, with decoded allocation and output dimensions bounded. Index a folder tree to search eligible file contents; the index stays in memory and is cleared after app-managed file changes or when hidden-item visibility changes. ZIP files can be browsed without extraction, with UTF-8 text previews for archive members. Properties let you change read-only state and modified time.

## TUI keys

| Key | Action |
| --- | --- |
| Arrows / `j` / `k` | Move through entries |
| Enter / Right | Open folder or launch file |
| Backspace / Left | Go to parent folder |
| `/` | Search recursively in the current folder tree |
| `i` / `I` | Index this folder tree / toggle content search |
| `v` / `e` | Preview selected UTF-8 text / edit it |
| `P` | Edit read-only state and modified time |
| `a` | Browse a selected ZIP archive |
| Space | Add/remove current entry from selection |
| `n` / F2 | Create folder / rename |
| `c` / `x` / `p` | Copy / cut / paste |
| `d` | Move selection to trash |
| `h` / `s` | Toggle hidden files / cycle sort |
| `q` | Quit |

In the TUI text editor, press Ctrl+S to save, Esc to close, or `x` to discard edits. Content indexing is limited to 100,000 files, 100,000 directories, and 128 MiB of text per build; each file is limited to 1 MiB. Image preview is GUI-only, ZIP browsing is read-only, and content indexes are not saved between launches.

The Linux milestone targets Ubuntu (`.deb`) and openSUSE (`.rpm`).
