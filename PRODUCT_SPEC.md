# File Browser — Product Requirements Document

## 1. Document Control & Metadata

| Field | Value |
| --- | --- |
| Product | File Browser |
| Document | Product requirements and interaction specification |
| Document version | 0.3.0 |
| Product release baseline | 0.1.0 |
| Status | Draft — implementation and platform validation in review |
| Target release | Windows v1; Linux Ubuntu/openSUSE milestone follows |
| Author | AI PM Agent |
| Human reviewer | [Placeholder] |
| Source of truth | This document; implementation status is called out separately from planned requirements |

### Changelog

| Version | Date | Author | Change |
| --- | --- | --- | --- |
| 0.3.0 | 2026-10-10 | AI PM Agent | Add text and image previews, in-memory content indexing, UTF-8 editing, ZIP browsing, and editable file properties for GUI/TUI. |
| 0.2.2 | 2026-10-10 | AI PM Agent | Split combined functional groups into 12 feature specifications with independent workflows and UI state matrices. |
| 0.2.1 | 2026-10-10 | AI PM Agent | Tightened measurable goals, motion/timeout behavior, technical targets, persona literacy, and release KPIs after PRD audit. |
| 0.2.0 | 2026-10-10 | AI PM Agent | Reorganized as a complete PRD; added feature journeys, UI state matrices, non-functional targets, telemetry policy, and open questions. |
| 0.1.0 | 2026-10-10 | AI PM Agent | Initial product and interaction specification. |

## 2. Strategic Context & Vision

### Executive summary

File Browser is a local-first Rust application for managing files through either a Windows/Finder-inspired desktop interface or a keyboard-driven terminal interface. Both interfaces share filesystem behavior, allowing the same navigation, search, selection, and transfer model to work on graphical and headless systems without accounts or cloud services.

### Problem statement

Desktop users need a predictable way to browse, find, and move local files, while remote and headless users often have only a terminal. Maintaining separate GUI and terminal utilities creates different behavior and makes file operations harder to learn consistently. File Browser combines both interfaces around one Rust core and uses familiar Explorer and Finder patterns for navigation, selection, tabs, and file operations.

### Goals (in scope)

- Provide all 17 feature modules in section 4 in the interface(s) appropriate to the capability; each module has at least one Given/When/Then acceptance scenario.
- Keep filesystem work off interactive event loops; block user-input handling for no more than 100 ms during supported operations.
- Reach at least 95% successful workflow completion across the Windows release acceptance matrix, with 100% correct mode-selection and terminal-restoration cases.
- Start at Home and choose the GUI for 100% of graphical-session cases; otherwise choose the TUI for 100% of interactive-terminal cases.
- Match 100% of the documented Windows/macOS labels and shortcut behaviors in the acceptance catalog.
- Deliver Windows v1 after its acceptance gates pass; define Ubuntu `.deb` and openSUSE `.rpm` as the next Linux packaging targets.
- Keep 100% of app-authored application code in Rust and enforce file access through the current OS user’s permissions.

### Non-goals (out of scope)

- PDF, media, and document-format previews; the built-in preview covers UTF-8 text and selected raster image formats.
- Persistent or cross-session content indexes; indexing is explicit, scoped to one folder tree, and held in memory.
- Editing non-UTF-8, oversized, symlink, or non-regular files; editing text is limited to UTF-8 files up to 1 MiB.
- Archive formats other than ZIP, archive editing, and archive extraction from the archive browser.
- Property editing beyond the read-only flag and modified time.
- Accounts, cloud storage, synchronization, remote administration, or production analytics telemetry.
- Undo/redo, operation cancellation, persistent tabs, per-folder view settings, or user-defined Locations outside Favorites.
- Touch-first layouts, a built-in terminal, or a web interface.
- Linux packaging and file-manager clipboard interoperability in the Windows v1 milestone.

## 3. Target Audience & Personas

### Persona 1: Desktop file organizer

- **Role/context:** Windows or macOS user with intermediate computer literacy who manages personal or work folders with a mouse and keyboard; comfortable with standard file-manager conventions.
- **Motivation:** Find and organize files without changing their default applications or risking accidental replacement/deletion.
- **Pain points:** Deep folder navigation, repeated destinations, unclear transfer conflicts, and losing selection context after sorting or navigation.
- **Success:** Can locate an item, inspect its path, and complete a copy/move/rename/trash action with visible confirmation and recoverable deletion.

### Persona 2: Headless and remote operator

- **Role/context:** Technically proficient user connected over SSH or working in a console without a graphical session; comfortable with keyboard shortcuts and terminal navigation.
- **Motivation:** Perform routine file management using only the keyboard and without launching another app.
- **Pain points:** GUI-only tools are unavailable; shell commands make selection, conflict handling, and safe trash behavior cumbersome.
- **Success:** Can navigate, search names, select multiple entries, transfer files, resolve conflicts, and restore the terminal after exit.

### Persona 3: Keyboard-first user

- **Role/context:** User with beginner-to-advanced computer literacy who prefers or requires keyboard operation, including users who benefit from visible focus and predictable shortcuts.
- **Motivation:** Complete file tasks without relying on pointer precision.
- **Pain points:** Hidden shortcut behavior, focus loss, and selection state that is difficult to distinguish.
- **Success:** GUI core tasks are available from documented shortcuts and visible focus; all core TUI tasks are keyboard-accessible. Screen-reader conformance remains an open validation requirement, not a shipped claim.

## 4. Functional Specifications

The 17 feature modules below define user stories, end-to-end flows, Gherkin acceptance criteria, and observable UI states. Every matrix covers default, empty, loading/progress, validation error, external dependency failure, and timeout behavior. File Browser has no application server; server failure is not applicable. Network-mounted volumes and OS integrations use the external-failure state. The desktop GUI uses a spinner for pending work and has no view-transition animation; the TUI uses status text only. The detailed interaction rules, complete shortcut maps, and FR-01–FR-47 acceptance catalog are in Appendix A.

### 4.1 Launch and mode selection

**User story:** As a desktop or terminal user, I want the app to choose an interface that is usable in my current session so I can start managing files without configuring a mode first.

**Workflow:** Launch with no arguments; detect GUI and terminal availability; choose GUI if available, otherwise TUI if stdin and stdout are interactive; fail with an actionable message if neither exists. `--gui` and `--tui` force a mode and report when its required environment is unavailable.

**Acceptance criteria:**

- Given a graphical session, when the user launches without an override, then the GUI opens at Home.
- Given no graphical session and an interactive terminal, when the user launches without an override, then the TUI opens at Home.
- Given neither environment, when the user launches, then the program exits with a message naming the required session types.
- Given both overrides, when the user launches, then the program reports that only one mode may be selected.

| UI state | Required behavior |
| --- | --- |
| Default | Open Home in the selected interface. |
| Empty | An empty Home directory shows the documented empty-folder state. |
| Loading/progress | GUI and TUI show listing progress without blocking the event loop. |
| Validation error | Invalid or conflicting CLI arguments print usage/error text and exit without opening a partial interface. |
| OS/session failure | Forced GUI without a graphical session or forced TUI without interactive stdin/stdout exits with the corresponding actionable error. |
| Timeout | Session detection is synchronous; v0.1 has no startup timeout or automatic retry. |

