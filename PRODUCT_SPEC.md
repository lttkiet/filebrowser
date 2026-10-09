# Rust File Browser — Product Specification

**Version:** 0.1.0 baseline  
**Status:** Desktop GUI and TUI are implemented. Linux packaging and desktop integration remain roadmap work.  
**Product name:** File Browser

This specification describes the current product as implemented in the repository, then identifies platform work that is still planned. It is intended to guide implementation, review, and acceptance testing. It does not imply that roadmap items are already shipped.

## 1. Product definition

File Browser is a local filesystem application with two Rust-authored interfaces:

- A desktop GUI using eframe/egui, following Windows Explorer conventions on Windows and Finder conventions on macOS.
- A keyboard-driven TUI using Ratatui and Crossterm for terminals, including headless systems.

Both interfaces use the same Rust filesystem core for listing, recursive name search, sorting, folder creation, rename, copy, move, and trash deletion. Platform adapters provide system file opening, trash behavior, modified-time display, and GUI file clipboard support where implemented.

The application starts in the current user’s home directory. Files remain on the local filesystem; the application has no account, sync, or cloud workflow.

## 2. Goals and product boundaries

### Goals

1. Make common file and folder operations available from either a desktop or terminal interface.
2. Keep operation behavior consistent through a shared Rust core.
3. Make navigation, sorting, search, and item selection predictable.
4. Protect existing destination items by asking how to handle transfer conflicts.
5. Keep filesystem work off the interactive UI loop and show operation outcomes.
6. Support Windows v1 and provide a documented Linux path for Ubuntu and openSUSE.

### Current scope

- Browse folders, navigate upward and through history, and open files with the operating system’s default application.
- Pin and unpin folders in the GUI Favorites sidebar; keep those shortcuts across launches.
- Remember GUI layout, hidden-item visibility, Details pane state, and sort order across launches.
- Show/hide hidden entries, sort entries, and recursively search names.
- Select multiple entries and create folders, rename, copy, move, and move items to the system trash.
- Use GUI drag-and-drop and platform file clipboard integration where available.
- Use an internal copy/cut/paste queue in the TUI.
- Choose GUI or TUI automatically, with explicit `--gui` and `--tui` overrides.

### Outside the current v0.1.0 scope

- File previews, content indexing, archive browsing/editing, file editing, and property editing.
- Undo/redo, operation cancellation, persistent tabs, per-folder view preferences, or user-defined Locations beyond Favorites.
- Cloud storage, accounts, synchronization, remote administration, or a built-in terminal.
- macOS Finder file clipboard interoperability, a Linux package build pipeline, or a Linux-specific desktop file clipboard adapter.

## 3. Users and primary tasks

### Desktop user

Uses a mouse and keyboard to browse local drives and common folders, search for names, move or copy items, manage folders, and open files in their usual applications.

### Headless/terminal user

Uses keyboard navigation over SSH, a console, or another interactive terminal. Needs basic browsing and file operations without a graphical session, preview, or external application.

### Primary workflows

- Find an item by navigating folders or searching names below the current folder.
- Open a folder or launch a file.
- Select one or more items, copy/cut them, navigate to a destination, and paste.
- Drag items onto a folder row in the GUI to move them; drop files from another application to copy them into the current folder or a hovered folder.
- Create a folder, rename an item, or send selected items to the system trash.
- Resolve destination conflicts by replacing, skipping, or canceling.

## 4. Platform and launch behavior

| Platform/context | Auto launch | GUI | TUI | Notes |
| --- | --- | --- | --- | --- |
| Windows desktop | GUI | Yes | Yes, when an interactive terminal is attached | GUI uses Windows file clipboard formats and system file opening/trash integration. |
| macOS desktop | GUI unless running over SSH without display forwarding | Yes | Yes, when an interactive terminal is attached | The GUI uses the native desktop session even when `DISPLAY` and `WAYLAND_DISPLAY` are unset and follows Finder labels and shortcuts. File-manager file clipboard interoperability is unavailable; the app-local queue supports transfers within File Browser. |
| macOS headless SSH terminal | TUI when stdin/stdout are interactive | No display session | Yes | Set `DISPLAY` for a forwarded graphical session or pass `--tui` explicitly. |
| Linux graphical session | GUI when `DISPLAY` or `WAYLAND_DISPLAY` is set | Built from eframe with X11/Wayland support | Yes | Linux packaging and file-manager clipboard integration are roadmap items. |
| Linux headless interactive terminal | TUI | No graphical session | Yes | TUI operations use the shared core and app-local clipboard queue. |
| No GUI and no interactive stdin/stdout | Error | No | No | The program reports that a graphical session or interactive terminal is required. |

`--gui` forces GUI mode and fails clearly if no graphical session is detected. `--tui` forces TUI mode and requires interactive stdin and stdout. Passing both modes is an error. `--help` prints usage.

## 5. Shared interaction and filesystem behavior

### 5.1 Navigation and starting location

- Start at the user home directory.
- A directory entry opens through GUI double-click/context action or TUI Enter/Right.
- Parent navigation uses GUI Up/breadcrumbs/history and TUI Backspace/Left. GUI Up (button or shortcut) selects and focuses the child folder just left in the parent listing. Breadcrumb navigation opens the chosen ancestor without forcing an item selection.
- The GUI keeps an in-memory history for each tab. Back and Forward navigate that tab’s history and restore the prior scroll position, visible selection, and focus when those items still exist; opening a new path after going Back discards the forward branch.
- Folder context menus and sidebar location context menus can open a folder in a new tab.
- Open Containing Folder from a search result or the Details pane navigates to the parent and selects/focuses that item in the listing.
- The TUI has one current path and no tab/history stack.

### 5.2 Entries and metadata

The shared entry model carries path, display name, directory flag, symlink flag, size, and optional modified time.

- Directory symlinks are identified as symlinks, displayed with a folder link glyph, and can be navigated like folders. Search can start from an explicitly opened folder link but does not descend through linked directories found below it.
- A file’s displayed size is its metadata length. GUI folders show no size; TUI folders show no size.
- GUI type labels follow platform conventions: Windows directories use `File folder`, while macOS and Linux use `Folder`; directory symlinks use `Folder link`, file symlinks use `Symbolic link`, regular files use extension-based labels such as `TXT file`, or `File` when there is no extension.
- TUI uses compact `[D]`, `[L]`, and `[F]` markers.
- Modified time uses the local time zone on Windows, macOS, and Linux. The GUI displays month/day/year with a 12-hour clock and AM/PM.

### 5.3 Hidden entries

Hidden entries are excluded by default. On Windows, hidden means the Windows Hidden attribute or a dot-prefixed name. On non-Windows platforms, a dot-prefixed name is hidden. The GUI exposes a `View > Hidden items` toggle; the TUI toggles with `h`.

### 5.4 Sorting

Directories remain grouped before files. Sort keys are:

- **Name:** case-insensitive natural ordering; digit runs compare by numeric value (`file2` before `file10`).
- **Type:** lowercase file extension, then natural name order.
- **Size:** metadata size, then natural name order.
- **Date modified:** modified time, then natural name order.

GUI sorting is available from sortable column headings and the Sort menu. Re-selecting the active sort reverses direction; the Sort menu also exposes Descending. The TUI cycles Name → Type → Size → Date modified → Name with `s`, and `S` reverses direction. The active sort and direction appear in the file list title.

### 5.5 Recursive search

- Search matches names only; it does not inspect file contents.
- Search is case-insensitive and searches descendants of the current directory.
- Hidden entries and their subtrees are omitted while hidden-item visibility is off.
- Search does not descend through directory symlinks found below its root. If the user explicitly navigates into a folder link, search can traverse that root while still skipping linked child directories.
- Inaccessible or disappearing child entries are skipped during recursive search; failure to open the search root is reported.
- Results use name ordering and can be selected and acted on like other entries. Their context menu can open the containing folder. In Details layout, a responsive Location column shows each result’s parent directory when space allows. In Icons layout, search cards include the parent directory; truncated search results show their full path on hover in all layouts.
- The GUI debounces searches while the field changes, cancels stale pending scans, and shows results in the active view. A clear button with the `Esc` hint cancels the current query. The TUI enters search with `/`, updates as text is entered, and exits search input with Enter or Esc.

### 5.6 Copy, move, replace, and skip

- GUI Copy/Cut writes the selected paths to the Windows file clipboard and also retains an app-local fallback queue. Paste prefers file paths from the OS clipboard; if unavailable, it uses the app-local queue. macOS and Linux currently use only the app-local queue for GUI transfers; file-manager clipboard interoperability is not implemented.
- A folder's context menu offers Paste Into Folder so clipboard items can be transferred there without opening it first.
- Copy Path writes plain-text paths to the system clipboard and clears the app-local file queue so Paste cannot reuse stale file selections.
- Items in the app’s cut queue appear dimmed in GUI list/icon views and TUI rows until a complete move clears the queue. The TUI footer also keeps the pending cut-item count visible while browsing elsewhere.
- GUI drag of selected items onto a directory row, icon card, sidebar location, empty pane, or another tab moves items within the same volume and copies items across volumes. In Details and List layouts the full folder row accepts drops. Dragging an unselected item transfers only that item. Hold Ctrl on Windows/Linux or Option on macOS to copy regardless of volume.
- File and folder rows can be selected across the full row width. A normal hover keeps the standard pointer; dragging starts only when the user presses and moves an item.
- The drag cursor indicates Move or Copy for the hovered destination; files and other invalid destinations show a not-allowed cursor. Releasing on an invalid file row does not transfer items and reports that files must be dropped on a folder or the empty pane. Dropping external files onto a tab copies them into that tab's folder.
- Folder rows, icon cards, sidebar locations, tabs, and the empty file pane show an outline while an internal drag hovers over them, making the destination visible before the drop.
- Dropping files from another application copies them into the hovered directory when its full row, icon card, or sidebar location is targeted; the destination folder is outlined during the drag. If no directory is targeted, the current directory is used.
- Drag-and-drop skips items already in the destination and rejects dropping a folder onto itself or into its own descendants before showing a conflict prompt.
- TUI Copy/Cut/Paste use an app-local path queue and never require a graphical clipboard.
- Copying directories is recursive. Copying a symlink copies the link itself where the platform API supports it; it does not recursively traverse the target.
- A destination conflict prompts for Replace, Skip, or Cancel. One choice applies to the batch: Replace replaces conflicts, Skip transfers non-conflicting items and reports skipped names, and Cancel stops the transfer. A conflict detected before a batch starts is preflighted so cancellation does not partially transfer earlier items.
- Replacement first stages the incoming item and moves the previous destination aside. If installation fails, the previous destination is restored where possible.
- Moving across filesystems falls back to copy followed by source removal. Errors are surfaced in the UI.
- A cut queue is cleared after a successful, complete move. Failed or skipped transfers keep the queue for retry.
- After a successful copy or move into the currently open folder, transferred items are selected when they remain visible.
- Multi-item operations are sequential rather than batch-atomic. If an OS error occurs after earlier items completed, those earlier changes remain; the current UI reports the operation error but does not provide a per-item history or rollback.