### 4.2 Navigation and location access

**User story:** As a desktop user, I want to move between folders using familiar history, breadcrumbs, and pinned locations; as a terminal user, I want parent/path navigation without a pointer.

**Workflow:** Start at Home; browse through the sidebar, address path, history, or file list; return to parents/history; pin frequently used folders. In the TUI, use Enter/Right, Backspace/Left, and `g` for a path.

**Acceptance criteria:**

- Given a visible folder, when the user opens it, then the active location changes and the current view returns to that folder.
- Given a path in the address editor, when the user submits a valid path, then the app navigates there; when it does not exist or is inaccessible, then it reports the OS error and offers retry where specified.
- Given a folder opened from search, when the user chooses Open Containing Folder, then its parent opens and the result is selected when still present.
- Given a user navigates to a parent folder, when the previous child remains visible, then GUI Up selects and focuses that child.

| UI state | Required behavior |
| --- | --- |
| Default | Show current path, navigation history, and available Home/Recent/Favorites/Locations entries. |
| Empty | Empty folders retain navigation controls and show a create-folder action. |
| Loading/progress | Show a loading state while the requested folder is listed; keep other controls responsive. |
| Validation error | Reject malformed/unavailable paths with the OS error; do not silently switch to a different folder. |
| Network/OS failure | A disconnected mount or inaccessible folder remains identifiable; report the error and keep the current path available for retry. |
| Timeout | No application-level deadline is defined; keep progress visible and the interface responsive until the OS call returns; do not retry automatically. |

### 4.3 Tabs and sidebar locations

**User story:** As a desktop user, I want to keep several locations open and return to frequent or recent folders without repeating navigation.

**Workflow:** Open a tab at the current or selected folder; activate, reorder, close, duplicate, or reopen tabs; navigate from Home, Recent Locations, Favorites, or mounted Locations; pin, reorder, or remove Favorites.

**Acceptance criteria:**

- Given two open tabs, when the user switches tabs, then that tab restores its query, view mode, sort, scroll, selection, and focus when the items remain available.
- Given a missing recent or favorite path, when the sidebar renders, then the path remains listed as Unavailable so it can be removed.
- Given the only tab is closed, when close completes, then that tab returns to Home; closed tabs can be reopened up to the ten-tab history limit.

| UI state | Required behavior |
| --- | --- |
| Default | Show the active tab and each sidebar section’s stored expanded/collapsed state. |
| Empty | Hide the Recent Locations section when it has no entries; keep Home, Favorites, and Locations available. |
| Loading/progress | Show a tab’s loading state while its folder is restored; do not block switching to another tab. |
| Validation error | Reject attempts to pin or reorder a path that is not an existing folder; keep the prior Favorites order. |
| Network/OS failure | Keep unavailable locations labeled and removable; report failures to persist Favorites or recent locations. |
| Timeout | No application-level deadline is defined; keep progress visible and the interface responsive until the OS call returns; do not retry automatically. |

### 4.4 Browse, view modes, and sorting

**User story:** As a desktop or terminal user, I want to scan folder contents and sort them in a layout that fits the task.

**Workflow:** Browse Details/List/Icons in the GUI or the terminal list; choose a sort key and direction; open visible file/folder entries.

**Acceptance criteria:**

- Given a populated folder, when the user changes a sort key, then directories remain grouped before files and the active key/direction remains visible.
- Given a narrow window or terminal, when the listing adapts, then names remain readable and metadata is progressively hidden before the name column.
- Given an empty folder or search, when the listing is rendered, then the screen distinguishes the empty folder from no matching results.

| UI state | Required behavior |
| --- | --- |
| Default | Show names and available metadata; indicate the active sort key and direction. |
| Empty | Show “This folder is empty” with New folder, or “No results found” with Clear search. |
| Loading/progress | Show folder listing or search progress without blocking input. |
| Validation error | If an entry disappears during refresh, omit it and keep remaining entries sorted. |
| Network/OS failure | Show the unreadable-folder or entry-metadata error with Retry; do not report a partial listing as complete. |
| Timeout | No application-level deadline is defined; keep progress visible and the interface responsive until the OS call returns; do not retry automatically. |

### 4.5 Selection and keyboard commands

**User story:** As a desktop or terminal user, I want to select one or more items and operate on the intended selection with mouse or keyboard.

**Workflow:** Select a row/card or move focus with the keyboard; extend/toggle selection; invoke an action; clear selection or return focus using the documented Escape behavior.

**Acceptance criteria:**

- Given a Details/List row, when the user clicks anywhere through the visible pane edge, then it selects; a press without pointer movement remains a click, while click-and-move starts a drag.
- Given a selected row, when the user Ctrl/Command-clicks it, then its selection toggles and it becomes the range anchor; Shift-click extends from that anchor.
- Given a multi-selection, when an action is invoked, then it operates on the selected set and the focused item remains visually distinct.
- Given the TUI has no explicit selection, when Copy/Cut/Rename/Delete is invoked, then the current cursor item is used when one exists.

| UI state | Required behavior |
| --- | --- |
| Default | Show selection and focus independently using row/card highlighting, focus outline, or TUI marker/reverse highlighting. |
| Empty | Disable selection-dependent actions; TUI actions without a selection use the current cursor item when possible. |
| Loading/progress | Keep the current selection visible while listing/search work runs; retain items that remain visible after completion. |
| Validation error | Reject Rename unless exactly one item is selected; explain required selection on disabled command controls. |
| Network/OS failure | If an item disappears or becomes inaccessible, retain valid selections and report the failed operation. |
| Timeout | Selection changes are synchronous and have no timeout; filesystem actions use the timeout behavior in their operation module. |

### 4.6 Recursive name search

**User story:** As a user, I want to find descendants by name without opening each folder or scanning file contents.

**Workflow:** Focus search; enter a case-insensitive name fragment; view results in the active layout; open a result or its parent; clear the query to return to the current folder.

**Acceptance criteria:**

- Given a query, when it changes, then results update after debounce and stale scans are discarded.
- Given a nested matching item, when search completes, then the item appears with its parent location where the layout permits.
- Given a directory symlink below the search root, when search traverses the tree, then it does not descend through that link.

| UI state | Required behavior |
| --- | --- |
| Default | Show the current folder until the user starts a query. |
| Empty | Show “No results found,” the active query, and Clear search. |
| Loading/progress | Show “Searching…” and keep input responsive; replace stale results only with the latest completed query. |
| Validation error | Empty query returns to the folder listing; invalid path/root errors are reported without changing the query silently. |
| Network/OS failure | Skip inaccessible descendants; report failure to open the search root with retry. |
| Timeout | No application-level deadline is defined; keep progress visible and the interface responsive until the OS call returns; do not retry automatically. |

### 4.7 Create and rename

**User story:** As a user, I want to create folders and rename items without accidentally replacing an existing destination.

**Workflow:** Choose New Folder or Rename; enter one path-component name; submit; correct validation errors; after success, return to the listing with the created/renamed item selected when visible.

**Acceptance criteria:**