### 5.7 Rename, folder creation, and deletion

- New folder creates one child directory in the current directory. Names must be a single path component; an empty name or an existing destination is rejected.
- Rename keeps the item in its current parent. An existing destination name is rejected rather than replaced.
- Delete means “move to system trash,” not permanent recursive removal through the UI. The GUI asks for confirmation; the TUI requires `y`/`Y` and accepts `n`/Esc to cancel.
- Permission errors, missing paths, invalid names, and OS integration failures are displayed as operation errors.

### 5.8 Opening files

- GUI double-click or context-menu Open launches a file through the system default application.
- The Details pane provides Open for a single file or folder and Copy Path for a single selection; a multi-selection can copy all selected paths as newline-separated text.
- TUI Enter/Right attempts to launch the selected file through the system opener. If no suitable desktop application exists, the TUI reports that it could not launch it; file management itself remains available.
- The app does not preview file contents.

### 5.9 End-to-end journeys

**Find and open:** Start in Home, navigate using folders/breadcrumbs/history or search names below the current folder, select a result, and open a folder or launch a file. In the TUI, use `/`, type the name fragment, then open the focused result with Enter.

**Copy or move:** Select one or more entries, use Copy/Cut and then Paste in the destination, or drag the selection onto a GUI folder. If destination names conflict, choose Replace, Skip, or Cancel. A successful complete cut/move clears its internal queue; failed and skipped moves keep the queue.

**Create or rename:** Choose `+ New > Folder` or Ctrl/Command+Shift+N in the GUI, or `n` in the TUI. For rename, select an item and use Rename/F2. Enter a single child name and commit; invalid or existing names produce an error without replacing the existing item.

**Delete:** Select one or more items and press Delete or choose Delete in the GUI, then confirm. In the TUI, use `d` and confirm with `y`. The app asks the platform trash integration to move items to trash.

**Headless session:** Run `filebrowser --tui` in an interactive terminal. Navigate, search, manage items with the key map, and quit with `q` or Ctrl+C; an active file operation is allowed to finish before exit. Terminal raw mode and the alternate screen are restored on exit.

## 6. Desktop GUI specification

### 6.1 Window, visual language, and platform conventions

- Initial window size: 1180 × 760 logical points.
- Minimum inner window size: 800 × 560 logical points so the main commands and file list remain usable.
- Combine Explorer’s tabbed location and details table with Finder’s concise navigation controls and Favorites/Locations sidebar sections.
- Use the active egui light/dark theme. Standard item spacing is 6 points horizontally and 4 vertically; buttons use 9 × 5 points of horizontal/vertical padding, and inactive/hovered controls have 4-point corners. Inactive control fills are transparent; hover and pressed fills are `#EBEBEB`/`#DEDEDE` in light mode and `#343434`/`#414141` in dark mode. Selection and focus use the active system palette. Folder glyphs use Windows/Linux yellow or macOS Finder blue, with theme-aware outlines that keep their shape distinct on pane and selection backgrounds; files use a page outline; symlinks use a link mark.
- Keep the shared layout and file-operation model consistent while following platform labels and shortcuts: Windows uses Ctrl, Delete, and the Recycle Bin; macOS uses Command, Move to Trash, Return to rename, Command+O to open, and Command+I for Get Info.
- Use Explorer/Finder interaction conventions: single-click selects, double-click opens, right-click exposes item actions, and drag-and-drop begins only after a pressed pointer moves.
- The left and right side panels are resizable; each splitter shows a horizontal-resize cursor and a tooltip naming its pane. The table columns are resizable; its final column fills remaining width.
- The window title follows platform convention: Windows/Linux combine the current folder name and `File Browser`; macOS shows the current folder name alone.

### 6.2 Screen regions