- Given an empty or multi-component name, when submitted, then the action is rejected and the prompt remains available for correction.
- Given an existing destination name, when create or rename is attempted, then the existing item remains unchanged and the error is shown.
- Given a valid name, when the filesystem operation succeeds, then the new or renamed item is selected if it remains visible.

| UI state | Required behavior |
| --- | --- |
| Default | Focus the name field; on rename, select the basename while preserving a recognized extension. |
| Empty | New Folder remains available in an empty folder; Rename is disabled until exactly one item is selected. |
| Loading/progress | Run mutation off the UI loop and show “Creating folder…” or “Renaming…”. |
| Validation error | Reject empty, multi-component, invalid, or existing destination names; preserve the source item. |
| Network/OS failure | Show the operation and OS error; keep the original item when rename fails. |
| Timeout | No operation deadline is configured; keep the prompt result pending and the interface responsive until the OS call returns. |

### 4.8 Move to trash

**User story:** As a user, I want to remove selected items recoverably and confirm exactly what will be moved.

**Workflow:** Select one or more items; invoke Delete/Move to Trash; review item names and count; confirm or cancel; inspect the completion or error status.

**Acceptance criteria:**

- Given a trash request, when the confirmation is shown, then it lists the affected items and the platform trash action.
- Given a confirmation, when the user accepts, then the OS trash adapter is invoked; when canceled, then no item is moved.
- Given trash integration failure, when the OS returns an error, then the UI reports it and does not claim success.

| UI state | Required behavior |
| --- | --- |
| Default | Confirmation names “Move to Recycle Bin?” on Windows and “Move to Trash?” on macOS/Linux and lists affected items. |
| Empty | Disable Delete/Move to Trash when no selection or TUI cursor item exists. |
| Loading/progress | Show “Moving to <trash name>…” while the OS integration runs. |
| Validation error | Cancel without change when no valid source paths remain. |
| Network/OS failure | Keep the source item visible when trash fails and show the OS error. |
| Timeout | No operation deadline is configured; keep progress visible and allow the OS call to finish; v0.1 has no cancellation action. |

### 4.9 Copy, move, clipboard, and conflicts

**User story:** As a user, I want to copy or move items using the platform clipboard or the TUI queue and resolve destination conflicts deliberately.

**Workflow:** Select items; Copy/Cut; navigate to a destination; Paste or Paste Into Folder; choose Replace, Skip, or Cancel on conflict; inspect completion, skipped names, or error.

**Acceptance criteria:**

- Given Windows GUI file paths in the system clipboard, when Paste is invoked, then those paths are used; otherwise the app-local queue is the fallback.
- Given the TUI, when Copy/Cut/Paste is invoked, then it uses the app-local queue without requiring a GUI clipboard.
- Given a transfer conflict, when the user selects Skip, then non-conflicting items transfer and skipped names are reported; Cancel stops transfer after preflight.

| UI state | Required behavior |
| --- | --- |
| Default | Paste controls enable only when file paths are available; cut-pending items remain visibly dimmed until a complete move. |
| Empty | Disable Paste and explain “No file paths in the clipboard.” |
| Loading/progress | Show source name/count and destination; keep UI event processing active. |
| Validation error | Show Replace/Skip/Cancel; reject self/descendant moves and retain prior destinations unless Replace is selected. |
| Network/OS failure | Report the failing operation and OS error; preserve retry queue after failed or partial moves; disclose that multi-item operations are not atomic. |
| Timeout | No transfer deadline is configured; keep progress and interface responsive until the OS call returns; do not retry automatically. |

### 4.10 GUI drag-and-drop

**User story:** As a desktop user, I want to transfer files by dragging only after I press and move, with clear feedback about the destination and action.

**Workflow:** Press a selected or unselected row/card; move the pointer to a folder row, icon card, sidebar location, tab, or empty pane; inspect the destination outline and Move/Copy cursor; release to transfer.

**Acceptance criteria:**

- Given a simple click on an item, when the pointer does not move, then the item is selected and no drag begins.
- Given a same-volume internal drop, when released on a folder, then the default action is Move; cross-volume drops default to Copy; Ctrl/Option forces Copy.
- Given an external file drop, when released over a directory target, then the files are copied there; an invalid file-row target is rejected with no transfer.

| UI state | Required behavior |
| --- | --- |
| Default | Keep the standard pointer over items; no drag cursor appears on hover alone. |
| Empty | The empty file pane accepts drops into the current folder and shows a destination outline. |
| Loading/progress | After release, show transfer source/count and destination in the status row. |
| Validation error | Reject file-row, self-folder, and descendant-folder destinations; show Not Allowed and an explanatory status. |
| Network/OS failure | Report transfer errors and retain cut queue state when the move is incomplete. |
| Timeout | No drag-transfer deadline is configured; after release, retain progress until the OS transfer returns. |

### 4.11 Open files and use the Details pane

**User story:** As a desktop user, I want to inspect an item’s metadata and open it with its usual application; as a terminal user, I want the app to report when no opener is available.

**Workflow:** Select an item; inspect name/type/location/size/modified time; copy path or open; for folders, navigate or open in a new tab.

**Acceptance criteria:**

- Given one selected item, when the Details pane is open, then it shows the item glyph, metadata, containing location, and relevant actions.
- Given no selection, when the Details pane is open, then it shows the current folder and its item/search-result count.
- Given a file without an available system application, when Open is invoked, then the UI reports the launch error and remains usable.

| UI state | Required behavior |
| --- | --- |
| Default | Show selected-item metadata or the current-folder summary. |
| Empty | With no selection, show the current folder rather than an empty pane. |
| Loading/progress | Show search-pending state and count while results are being produced. |
| Validation error | If the item is no longer available, keep the pane consistent with the remaining selection and show the error. |
| Network/OS failure | Report default-app launch or path access errors; do not preview or modify file contents. |
| Timeout | No application-level deadline is defined; keep the interface responsive while the OS opener returns. |

### 4.12 Headless TUI interaction

**User story:** As a remote or headless user, I want to complete file-management tasks with keyboard-only controls and leave my terminal restored after exit.

**Workflow:** Navigate with arrows or `j`/`k`; search with `/`; select with Space; copy/cut/paste with `c`/`x`/`p`; create/rename with `n`/F2; trash with `d`; resolve conflicts; quit with `q` or Ctrl+C.

**Acceptance criteria:**

- Given a terminal below 52 columns, when the list renders, then names remain the priority and file sizes are hidden.
- Given a long Unicode name or path, when rendered, then truncation occurs on grapheme boundaries using terminal display width.
- Given normal exit or an error after terminal entry, when the TUI ends, then raw mode and the alternate screen are restored.

| UI state | Required behavior |
| --- | --- |
| Default | Show path, file rows, selection/focus, status, and width-adaptive key help. |
| Empty | Show distinct empty-folder and no-search-match messages. |
| Loading/progress | Show listing/search/operation progress and keep keyboard polling responsive. |
| Validation error | Keep prompts open for correctable invalid names; conflict prompts offer Replace/Skip/Cancel. |
| Network/OS failure | Show concise operation errors and preserve terminal state; opening a file without an app reports that limitation. |
| Timeout | No application-level deadline is defined; keep progress visible and the interface responsive until the OS call returns; do not retry automatically. |

### 4.13 File-content preview

**User story:** As a desktop or headless user, I want to inspect supported file contents without launching another application so I can identify a file before acting on it.

**Workflow:** Select a regular file; choose Preview Text or press `v` in the TUI; inspect UTF-8 text up to 1 MiB. In the GUI, supported BMP, GIF, JPEG, PNG, and WebP images are decoded and scaled to fit. ZIP members can be previewed as UTF-8 text up to 1 MiB from the archive browser.

**Acceptance criteria:**

- Given a readable UTF-8 regular file no larger than 1 MiB, when Preview Text is chosen, then the contents appear in a read-only viewer without launching the system opener.
- Given a supported raster image no larger than 64 MiB encoded and no larger than 8192 × 8192 pixels, when previewed in the GUI, then it is decoded off the UI thread and scaled within the viewer while preserving aspect ratio.
- Given an unsupported, binary, unreadable, or oversized file, when preview is requested, then an explanatory error appears and the file remains unchanged.
- Given a file selected in the TUI, when `v` is pressed, then UTF-8 text is shown with keyboard scrolling; unsupported image display is reported as unavailable in the terminal.

| UI state | Required behavior |
| --- | --- |
| Default | Viewer shows the selected file path and supported text or image content; the original file is not modified. |
| Empty | An empty text file shows an empty viewer with its path and Close action. |
| Loading/progress | Show Loading/Reading status while file bytes are read and decoded off the UI thread. |
| Validation error | Reject directories, symbolic links, invalid UTF-8 text, unsupported image types, or files beyond the stated limits with a specific message. |
| Network/OS failure | Show the OS read/decode error and keep the file listing available. |
| Timeout | No application timeout is defined; the viewer stays responsive until the OS/decode worker returns and does not retry automatically. |

### 4.14 Content indexing and search

**User story:** As a user, I want to index the text in a folder tree and search it so I can find files by words in their contents.

**Workflow:** Choose Index this folder’s text files in the GUI or press `i` in the TUI; wait for indexing; enable content search (`Search indexed contents` in the GUI or `I` in the TUI); enter a query. Reindex after external changes, changing hidden-item visibility, or switching to another folder tree.

**Acceptance criteria:**

- Given a regular folder, when indexing starts, then it traverses descendant directories without following directory symlinks and skips hidden entries unless hidden visibility is enabled.
- Given a tree larger than the per-build directory cap, when indexing completes, then at most 100,000 directories are visited and the summary reports that the directory limit was reached.
- Given readable UTF-8 files up to 1 MiB each, when indexing completes, then they are available for case-insensitive substring search in the active folder tree.
- Given files larger than 1 MiB, unreadable files, binary/non-UTF-8 files, or resource caps reached, when indexing completes, then those files are skipped and the summary reports indexed and skipped counts.
- Given content search is enabled without an index for the current folder, when a query is entered, then the app instructs the user to index that folder first.
- Given application-managed content changes, when a text file is saved or an operation mutates the listing, then the prior index is discarded and must be rebuilt before content search.
- Given hidden-item visibility changes after indexing, when content search is requested, then the prior index is discarded and the user must rebuild it using the new visibility setting.

| UI state | Required behavior |
| --- | --- |
| Default | Name search remains the default; content search uses only the current in-memory index. |
| Empty | A query with no content matches shows No results found and Clear search. |
| Loading/progress | Show Indexing text files or Searching status; work runs in a background thread and keeps input responsive. |
| Validation error | Reject a non-directory or symbolic-link indexing root; content search without a current-root index gives an Index this folder prompt. |
| Network/OS failure | Skip inaccessible descendants/files, report indexed/skipped counts, and report a root-level failure. |
| Timeout | No deadline is defined; keep progress visible until the worker returns and do not retry automatically. |

Resource limits: maximum 100,000 indexed files and 128 MiB of indexed text per build; each file is limited to 1 MiB. The index is process-local, is not written to disk, and is not sent over a network.

### 4.15 UTF-8 text editing

**User story:** As a user, I want to edit a supported text file in File Browser and save it without silently overwriting external changes.

**Workflow:** Choose Edit Text from the GUI context menu/details pane or press `e` in the TUI; edit the UTF-8 contents; choose Save or press Ctrl+S; explicitly discard or close when there are unsaved changes.

**Acceptance criteria:**

- Given a regular non-symlink UTF-8 file no larger than 1 MiB, when Edit Text is chosen, then its contents appear in an editable buffer and the source remains unchanged until Save.
- Given unchanged source revision metadata, when Save is requested, then the app stages and replaces the file while preserving its permissions.
- Given source size or modified time changed after the editor loaded, when Save is requested, then saving is rejected with a stale-file message and Reload from disk is offered.
- Given an edit buffer has unsaved changes, when the user closes the editor, then the app requires an explicit Discard choice or save.

| UI state | Required behavior |
| --- | --- |
| Default | Show path, editable buffer, and Save/Close controls; TUI shows a caret marker and key help. |
| Empty | Empty files open as editable empty buffers. |
| Loading/progress | Load and save on a worker thread; disable editing while a save is in progress and show status. |
| Validation error | Reject non-regular files, symlinks, invalid UTF-8, files over 1 MiB, and stale revisions; keep unsaved text visible. |
| Network/OS failure | Keep the edit buffer and show the OS write error; do not claim the file was saved. |
| Timeout | No application deadline is defined; keep the editor responsive until the read/write worker returns and do not retry automatically. |

### 4.16 ZIP archive browsing

**User story:** As a user, I want to browse files and folders inside ZIP archives without extracting them first.

**Workflow:** Choose Browse ZIP archive from a `.zip` file’s GUI context menu or press `a` on a selected `.zip` in the TUI; navigate direct children with Open/Enter; preview UTF-8 text members; use Backspace/Up to navigate to a parent and Esc to close.

**Acceptance criteria:**

- Given a readable ZIP archive with at most 100,000 entries, when opened, then the app lists direct children and their file sizes without extracting data.
- Given a nested archive folder, when opened, then only its direct child folders and files are shown; parent navigation returns to the preceding archive folder.
- Given an archive member path that is absolute or escapes the archive root, when listing or previewing, then it is excluded or rejected.
- Given a text member up to 1 MiB, when Preview is chosen, then UTF-8 content appears read-only; unsupported/binary/oversized members show an error.
- Given a corrupt, unsupported, or encrypted archive, when opened or read, then the error is shown without modifying the archive or filesystem.

| UI state | Required behavior |
| --- | --- |
| Default | Show archive path, current internal folder, direct children, and parent/close controls. |
| Empty | Show “This archive folder is empty.” |
| Loading/progress | Read the archive directory or preview on a worker thread and show progress. |
| Validation error | Reject invalid member paths and archives with more than 100,000 entries; archive operations remain read-only. |
| Network/OS failure | Report open/read errors and offer Retry where the archive remains open. |
| Timeout | No application deadline is defined; keep the interface responsive until the worker returns and do not retry automatically. |

ZIP support is read-only: archive editing, extraction, and formats other than ZIP are out of scope.

### 4.17 File-property editing

**User story:** As a user, I want to change a file or folder’s read-only state and modified time from the Details pane, item menu, or TUI.

**Workflow:** Select one regular file/folder; open Edit Properties or press `P` in the TUI; change Read-only and the local modified timestamp; save; review the operation result.