| Region | Contents and behavior |
| --- | --- |
| Tabs | Folder icon and current folder name per tab; duplicate folder names include their parent folder, expanding to the parent path when that label is still ambiguous. Clicking anywhere in a tab, including its icon and blank margin, activates it. The active tab has a distinct background; the strip scrolls horizontally when tabs exceed the available width and stays pinned to the newest tab when already at the right edge; switching tabs restores that tab's search query, view mode, icon size, sort key and direction, scroll position, visible selection, and keyboard focus when those items still exist. Drag tabs to reorder them, with a before/after marker. Drop internal items onto a tab to transfer them into its folder; drop external files onto a tab to copy them there. `+` opens a tab at the current path; `×` or middle-click closes a tab. On macOS, `×` appears while hovering a tab; Windows/Linux keep it visible on every tab. The tab tooltip shows the full path and drag guidance, with the platform-specific close hint. Up to ten closed tabs can be reopened with Ctrl/Command+Shift+T. Right-click a tab to copy its path, duplicate it, close it, or close other tabs. Folder context menus can open folders in a new tab. Closing the only tab resets it to Home. Ctrl/Command+T opens a tab; Ctrl/Command+W closes the active tab; Ctrl+Tab and Ctrl+Shift+Tab cycle on Windows/Linux, while Control+Tab and Control+Shift+Tab cycle on macOS. |
| Command row | `+ New > Folder`, Cut, Copy, Paste, Rename, Delete (Move to Trash on macOS and Linux), Sort, View, Select. The command row wraps onto another line when the window is too narrow, keeping every action reachable. Selection-dependent commands are disabled when their prerequisites are missing; hovering a disabled Cut, Copy, Rename, or Delete/Move to Trash control explains what selection is needed, and disabled Paste explains that the file clipboard has no paths. Select provides select-all-visible (Ctrl/Command+A) and clear-selection (Esc) actions. View switches the active tab between Icons, List, and Details layouts, adjusts that tab's icon size in Icons view, toggles global hidden-item visibility without clearing an active search, and opens or hides the Navigation and Details panes. Hiding Navigation gives the file list more room while keeping the View menu available to restore it. The last active tab's view mode, icon size, and sort order initialize the next session; hidden-item visibility, panel visibility and widths, and window size are application-wide preferences. Each open tab retains its own view mode, icon size, sort key and direction, query, scroll position, selection, and keyboard focus for the current session. |
| Address row | Navigation pane toggle, Back, Forward, Up, and Refresh controls compact at narrower window widths; Back/Forward disable when history is unavailable and work with mouse side buttons; Up disables at the filesystem root. The row also provides clickable breadcrumb components or an editable path with Go and a search field scoped to the current folder. The current breadcrumb is emphasized and can be clicked to edit the address; its context menu can also edit the address, open it in a new tab, or copy its path. Breadcrumb and search widths adapt to the available space; a deep breadcrumb stays scrolled to the current folder while allowing horizontal access to ancestors. Ctrl/Command+L enters path-edit mode and selects the current path; Enter or Go navigates. Relative paths resolve from the current folder, and `~` expands to Home. F5 refreshes the folder, preserves the active search and retains selected items that remain visible. The `×` control displays a Clear search (Esc) tooltip and clears a non-empty query, canceling any pending scan. |
| Left navigation | Browse heading, Home, up to eight recent locations, persistent user-managed Favorites, and Locations (drive roots on Windows, mounted volumes on macOS, and mounted device, network, and user FUSE volumes on Linux; pseudo-filesystems and loop images are omitted) with distinct drive glyphs. Duplicate recent, favorite, or mount names include parent context, expanding to the parent path if needed. Recent Locations, Favorites, and Locations can be collapsed independently by clicking anywhere on their full-width heading row; each open state is remembered across launches. Recent locations update as folders are opened, persist across launches, and can be removed individually or cleared. Missing recent or favorite folders remain listed with an Unavailable label so they can be identified and removed. On first launch Favorites start with the standard folders that exist (Desktop, Documents, Downloads, Pictures, Music, Videos). Pin the current folder from the star beside the address path or a folder context menu; reorder favorites by dragging them within the sidebar, with a before/after marker; remove a favorite from its sidebar context menu or folder context menu. Right-click any location to copy its path and right-click an existing location to open it in a new tab. The pane scrolls vertically when its contents exceed the window height. Selecting an item navigates there. Default width is 218 points; it is resizable from 175 to 320 points, with a responsive maximum that preserves at least 320 points for the main pane; its preferred width is remembered across launches and restored when the window has room. |
| Main table | Details layout shows Name, Date modified, Type, Size columns as space allows; modification times use the system’s local time zone, and sizes scale from bytes through EB. Common file types have subtle category-colored document marks; unknown types stay neutral. On narrow layouts, metadata columns progressively collapse so Name remains readable. During recursive search, Details layout shows a responsive Location column for each result’s parent folder, with Type and Size appearing as additional width permits. The optional Details pane continues to expose the selected item’s metadata. Details and List rows, plus Icons cards, render only visible items so large folders stay responsive. List layout shows full-width icon/name rows without metadata columns; Icons layout shows selectable file/folder cards in a responsive grid. Each visible sortable header cell is clickable across its full width; Name/type/size/date headings change sort where shown, and the active heading shows direction with hover text explaining the current order and how to reverse it. The Sort menu marks the active key with an ascending/descending arrow, and its hover hint explains how selecting it again reverses direction. Sorting preserves the current selection. Clicking a row or icon card selects it; double-click opens it. |
| Details pane | Optional resizable, vertically scrollable pane for one selected item: a 42-point platform-colored file/folder glyph beside its name, type, clickable containing-folder location, size, modified time, Open/Open folder, and Copy Path. The name truncates to the available width and shows its full value on hover. Multiple selection shows the item count, total size for selected files, a note when folder sizes are excluded, common location when all selected items share one, and Copy Paths. With no selection, it shows the current folder with a 42-point folder glyph beside its name, its location, item or search-result count, and Copy Path. The item context menu offers Properties (Get Info on macOS); it preserves a multi-selection when the clicked item belongs to it, otherwise it selects the clicked item before showing the pane. Default width is 270 points; it is resizable from 220 to 360 points, with a responsive maximum that preserves at least 320 points for the main pane; its preferred width is remembered across launches and restored when the window has room. Toggle from View or the Details control in the status row. |
| Status row | Current result/item count, selected count and combined selected-file size when nonzero (folder sizes are excluded and explained on hover), transient operation/search/error status, an activity spinner while work is running, and Details toggle. Errors use the active theme's error color; partial outcomes and blocked actions use its warning color; normal progress and completion use the standard text color. The selected-item summary is capped at 280 points and truncates with a tooltip containing the complete summary. Transfer status names the item or item count and destination folder; the spinner tooltip repeats the current status, including when multiple operations overlap. |

Sidebar row behavior: the full row width, including its icon, highlights on hover and when selected, and navigates to Home, Recent Locations, Favorites, or Locations. Right-click anywhere on the row opens its context menu. Favorite rows can be dragged from anywhere on the row to reorder them; file drops onto an available folder row transfer files there.

Paste enablement: the command-row Paste button, empty-pane Paste action, and folder Paste Into Folder action are disabled when neither the system file clipboard nor the app-local queue contains file paths. While the GUI is open, clipboard availability is refreshed periodically so Explorer file-copy/cut changes are reflected without reopening the app. Ctrl/Command+V still checks the clipboard when invoked by keyboard and reports when no file paths are available.