**Acceptance criteria:**

- Given one regular non-symlink file or directory, when Edit Properties opens, then current read-only and modified-time values are loaded.
- Given a valid local timestamp in `YYYY-MM-DD HH:MM:SS` format, when Save is chosen, then the modified time and read-only state are applied using the current OS user’s permissions.
- Given invalid/ambiguous local time or a symbolic link, when Save is requested, then the change is rejected with a corrective error and the prior property values remain available.
- Given the OS denies a property update, when Save fails, then the OS error is shown and no success is reported.

| UI state | Required behavior |
| --- | --- |
| Default | Show the selected path, Read-only checkbox, local modified-time field, Save, and Close. |
| Empty | Not applicable; the dialog requires one existing selected item. |
| Loading/progress | Load and save properties on a worker thread; disable editing while either operation is pending. |
| Validation error | Keep the dialog open and mark the timestamp format or unsupported symbolic-link condition. |
| Network/OS failure | Show the OS error and retain entered values for correction/retry. |
| Timeout | No application deadline is defined; keep the interface responsive until the OS worker returns and do not retry automatically. |

## 5. Non-Functional & Technical Specifications

### 5.1 Platforms and display support

| Target | Release state | Display contract |
| --- | --- | --- |
| Windows desktop | v1 target; GUI and TUI code are implemented. | GUI starts at 1180 × 760 logical points and has an 800 × 560 minimum inner size. |
| macOS desktop | GUI/TUI code and Finder-specific labels/shortcuts are implemented; release-level OS/version validation is not recorded. | Same desktop window dimensions; Finder-blue folder glyphs and Command-key conventions. |
| Linux desktop | Roadmap milestone for Ubuntu `.deb` and openSUSE `.rpm`; distribution validation remains outstanding. | eframe X11/Wayland support; window dimensions match desktop baseline. |
| Headless terminal | TUI target on Windows, macOS, and Linux when stdin/stdout are interactive. | Layout adapts at widths below 52, 40, and 80 columns; narrow terminals prioritize names and keep prompts within the footer. |

Minimum supported OS versions are not declared in the manifest and remain an open release decision. Touch input is not a supported target; desktop pointer targets are sized in logical points.

This is a native desktop/terminal application; a browser compatibility matrix and mobile battery budget are not applicable.

### 5.2 Performance and responsiveness

**Current implementation requirements:** directory listing, recursive search, and mutations execute on worker threads; GUI rows/cards are virtualized to visible content; the TUI polls input while work runs. Multi-item operations are sequential and are not batch-atomic.

**Provisional release targets — not yet benchmarked:** on a reference Windows 11 PC with 8 logical CPU cores, 16 GB RAM, and NVMe storage:

- P95 first usable GUI frame within 2.0 seconds across 10 cold starts.
- P95 folder listing of up to 10,000 entries visible within 1.0 second after navigation, measured across 30 runs on a local volume.
- P95 recursive name search over up to 100,000 entries settles within 3.0 seconds, measured across 30 runs on a local volume.
- P95 content-index build over 100,000 local files and up to 128 MiB of eligible UTF-8 content completes within 15 seconds, measured across 30 runs; the index itself stores no more than 128 MiB of normalized text.
- No user-input event is blocked by filesystem work for more than 100 ms.
- Process working set remains at or below 512 MiB during the 100,000-entry search scenario.

The working-set target is a provisional resource budget. Battery consumption is not budgeted because mobile devices are out of scope. These targets must be benchmarked on representative local and mounted volumes before they are accepted as release SLAs. They are goals, not verified claims about v0.1.0.

### 5.3 Security and data privacy

- The app has no sign-in, accounts, network service, synchronization, or production telemetry.
- OAuth, passkeys, MFA, sessions, and role-based access control do not apply; the local OS user and filesystem ACLs are the authorization boundary.
- Filesystem access runs with the launching user’s OS identity and permissions; the app does not elevate privileges.
- Content indexing is user-initiated and scans only the selected local folder tree; the in-memory index is discarded when replaced or when the process exits. The app does not upload file contents.
- Text preview/edit reads UTF-8 regular files up to 1 MiB; the GUI image preview supports BMP, GIF, JPEG, PNG, and WebP with encoded-size and decoder dimension/allocation limits.
- ZIP browsing is read-only, validates member paths, and does not extract archive contents. Other archive formats are not supported.
- File opening delegates a path to the OS default application. File Browser does not inspect the opened application’s behavior.
- GUI file clipboard integration uses file paths; Copy Path writes plain-text paths. The TUI queue is process-local and stores paths only in memory.
- Favorites, recent locations, and GUI preferences are stored locally in the user’s configuration area. The exact retention and reset UI are not currently specified.
- Delete actions use the OS trash adapter and require confirmation; permanent deletion is not a user-facing operation.
- No GDPR, CCPA, SOC 2, or other compliance certification is claimed. Legal applicability depends on distribution and jurisdiction and remains subject to review before a commercial release.

### 5.4 Offline and edge connectivity

Core browsing and operations are local and do not require an internet connection. Mounted network volumes are accessed through normal OS paths; disconnects, permission changes, and unavailable mounts surface as OS errors. There is no offline synchronization, conflict reconciliation, retry scheduler, or cache invalidation service. The user retries after connectivity returns.

### 5.5 Accessibility and resilience

- TUI core workflows are keyboard-only. The GUI documents shortcuts for core file operations, navigation, selection, and pane controls; focus and selection have visible indicators.
- The desktop GUI uses egui’s system light/dark palette. Text contrast ratios and screen-reader semantics have not been measured or validated; the current build does not claim WCAG conformance.
- Release target: map applicable WCAG 2.2 AA criteria to desktop controls, retain visible keyboard focus, provide accessible names/state for custom-painted controls, and validate with Windows Narrator and macOS VoiceOver. Target pointer controls at 24 logical points or larger; touch targets are out of scope.
- Terminal text sanitization escapes control and bidi-formatting characters for display without changing paths used for filesystem operations.
- Errors retain the operation context and useful OS text. Partial transfer completion does not imply rollback; completed earlier items remain changed and skipped names are reported.

### 5.6 Technical architecture

- Application language: Rust, edition 2024; declared minimum Rust version 1.89.
- Desktop UI: eframe/egui; terminal UI: Ratatui/Crossterm; both call the shared `src/core.rs` filesystem layer and `src/content_index.rs` in-memory text index.
- Platform adapters isolate GUI detection, file clipboard, local-time formatting, mounted-location discovery, file opening, and trash behavior.
- Platform-specific capabilities are conditional Rust modules; app-authored product code remains Rust.
- Linux package production and Linux file-manager clipboard integration are roadmap items, not current release capabilities.

## 6. Analytics, Telemetry & Success Metrics

### North Star and release KPIs

The product is local-first and emits no production analytics. The following are release-validation metrics collected by scripted/manual QA, not transmitted from user devices:

**North Star metric:** successful core file-task completion rate, calculated as completed scripted workflows without a product defect divided by total workflow attempts. The release target is at least 95% across the Windows acceptance matrix.

| Metric | Definition | Release target |
| --- | --- | --- |
| Core task completion | Successful completion of browse/open, search, create/rename, copy/move, conflict resolution, and trash workflows in the Windows acceptance matrix. | At least 95% of scripted runs complete without a product defect. |
| Destructive-operation safety | Confirmed delete and replacement behavior. | 0 items trashed without confirmation; 0 conflicting destinations replaced without an explicit Replace choice. |
| Session and terminal reliability | Correct GUI/TUI mode selection plus raw-mode/alternate-screen restoration after TUI exit. | 100% of supported mode-selection and terminal-exit cases pass. |
| Responsiveness | Results against provisional section 5.2 targets. | All five provisional targets measured and dispositioned before release approval. |

### Event tracking plan

| Event name | Trigger / screen | Parameters / properties | Purpose |
| --- | --- | --- | --- |
| None in v0.1 | No production event is emitted. | None; no user identifier or file-path telemetry is collected. | Preserve offline behavior and local-file privacy. |

Future telemetry requires an explicit product decision, opt-in design, data minimization, retention period, and privacy review before implementation.

## 7. Dependencies, Assumptions & Open Questions

### Third-party and OS dependencies

| Dependency | Use | Boundary |
| --- | --- | --- |
| eframe/egui | Desktop window and GUI widgets. | Rust dependency; uses platform windowing/rendering backends. |
| Ratatui/Crossterm | Terminal layout, rendering, and keyboard input. | Requires an interactive terminal for TUI mode. |
| `trash` | Move items to the platform trash where supported. | OS/distro behavior must be validated on each release target. |
| `opener` | Launch files with the system default application. | Failure is surfaced; the app does not bundle file viewers. |
| `image` | Decode BMP, GIF, JPEG, PNG, and WebP previews in the desktop GUI. | Decoding is bounded; terminal image rendering is not supported. |
| `zip` | List ZIP members and preview bounded UTF-8 text without extraction. | Read-only browsing; unsafe member paths are rejected. |
| `dirs` and OS APIs | Home/config paths, Windows CF_HDROP clipboard and platform capability discovery. | Preferences and paths remain under the current user context. |
| Filesystem and mounted volumes | Listing, metadata, copy/move, symlinks, and search. | Permissions and failure semantics are inherited from the OS/filesystem. |

### Assumptions and constraints

- Windows is the first release target; Linux packages and file-manager clipboard support follow in a separate milestone.
- macOS Finder conventions are implemented, but the supported macOS version and release validation status need confirmation.
- Rust-only means Rust-authored application logic; using Rust dependencies and OS APIs is allowed.
- The app is a local file manager with a bounded UTF-8 text editor and process-local content index; it is not a synchronization client or server.
- Content indexes are volatile and limited to one active folder tree; users rebuild after application-managed changes or external edits.
- Property editing is limited to read-only state and modified time; archive browsing is limited to ZIP and is read-only.
- Multi-item copy/move is sequential and can partially complete on OS error; no operation history or rollback is provided.
- Preferences and recent/favorite paths may reveal folder names to anyone with access to the user profile; OS profile permissions are the storage protection.

### Open questions

| ID | Decision needed | Why it matters | Proposed owner / timing |
| --- | --- | --- | --- |
| OQ-01 | Declare minimum Windows and macOS versions, and whether macOS is a supported release target or a convention-only build target. | Determines packaging, validation devices, and platform support claims. | Product + release engineering before v1 approval. |
| OQ-02 | Confirm the reference hardware and storage mix for performance SLAs. | Section 5.2 targets cannot be release gates until the benchmark baseline is agreed. | Product + engineering before performance sign-off. |
| OQ-03 | Confirm screen-reader acceptance scope and test owners for Narrator/VoiceOver. | AccessKit/native accessibility semantics are not currently validated. | Product + accessibility reviewer before claiming conformance. |
| OQ-04 | Confirm whether local preference data needs an in-app reset/clear control and a documented retention policy. | Favorites and recent paths persist in the user profile. | Product before wider distribution. |
| OQ-05 | Confirm fallback behavior when the OS trash facility is unavailable. | Recoverable deletion is a core safety promise; platform support differs. | Engineering + product before each platform release. |
| OQ-06 | Approve Linux distribution baselines and clipboard interoperability requirements. | Ubuntu/openSUSE versions, packaging formats, and file-manager protocols affect the Linux milestone. | Product + release engineering before Linux milestone kickoff. |

## Appendix A: Detailed Interaction and Acceptance Specification

The following material captures the implemented product behavior and the complete interaction rules used by the PRD above. A capability marked as roadmap work is not a shipped release claim.

### A.1 Platform and launch behavior

| Platform/context | Auto launch | GUI | TUI | Notes |
| --- | --- | --- | --- | --- |
| Windows desktop | GUI | Yes | Yes, when an interactive terminal is attached | GUI uses Windows file clipboard formats and system file opening/trash integration. |
| macOS desktop | GUI unless running over SSH without display forwarding | Yes | Yes, when an interactive terminal is attached | The GUI uses the native desktop session even when `DISPLAY` and `WAYLAND_DISPLAY` are unset and follows Finder labels and shortcuts. File-manager file clipboard interoperability is unavailable; the app-local queue supports transfers within File Browser. |
| macOS headless SSH terminal | TUI when stdin/stdout are interactive | No display session | Yes | Set `DISPLAY` for a forwarded graphical session or pass `--tui` explicitly. |
| Linux graphical session | GUI when `DISPLAY` or `WAYLAND_DISPLAY` is set | Built from eframe with X11/Wayland support | Yes | Linux packaging and file-manager clipboard integration are roadmap items. |
| Linux headless interactive terminal | TUI | No graphical session | Yes | TUI operations use the shared core and app-local clipboard queue. |
| No GUI and no interactive stdin/stdout | Error | No | No | The program reports that a graphical session or interactive terminal is required. |

`--gui` forces GUI mode and fails clearly if no graphical session is detected. `--tui` forces TUI mode and requires interactive stdin and stdout. Passing both modes is an error. `--help` prints usage.

### A.2 Shared interaction and filesystem behavior

#### 5.1 Navigation and starting location

- Start at the user home directory.
- A directory entry opens through GUI double-click/context action or TUI Enter/Right.
- Parent navigation uses GUI Up/breadcrumbs/history and TUI Backspace/Left. GUI Up (button or shortcut) selects and focuses the child folder just left in the parent listing. Breadcrumb navigation opens the chosen ancestor without forcing an item selection.
- The GUI keeps an in-memory history for each tab. Back and Forward navigate that tab’s history and restore the prior scroll position, visible selection, and focus when those items still exist; opening a new path after going Back discards the forward branch.
- Folder context menus and sidebar location context menus can open a folder in a new tab.
- Open Containing Folder from a search result or the Details pane navigates to the parent and selects/focuses that item in the listing.
- The TUI has one current path and no tab/history stack.

#### 5.2 Entries and metadata

The shared entry model carries path, display name, directory flag, symlink flag, size, and optional modified time.