Details pane controls: the View menu checkbox and status-row Details toggle show or hide the pane. Hovering either control explains the current action; the tooltip displays Alt+Shift+P on Windows/Linux and Option+Shift+P on macOS.

Conceptual desktop layout:

```text
┌─ Folder tab ─ Folder tab ──────────────────────────────────────────────── + ┐
├─ + New ─ Cut ─ Copy ─ Paste ─ Rename ─ Delete ─ Sort ─ View ─ Select ─────┤
├─ ← ─ → ─ ↑ ─ ↻ ─ Breadcrumb / Path ─────────────────────── Search ────────┤
├─ Browse / Favorites ──┬─ Name ─ Date modified ─ Type ─ Size ─┬─ Details ──┤
│                       │                                      │             │
│ Desktop               │  File and folder rows                │  Metadata   │
│ Documents             │  Full-row selection                  │  Open folder│
│ Downloads             │                                      │             │
│ Locations / drives    │                                      │             │
├─ Items ─ Selected ─────────────────────────────────────────── Details ────┤
└───────────────────────────────────────────────────────────────────────────┘
```

The Details pane is optional. When it is closed, the main table takes the available width.

The tab strip, address row, and status row are 38, 42, and 28 points high respectively. The command row takes the height required by its contents and grows when its controls wrap onto additional lines. Default table column widths are 390 points for Name, 175 for Date modified, 145 for Type, and the remainder for Size. Columns can be resized in the current session.

### 6.3 Selection and row targets

- Hovering any row cell highlights the full row; selected rows keep their selection highlight.
- A single click anywhere across the full row selects it, including unused width after the final table column through the visible edge of the file pane; hover feedback spans that same width.
- The item name/path and the right-click or Shift+F10 context-menu hint appear when hovering any part of the row.
- Clicking unused space in the file pane clears the current selection.
- Ctrl/Command-click toggles an individual item and makes it the next range-selection anchor. Shift-click selects the contiguous range from the existing anchor; if none exists, the first Shift-click establishes the anchor so later Shift-clicks extend from it.
- Right-click anywhere across a row selects it if it was not already selected and opens its context menu.
- Cut, Copy, and Delete/Move to Trash in a context menu act on the current selection; Open, Copy Path, and Rename act on the clicked item.
- The toolbar Rename action is enabled only for a single selected item; context-menu Rename always targets the row under the pointer.
- Context-menu Open, Properties/Get Info, Open in New Tab, Cut, Copy, Rename, and Delete/Move to Trash display their keyboard shortcuts for the active platform. Finder uses Command+O, Command+I, and Return for Open, Get Info, and Rename. Paste Into Folder is a destination-specific action and has no separate shortcut.
- In Details and List layouts, the full item row is the click target, drag source, and folder drop target; in Icons layout, the full card is the target. A simple click selects; click-and-drag publishes the selected item(s) as the drag payload. Sidebar locations, tabs, and the empty pane accept drops across their target surfaces. Hovering does not show the Grab cursor.
- Dragging selected rows carries the selected set; dragging an unselected row carries only that row.
- Every item context menu offers Open, Copy Path, Properties/Get Info, Cut, Copy, Rename, and Delete/Move to Trash. Search results also offer Open Containing Folder. Folder entries add Paste Into Folder, Open in New Tab, and Add/Remove from Favorites.
- Right-clicking empty file-pane space offers New Folder, Paste, Select all visible items, and Refresh, with the active platform’s shortcut shown beside each action.

### 6.4 Dialogs and transient states

- Confirmation and naming dialogs are centered modal surfaces with a dimmed backdrop; clicks cannot activate the file list underneath, and clicking outside cancels the current prompt.
- **New folder:** focused single-line name entry with Create/Cancel; Enter creates the folder and Esc cancels. After successful creation, the new folder is selected and focused.
- **Rename:** focused single-line name entry with Rename/Cancel. The filename is selected on open, excluding a recognized file extension so it can be preserved while editing the base name. Enter commits and Esc cancels. After successful rename, the renamed item is selected and focused when it remains visible.
- **Move to trash:** asks for confirmation and lists the affected item names; confirm with Move to Trash or cancel with Cancel/Esc.
- **Destination conflict:** shows the destination and conflicting item names, then offers Replace/Skip/Cancel when one or more target names already exist. Esc cancels.
- **Loading/searching/operation:** non-blocking UI with a status message; errors and skipped names remain visible after refresh.
- **Empty folder/search:** centered explanation distinguishes an empty folder from no search matches; folder state offers New folder and search state offers Clear search.
- **Loading/error:** an in-context spinner appears while the first folder listing or search is pending; failed locations show the error in the content area with a retry action that preserves the current path or search query.

### 6.5 GUI shortcuts

Unless explicitly written as Control on macOS, `Ctrl` means egui’s platform command modifier: Ctrl on Windows/Linux and Command on macOS.

| Shortcut | Action |
| --- | --- |
| Type a name prefix with no text field focused | Select the next visible item whose name starts with the typed text |
| Arrow keys | Move the selection; Icons layout follows the grid |
| Page Up / Page Down | Move selection by approximately one visible page |
| Shift+Up / Shift+Down | Extend the selection |
| Home / End | Select the first / last visible item |
| Enter | Open the focused item on Windows/Linux; rename the selection on macOS (Command+Down or Command+O opens it) |
| Ctrl+Enter | Open the focused folder in a new tab |
| Alt+Enter (Command+I on macOS) | Show Properties / Get Info for the selection |
| Shift+F10 | Open the focused item's context menu |
| Escape | Cancel path editing, clear a non-empty search, or clear selection (one action per press, in that order); clearing or dismissing search focus returns keyboard focus to the file list |
| Ctrl+A | Select all visible items |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste |
| Delete / Move to Trash (Command+Backspace on macOS) | Confirm move to trash |
| F2 | Rename selected item |
| Ctrl/Command+F (F3 on Windows/Linux) | Focus recursive search |
| Ctrl+H (Command+Shift+. on macOS) | Toggle hidden items |
| Ctrl/Command+L, Alt+D (Windows/Linux), Command+Shift+G (macOS) | Edit address path / Go to Folder |
| Ctrl+T / Ctrl+W | New tab / close active tab |
| Ctrl+Shift+T | Reopen most recently closed tab |
| Middle-click tab | Close tab |
| Ctrl+Tab / Ctrl+Shift+Tab (Control on macOS) | Next tab / previous tab |
| Ctrl+Shift+N | New folder |
| Alt+Left / Alt+Right (Command+[ / Command+] on macOS) | Back / forward |
| Alt+Up (Command+Up on macOS) | Navigate to parent folder |
| F5 | Refresh current folder |
| Alt+Shift+P (Option+Shift+P on macOS) | Toggle Details pane |

## 7. TUI specification

### 7.1 Screen layout

The TUI fills the terminal with three stacked regions:

1. **Path header** (three terminal rows): current path in a bordered panel; long paths compact from the left on grapheme boundaries using terminal display width to keep the current location visible without overflowing. Terminal control and bidi-formatting characters in displayed paths are escaped; the original path remains unchanged for navigation and file operations.
2. **Files list** (remaining height, minimum two rows): selection marker, type marker, and a name truncated to fit the terminal width. Terminal control and bidi-formatting characters are escaped in displayed names; file operations use the original names. Items in the app-local cut queue appear dimmed until a complete move clears the queue. At 52 columns and wider, file sizes occupy a right-aligned column; narrower terminals hide the size column to preserve name space. Directories do not show a size. The focused row uses reverse highlighting.
3. **Status/help footer** (six rows at widths of 80 columns or more, seven rows at 52–79 columns, nine rows at 40–51 columns, and ten rows below 40 columns; naming/path prompts use three, four, or five rows as terminal width decreases, and delete/conflict prompts use five): current status and a responsive keyboard legend, or the active prompt with item names and transfer destination where relevant. Errors are light red, partial transfer outcomes and blocked actions are yellow, and the keyboard legend stays in the terminal's default color. While search input is active, the general legend is replaced with responsive search instructions. Selection count appears in the status line; a pending cut queue keeps its item count visible while browsing other folders.

The UI is keyboard-only. It has no preview or separate details pane. Terminal raw mode and the alternate screen must be restored on normal exit and on errors after terminal entry.

When the folder is empty, a search has no matches, or a listing/search is still loading or has failed, the list area displays a short status instead of remaining blank. Search results replace the prior visible rows while a new query is in flight.

### 7.2 TUI key map

| Key | Action |
| --- | --- |
| Up/Down or `k`/`j` | Move the cursor |
| Page Up / Page Down | Move the cursor by one visible page |
| Home/End | Move to first/last visible item |
| Enter/Right | Open directory or launch file |
| Backspace/Left | Navigate to parent and restore focus to the folder you left |
| `g` / Ctrl+L | Enter a folder path and navigate there; relative paths use the current folder and `~` expands to Home |
| F5 | Refresh the folder while preserving the active search, selection, and cursor when still visible |
| `/` | Start recursive name search |
| Search text | Edit at the visible cursor; Left/Right move by grapheme, Home/End move to either end, Backspace/Delete remove one grapheme, and Ctrl+A clears the query. Results update as text changes; Enter keeps the query and exits input; Esc exits and clears search. The footer replaces its general key legend with responsive search-specific guidance while input is active. |
| Space | Toggle current item in the multi-selection |
| Ctrl+A | Select all visible items |
| Esc | Clear selection outside search/prompt mode |
| `n` | Prompt for a new folder name |
| F2 | Prompt to rename the selected item or current item |
| `c` / `x` / `p` | Copy / cut / paste through the internal queue |
| `d` | Confirm moving selected/current item(s) to trash |
| `h` | Toggle hidden items while preserving the active search, selection, and cursor when still visible |
| `s` / `S` | Cycle sort key / reverse sort direction |
| `q` | Quit and restore the terminal |
| Ctrl+C | Cancel the active prompt, or quit outside a prompt; waits for file operations to finish |

When no explicit selection exists, copy, cut, rename, and delete use the current cursor item if one exists. With a selection, those actions use the selected set.

### 7.3 TUI prompts

- Path, new-folder, and rename prompts show the insertion point; Left/Right move by grapheme, Home/End move to either end, Backspace/Delete remove one grapheme, Ctrl+A clears the input, Enter commits, and Esc cancels. Prompt instructions wrap into additional footer rows on narrow terminals, and long input keeps the cursor-visible portion in view.
- Delete shows the item count; `y` confirms, while `n` or Esc cancels.
- On destination conflicts, `r` replaces, `s` skips conflicting items, and Esc cancels.
- The footer reports loading/search/operation progress and errors. Resizing is handled by Ratatui’s layout on the next draw.

## 8. Functional requirements and acceptance criteria