- Directory symlinks are identified as symlinks, displayed with a folder link glyph, and can be navigated like folders. Search can start from an explicitly opened folder link but does not descend through linked directories found below it.
- A file’s displayed size is its metadata length. GUI folders show no size; TUI folders show no size.
- GUI type labels follow platform conventions: Windows directories use `File folder`, while macOS and Linux use `Folder`; directory symlinks use `Folder link`, file symlinks use `Symbolic link`, regular files use extension-based labels such as `TXT file`, or `File` when there is no extension.
- TUI uses compact `[D]`, `[L]`, and `[F]` markers.
- Modified time uses the local time zone on Windows, macOS, and Linux. The GUI displays month/day/year with a 12-hour clock and AM/PM.

#### 5.3 Hidden entries

Hidden entries are excluded by default. On Windows, hidden means the Windows Hidden attribute or a dot-prefixed name. On non-Windows platforms, a dot-prefixed name is hidden. The GUI exposes a `View > Hidden items` toggle; the TUI toggles with `h`.

#### 5.4 Sorting

Directories remain grouped before files. Sort keys are:

- **Name:** case-insensitive natural ordering; digit runs compare by numeric value (`file2` before `file10`).
- **Type:** lowercase file extension, then natural name order.
- **Size:** metadata size, then natural name order.
- **Date modified:** modified time, then natural name order.

GUI sorting is available from sortable column headings and the Sort menu. Re-selecting the active sort reverses direction; the Sort menu also exposes Descending. The TUI cycles Name → Type → Size → Date modified → Name with `s`, and `S` reverses direction. The active sort and direction appear in the file list title.

#### 5.5 Recursive search

- Search matches names only; it does not inspect file contents.
- Search is case-insensitive and searches descendants of the current directory.
- Hidden entries and their subtrees are omitted while hidden-item visibility is off.
- Search does not descend through directory symlinks found below its root. If the user explicitly navigates into a folder link, search can traverse that root while still skipping linked child directories.
- Inaccessible or disappearing child entries are skipped during recursive search; failure to open the search root is reported.
- Results use name ordering and can be selected and acted on like other entries. Their context menu can open the containing folder. In Details layout, a responsive Location column shows each result’s parent directory when space allows. In Icons layout, search cards include the parent directory; truncated search results show their full path on hover in all layouts.
- The GUI debounces searches while the field changes, cancels stale pending scans, and shows results in the active view. A clear button with the `Esc` hint cancels the current query. The TUI enters search with `/`, updates as text is entered, and exits search input with Enter or Esc.

#### 5.6 Copy, move, replace, and skip

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

#### 5.7 Rename, folder creation, and deletion

- New folder creates one child directory in the current directory. Names must be a single path component; an empty name or an existing destination is rejected.
- Rename keeps the item in its current parent. An existing destination name is rejected rather than replaced.
- Delete means “move to system trash,” not permanent recursive removal through the UI. The GUI asks for confirmation; the TUI requires `y`/`Y` and accepts `n`/Esc to cancel.
- Permission errors, missing paths, invalid names, and OS integration failures are displayed as operation errors.

#### 5.8 Opening files

- GUI double-click or context-menu Open launches a file through the system default application.
- The Details pane provides Open for a single file or folder and Copy Path for a single selection; a multi-selection can copy all selected paths as newline-separated text.
- TUI Enter/Right attempts to launch the selected file through the system opener. If no suitable desktop application exists, the TUI reports that it could not launch it; file management itself remains available.
- The app previews bounded UTF-8 text and selected raster image formats in the GUI; the TUI previews UTF-8 text only. ZIP archive browsing and content indexing are described in sections 4.13–4.16.

#### 5.9 End-to-end journeys

**Find and open:** Start in Home, navigate using folders/breadcrumbs/history or search names below the current folder, select a result, and open a folder or launch a file. In the TUI, use `/`, type the name fragment, then open the focused result with Enter.

**Copy or move:** Select one or more entries, use Copy/Cut and then Paste in the destination, or drag the selection onto a GUI folder. If destination names conflict, choose Replace, Skip, or Cancel. A successful complete cut/move clears its internal queue; failed and skipped moves keep the queue.

**Create or rename:** Choose `+ New > Folder` or Ctrl/Command+Shift+N in the GUI, or `n` in the TUI. For rename, select an item and use Rename/F2. Enter a single child name and commit; invalid or existing names produce an error without replacing the existing item.

**Delete:** Select one or more items and press Delete or choose Delete in the GUI, then confirm. In the TUI, use `d` and confirm with `y`. The app asks the platform trash integration to move items to trash.

**Headless session:** Run `filebrowser --tui` in an interactive terminal. Navigate, search, manage items with the key map, and quit with `q` or Ctrl+C; an active file operation is allowed to finish before exit. Terminal raw mode and the alternate screen are restored on exit.

### A.3 Desktop GUI specification

#### 6.1 Window, visual language, and platform conventions

- Initial window size: 1180 × 760 logical points.
- Minimum inner window size: 800 × 560 logical points so the main commands and file list remain usable.
- Combine Explorer’s tabbed location and details table with Finder’s concise navigation controls and Favorites/Locations sidebar sections.
- Use the active egui light/dark theme. Standard item spacing is 6 points horizontally and 4 vertically; buttons use 9 × 5 points of horizontal/vertical padding, and inactive/hovered controls have 4-point corners. Inactive control fills are transparent; hover and pressed fills are `#EBEBEB`/`#DEDEDE` in light mode and `#343434`/`#414141` in dark mode. Selection and focus use the active system palette. Folder glyphs use Windows/Linux yellow or macOS Finder blue, with theme-aware outlines that keep their shape distinct on pane and selection backgrounds; files use a page outline; symlinks use a link mark.
- Keep the shared layout and file-operation model consistent while following platform labels and shortcuts: Windows uses Ctrl, Delete, and the Recycle Bin; macOS uses Command, Move to Trash, Return to rename, Command+O to open, and Command+I for Get Info.
- Use Explorer/Finder interaction conventions: single-click selects, double-click opens, right-click exposes item actions, and drag-and-drop begins only after a pressed pointer moves.
- The left and right side panels are resizable; each splitter shows a horizontal-resize cursor and a tooltip naming its pane. The table columns are resizable; its final column fills remaining width.
- The window title follows platform convention: Windows/Linux combine the current folder name and `File Browser`; macOS shows the current folder name alone.

#### 6.2 Screen regions

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

#### 6.3 Selection and row targets

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

#### 6.4 Dialogs and transient states

- Confirmation and naming dialogs are centered modal surfaces with a dimmed backdrop; clicks cannot activate the file list underneath, and clicking outside cancels the current prompt.
- **New folder:** focused single-line name entry with Create/Cancel; Enter creates the folder and Esc cancels. After successful creation, the new folder is selected and focused.
- **Rename:** focused single-line name entry with Rename/Cancel. The filename is selected on open, excluding a recognized file extension so it can be preserved while editing the base name. Enter commits and Esc cancels. After successful rename, the renamed item is selected and focused when it remains visible.
- **Move to trash:** asks for confirmation and lists the affected item names; confirm with Move to Trash or cancel with Cancel/Esc.
- **Destination conflict:** shows the destination and conflicting item names, then offers Replace/Skip/Cancel when one or more target names already exist. Esc cancels.
- **Loading/searching/operation:** non-blocking UI with a status message; errors and skipped names remain visible after refresh.
- **Empty folder/search:** centered explanation distinguishes an empty folder from no search matches; folder state offers New folder and search state offers Clear search.
- **Loading/error:** an in-context spinner appears while the first folder listing or search is pending; failed locations show the error in the content area with a retry action that preserves the current path or search query.