| ID | Requirement | Acceptance criteria / evidence |
| --- | --- | --- |
| FR-01 | Launch at Home and select GUI/TUI according to session availability. | Automated mode tests cover GUI preference, terminal fallback, no-session error, and explicit overrides. |
| FR-02 | Navigate directories and open files. | GUI row double-click/context action and TUI Enter/Right navigate or invoke the system opener; GUI Up selects the child folder just left, and Open Containing Folder selects its search result in the parent listing. |
| FR-03 | List entries and show/hide hidden items. | Shared listing tests verify hidden filtering and folder-first ordering. |
| FR-04 | Sort by name, type, size, and modified date. | Automated tests exercise all four keys in ascending and descending modes; GUI header/menu and TUI cycle are smoke-checked. |
| FR-05 | Search names recursively without traversing linked child directories. | Search tests cover nested results, explicit linked roots, and symlink safety; GUI interaction confirms typing debounce, stale-scan cancellation, active-view results, and clear/Esc behavior. |
| FR-06 | Select one or more items and operate on the active selection. | GUI hover highlights the full Details/List row through the visible pane edge and shows the item/context-menu hint; single-click anywhere from the icon through the far-right row edge, including unused space after the final table column, selects it; Ctrl/Command toggles an item and resets the range anchor; repeated Shift-clicks extend from the established anchor across layouts. TUI Space selection and selected-or-cursor fallback work. |
| FR-07 | Create folders and rename without replacing an existing name. | Automated tests cover successful creation/rename, invalid names, existing names, and path errors. |
| FR-08 | Copy and move files/folders, preserving symlinks where supported. | Core tests cover recursive copy, move, symlinks where available, self-descendant prevention, and failure handling. |
| FR-09 | Resolve batch conflicts by Replace, Skip, or Cancel. | Core tests verify old destination preservation, skipped items, and cancel preflight. |
| FR-10 | Delete through the OS trash integration after confirmation. | GUI/TUI confirmation flow and platform smoke checks. |
| FR-11 | Support internal drag/drop and external file drop in the GUI. | Interaction check: click alone on any part of a Details/List row selects; pressing and moving from the row or icon card starts transfer; the full folder row accepts internal and external drops; dropping on a file row is rejected with a not-allowed cursor and no transfer; internal drops move within a volume, copy across volumes, and copy with Ctrl/Option; external drops copy. |
| FR-12 | Interoperate with Windows file clipboard in the GUI. | Explorer copy/cut/paste round-trip for files and folders. |
| FR-13 | Use app-local copy/cut/paste in the TUI. | Headless terminal transfer smoke test, independent of GUI clipboard. |
| FR-14 | Keep UI responsive and report operation outcomes. | Large-directory/search/transfer smoke checks and visible error/skipped-item states; the GUI status row uses theme error color for failures and warning color for partial outcomes or blocked actions, while the TUI footer uses light red and yellow respectively without recoloring its keyboard legend; while transferring, the status identifies the source item or item count and destination folder, and the GUI spinner tooltip exposes the active status. |
| FR-15 | Restore terminal state when the TUI exits. | Quit/error smoke check confirms raw mode and alternate screen are restored. |
| FR-16 | Ship Linux packages and validate target desktop/TUI environments. | Future Ubuntu `.deb` and openSUSE `.rpm` builds plus distro smoke tests; not yet complete. |
| FR-17 | Switch the desktop browser between Icons, List, and Details layouts. | View menu switches between grid cards, full-width names, and metadata columns while selection, open, context actions, and drag/drop continue to work in each layout; each tab restores its own icon-card size when switched to or reopened. |
| FR-18 | Navigate and operate on visible items from the keyboard. | Arrow/Home/End selection keeps the focused item in view; Shift range selection, Enter open, Escape clears path editing/search/selection in order and returns focus from search to the file list; Ctrl+A operates on the current folder or search results without stealing text-field input. |
| FR-19 | Make the active sort key and direction visible where users change sorting. | Selecting a sort key updates the active header and Sort menu arrow; selecting the active key again reverses direction and preserves selection. |
| FR-20 | Keep the Details pane useful without an item selection. | With no item selected, the pane shows a 42-point folder glyph beside the current folder name, its path, and item/search-result count; it shows a searching state while results are pending and offers Copy Path. |
| FR-21 | Make context actions and shortcuts discoverable. | Item and empty-pane menus expose their documented actions; shortcut labels match the active platform and each shortcut performs the displayed action. Destination-specific Paste Into Folder has no misleading shortcut label. |
| FR-22 | Allow direct editing of the current address. | Clicking the current breadcrumb enters path-edit mode with the current path selected; clicking an ancestor navigates to that folder. |
| FR-23 | Make navigation rows easy to target. | Hover and selection feedback span the full row including its icon; clicking the icon, label, or remaining row area navigates; right-clicking any part opens the row menu; favorite reordering and file drops work across the row. |
| FR-24 | Keep TUI search instructions visible while entering a query. | Starting `/` replaces the general footer legend with width-adaptive search guidance; Enter keeps the query and leaves input, Esc clears it, and Ctrl+C quits. |
| FR-25 | Reflect file clipboard availability in Paste controls. | Paste buttons disable when no system or app-local file paths are available, update after clipboard changes while the GUI is open, and Ctrl/Command+V reports an empty clipboard without launching an operation. |
| FR-26 | Make the Details pane control self-explanatory. | View and status-row controls show/hide the pane; hover hints state whether the pane will be shown or hidden and display the platform shortcut. |
| FR-27 | Keep icon zoom with its tab. | Changing the icon-card size in one tab does not alter another tab; switching back and reopening a closed tab restores its previous size. New tabs inherit the current tab's size. |
| FR-28 | Use platform-appropriate file metadata labels. | Windows directory type is `File folder`; macOS and Linux directory type is `Folder`; modified times display in the local time zone across all three platforms. |
| FR-29 | Match folder icon colors to platform conventions while keeping their shape legible. | Folder glyphs use yellow on Windows/Linux and Finder blue on macOS in the file list/grid and navigation sidebar. Theme-aware outlines distinguish the folder shape on light/dark pane and selection backgrounds; cut-pending folders dim their fill and name while retaining the outline. |
| FR-30 | Make sortable column headings easy to target. | Clicking anywhere in a visible sortable header cell changes its sort order; hovering the cell shows subtle feedback, while the heading text stays left-aligned and the active direction remains visible. |
| FR-31 | Keep desktop commands reachable in a narrow window. | When command buttons wrap, the toolbar grows to fit every line without clipping or overlapping the address row; all commands remain operable. |
| FR-32 | Preserve operation status space beside a large selection. | The selected-item summary uses no more than 280 points and exposes its full text and folder-size exclusion note on hover; the status message and Details toggle remain available. |
| FR-33 | Make navigation sections easy to collapse and expand. | The disclosure glyph, section title, and blank area across the heading row toggle the section; Recent Locations keeps Clear as a separate action. |
| FR-34 | Keep TUI names and sizes readable at different terminal widths. | At widths of 52 columns or more, file sizes align in a right-side column; narrower layouts prioritize names. Long names truncate on grapheme boundaries using terminal display width and remain openable by the cursor. |
| FR-35 | Keep compacted TUI paths inside their panels. | Long paths retain their rightmost location context, truncate on grapheme boundaries, and fit the available terminal display width. |
| FR-36 | Keep cut queue state visible in the TUI. | Paths queued for cut appear dimmed whenever they are listed, and the footer displays their count while browsing elsewhere; both cues clear after a complete move or queue replacement. |
| FR-37 | Render filesystem text safely in the TUI. | Terminal control and bidi-formatting characters in names, paths, queries, and status messages are escaped for display without changing paths used by filesystem operations. |
| FR-38 | Make each tab easy to activate and close. | Clicking its icon, name, or blank margin activates the tab; inactive tabs show subtle hover feedback; the close button stays visible on Windows/Linux and appears when hovering a macOS tab; clicking it or middle-clicking closes the tab; dragging from anywhere on the tab reorders it; folder drops still target the tab. |
| FR-39 | Make the selected item recognizable in the Details pane. | A single selection shows a 42-point platform-colored file/folder glyph beside the name; long names truncate to pane width and expose the full name on hover. |
| FR-40 | Preserve usable file-list width while resizing side panes. | With the Navigation and Details panes open, their responsive maximum widths leave at least 320 logical points for the main pane; each splitter shows a horizontal-resize cursor and a pane-specific tooltip. Each pane remains resizable within its minimum and maximum widths. Automatically constrained widths do not replace the preferred widths, which return when the window has room. |
| FR-41 | Edit TUI search and prompts at an insertion point. | Left/Right move by grapheme; Home/End, Backspace/Delete, and Ctrl+A operate on the text at the cursor; the cursor remains visible when long inputs are compacted to the terminal width. |
| FR-42 | Explain why command-row actions are disabled. | Hovering disabled Cut, Copy, Rename, Delete/Move to Trash, or Paste controls explains the missing selection or clipboard requirement; enabled controls retain their action and shortcut hints. |