#### 6.5 GUI shortcuts

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

### A.4 TUI specification

#### 7.1 Screen layout

The TUI fills the terminal with three stacked regions:

1. **Path header** (three terminal rows): current path in a bordered panel; long paths compact from the left on grapheme boundaries using terminal display width to keep the current location visible without overflowing. Terminal control and bidi-formatting characters in displayed paths are escaped; the original path remains unchanged for navigation and file operations.
2. **Files list** (remaining height, minimum two rows): a cyan selection marker, platform-colored folder marker (yellow on Windows/Linux, light blue on macOS), magenta symlink marker, and a name truncated to fit the terminal width. File markers use the terminal's default text color. Terminal control and bidi-formatting characters are escaped in displayed names; file operations use the original names. Items in the app-local cut queue appear dimmed until a complete move clears the queue. At 52 columns and wider, file sizes occupy a right-aligned column; narrower terminals hide the size column to preserve name space. Directories do not show a size. The focused row uses reverse highlighting.
3. **Status/help footer** (six rows at widths of 80 columns or more, seven rows at 52–79 columns, nine rows at 40–51 columns, and ten rows below 40 columns; naming/path prompts use three, four, or five rows as terminal width decreases, and delete/conflict prompts use five): current status and a responsive keyboard legend, or the active prompt with item names and transfer destination where relevant. Errors are light red, partial transfer outcomes and blocked actions are yellow, and the keyboard legend stays in the terminal's default color. While search input is active, the general legend is replaced with responsive search instructions. Selection count appears in the status line; a pending cut queue keeps its item count visible while browsing other folders.

The UI is keyboard-only. It supports UTF-8 text preview and editing, ZIP browsing, content indexing, and file-property editing through focused views and prompts; it has no image preview or separate Details pane. Terminal raw mode and the alternate screen must be restored on normal exit and on errors after terminal entry.

When the folder is empty, a search has no matches, or a listing/search is still loading or has failed, the list area displays a short status instead of remaining blank. Search results replace the prior visible rows while a new query is in flight.

#### 7.2 TUI key map

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
| `i` / `I` | Index the current folder tree / toggle content-search mode |
| `v` | Preview selected/current UTF-8 text file |
| `e` | Edit selected/current UTF-8 text file; Ctrl+S saves, Esc closes, and `x` discards changes |
| `P` | Edit selected/current file or folder properties (read-only and modified time) |
| `a` | Browse the selected/current ZIP archive |
| Archive browser | Enter opens a folder/member, `v` previews UTF-8 text members, Backspace/Left goes to the parent, and Esc closes |
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

#### 7.3 TUI prompts

- Path, new-folder, and rename prompts show the insertion point; Left/Right move by grapheme, Home/End move to either end, Backspace/Delete remove one grapheme, Ctrl+A clears the input, Enter commits, and Esc cancels. Prompt instructions wrap into additional footer rows on narrow terminals, and long input keeps the cursor-visible portion in view.
- Delete shows the item count; `y` confirms, while `n` or Esc cancels.
- On destination conflicts, `r` replaces, `s` skips conflicting items, and Esc cancels.
- The footer reports loading/search/operation progress and errors. Resizing is handled by Ratatui’s layout on the next draw.

### A.5 Functional requirements and acceptance criteria

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
| FR-29 | Match folder icon colors to platform conventions while keeping their shape legible. | GUI folder glyphs use yellow on Windows/Linux and Finder blue on macOS in the file list/grid and navigation sidebar; TUI folder markers use yellow on Windows/Linux and light blue on macOS, while symlink markers use magenta and file markers retain terminal text color. Theme-aware outlines distinguish the GUI folder shape on light/dark pane and selection backgrounds; cut-pending folders dim their fill and name while retaining the outline, and TUI cut-pending entries dim their marker and name. |
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
| FR-43 | Preview bounded file contents. | GUI previews UTF-8 text up to 1 MiB and BMP/GIF/JPEG/PNG/WebP images within decoder limits; TUI previews UTF-8 text and reports image preview as unavailable. |
| FR-44 | Index and search file contents. | Indexes UTF-8 files up to 1 MiB without following descendant directory symlinks; limits each build to 100,000 files, 100,000 directories, and 128 MiB of text; the volatile index is cleared after app-managed saves and file operations. |
| FR-45 | Edit UTF-8 text files. | GUI and TUI edit files up to 1 MiB, protect unsaved changes, and reject saves when the file revision is stale. |
| FR-46 | Browse ZIP archives without extraction. | GUI and TUI list safe archive entries and preview bounded UTF-8 text members; archive contents remain read-only. |
| FR-47 | Edit basic file properties. | GUI and TUI update read-only state and modified time for one regular non-symlink file or directory. |

### A.6 Design, accessibility, and resilience requirements

- All core tasks must be possible without a mouse in the TUI. GUI core actions have keyboard shortcuts and context/menu alternatives where implemented.
- Selection state must be visible through row highlighting or the TUI marker and cursor highlight. The focused GUI item has an additional outline so it remains distinguishable within a multi-selection.
- Interactive controls must retain visible focus/hover feedback without making ordinary file-name hover look like an active drag. During an active internal drag over a valid destination, the pointer indicates Move or Copy according to the platform modifier.
- Long filesystem work must not block the GUI frame loop or TUI event polling.
- Errors must identify the operation and retain useful OS error text. Conflict skip and refresh failures must not be hidden by success messages.
- Search must skip directory links found below its root to prevent recursive loops and unexpected traversal. A folder link explicitly opened by the user can serve as the search root.
- Trash deletion must remain recoverable through the platform trash facility when supported.
- The current implementation follows the system egui light/dark theme. Localization, custom themes, and screen-reader conformance are not yet specified as shipped capabilities.

### A.7 Linux roadmap

Linux is the next platform milestone after Windows v1:

1. Produce an Ubuntu `.deb` and openSUSE `.rpm` from the Rust application.
2. Validate GUI startup, desktop file opening, trash behavior, drag/drop, and the navigation/search workflows on both distributions.
3. Validate TUI startup, keyboard navigation, resizing, file operations, and terminal restoration in a headless terminal on both distributions.
4. Implement and validate Linux file-manager clipboard interoperability for GUI copy/cut/paste. The current non-Windows clipboard adapter reports that file clipboard integration is unavailable; the TUI internal queue remains available.

Linux milestone acceptance requires running the package and smoke-test matrix on the target distributions, not just compiling the shared Rust source on Windows.

### A.8 Implementation map

| Area | Source |
| --- | --- |
| Mode detection, CLI arguments, and mode tests | `src/main.rs` |
| Shared filesystem model, search, sorting, transfers, trash, and tests | `src/core.rs` |
| Desktop screen, interactions, dialogs, and state | `src/gui.rs` |
| TUI screen, keyboard loop, prompts, and terminal lifecycle | `src/tui.rs` |
| Cross-platform capability selection and local-time formatting | `src/platform.rs` |
| Windows GUI detection, local time, and CF_HDROP clipboard | `src/platform/windows.rs` |