## 9. Design, accessibility, and resilience requirements

- All core tasks must be possible without a mouse in the TUI. GUI core actions have keyboard shortcuts and context/menu alternatives where implemented.
- Selection state must be visible through row highlighting or the TUI marker and cursor highlight. The focused GUI item has an additional outline so it remains distinguishable within a multi-selection.
- Interactive controls must retain visible focus/hover feedback without making ordinary file-name hover look like an active drag. During an active internal drag over a valid destination, the pointer indicates Move or Copy according to the platform modifier.
- Long filesystem work must not block the GUI frame loop or TUI event polling.
- Errors must identify the operation and retain useful OS error text. Conflict skip and refresh failures must not be hidden by success messages.
- Search must skip directory links found below its root to prevent recursive loops and unexpected traversal. A folder link explicitly opened by the user can serve as the search root.
- Trash deletion must remain recoverable through the platform trash facility when supported.
- The current implementation follows the system egui light/dark theme. Localization, custom themes, and screen-reader conformance are not yet specified as shipped capabilities.

## 10. Linux roadmap

Linux is the next platform milestone after Windows v1:

1. Produce an Ubuntu `.deb` and openSUSE `.rpm` from the Rust application.
2. Validate GUI startup, desktop file opening, trash behavior, drag/drop, and the navigation/search workflows on both distributions.
3. Validate TUI startup, keyboard navigation, resizing, file operations, and terminal restoration in a headless terminal on both distributions.
4. Implement and validate Linux file-manager clipboard interoperability for GUI copy/cut/paste. The current non-Windows clipboard adapter reports that file clipboard integration is unavailable; the TUI internal queue remains available.

Linux milestone acceptance requires running the package and smoke-test matrix on the target distributions, not just compiling the shared Rust source on Windows.

## 11. Implementation map

| Area | Source |
| --- | --- |
| Mode detection, CLI arguments, and mode tests | `src/main.rs` |
| Shared filesystem model, search, sorting, transfers, trash, and tests | `src/core.rs` |
| Desktop screen, interactions, dialogs, and state | `src/gui.rs` |
| TUI screen, keyboard loop, prompts, and terminal lifecycle | `src/tui.rs` |
| Cross-platform capability selection and local-time formatting | `src/platform.rs` |
| Windows GUI detection, local time, and CF_HDROP clipboard | `src/platform/windows.rs` |
