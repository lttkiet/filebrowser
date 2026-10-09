use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::{
    core::{self, ConflictChoice, Entry, SortBy, Transfer},
    display,
    platform,
};

enum Event {
    Listed(Option<u64>, PathBuf, Result<Vec<Entry>, String>),
    Searched(u64, String, Result<Vec<Entry>, String>),
    Finished(
        String,
        Option<Vec<PathBuf>>,
        Option<Vec<PathBuf>>,
        HashSet<PathBuf>,
        Option<PathBuf>,
    ),
}

enum Confirm {
    Delete(Vec<PathBuf>),
    Conflict(Vec<PathBuf>, PathBuf, Transfer),
    NewFolder,
    Rename(PathBuf),
}

#[derive(Clone)]
enum InternalDrag {
    Files(Vec<PathBuf>),
    Favorite(PathBuf),
    Tab(usize),
}

#[derive(Clone)]
struct TabState {
    path: PathBuf,
    query: String,
    list_scroll_offset: f32,
    icon_scroll_offset: f32,
    view_mode: ViewMode,
    icon_card_size: u16,
    sort: SortBy,
    sort_descending: bool,
    history: Vec<PathBuf>,
    history_index: usize,
    history_selection: Vec<HistoryState>,
    selected: Vec<PathBuf>,
    focus_path: Option<PathBuf>,
}

#[derive(Clone, Default)]
struct HistoryState {
    selected: Vec<PathBuf>,
    focus_path: Option<PathBuf>,
    list_scroll_offset: f32,
    icon_scroll_offset: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Details,
    List,
    Icons,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct GuiPreferences {
    view_mode: ViewMode,
    icon_card_size: u16,
    show_hidden: bool,
    details_open: bool,
    navigation_open: bool,
    recent_locations_open: bool,
    favorites_open: bool,
    locations_open: bool,
    sort: SortBy,
    sort_descending: bool,
    navigation_width: u16,
    details_width: u16,
    window_width: u16,
    window_height: u16,
}

impl Default for GuiPreferences {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::Details,
            icon_card_size: 136,
            show_hidden: false,
            details_open: false,
            navigation_open: true,
            recent_locations_open: true,
            favorites_open: true,
            locations_open: true,
            sort: SortBy::Name,
            sort_descending: false,
            navigation_width: 218,
            details_width: 270,
            window_width: 1180,
            window_height: 760,
        }
    }
}

pub fn run(home: PathBuf) -> anyhow::Result<()> {
    let preferences = load_gui_preferences();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("File Browser")
            .with_inner_size([
                f32::from(preferences.window_width),
                f32::from(preferences.window_height),
            ])
            .with_min_inner_size([800.0, 560.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "File Browser",
        options,
        Box::new(|_| Ok(Box::new(Browser::new(home, preferences)))),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(())
}

struct Browser {
    home: PathBuf,
    current: PathBuf,
    favorites: Vec<PathBuf>,
    recent_locations: Vec<PathBuf>,
    path_edit: String,
    path_editing: bool,
    path_edit_select_all_pending: bool,
    tabs: Vec<TabState>,
    closed_tabs: Vec<TabState>,
    active_tab: usize,
    pending_tab_selection: Option<(PathBuf, Vec<PathBuf>, Option<PathBuf>)>,
    entries: Arc<Vec<Entry>>,
    search_results: Option<Arc<Vec<Entry>>>,
    search_generation: u64,
    search_epoch: Arc<AtomicU64>,
    listing_generation: u64,
    query: String,
    type_select_query: String,
    type_select_at: Option<Instant>,
    selected: HashSet<PathBuf>,
    context_menu_request: Option<PathBuf>,
    clipboard: Vec<PathBuf>,
    clipboard_available: bool,
    next_clipboard_check: Instant,
    cut: bool,
    show_hidden: bool,
    details_open: bool,
    navigation_open: bool,
    recent_locations_open: bool,
    favorites_open: bool,
    locations_open: bool,
    navigation_width: f32,
    details_width: f32,
    window_width: u16,
    window_height: u16,
    view_mode: ViewMode,
    icon_card_size: u16,
    saved_preferences: GuiPreferences,
    icon_columns: usize,
    page_rows: usize,
    list_scroll_offset: f32,
    list_scroll_restore: Option<f32>,
    icon_scroll_offset: f32,
    icon_scroll_target: Option<PathBuf>,
    sort: SortBy,
    sort_descending: bool,
    selection_anchor: Option<PathBuf>,
    focus_path: Option<PathBuf>,
    confirm: Option<Confirm>,
    name_edit: String,
    new_folder_focus_pending: bool,
    rename_focus_pending: bool,
    rename_selection_end: usize,
    active_operations: usize,
    message: String,
    external_drop_target: Option<PathBuf>,
    events_tx: Sender<Event>,
    events_rx: Receiver<Event>,
}

impl Browser {
    fn new(home: PathBuf, preferences: GuiPreferences) -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        let mut this = Self {
            home: home.clone(),
            current: home.clone(),
            favorites: load_favorites(),
            recent_locations: load_recent_locations(),
            path_edit: home.to_string_lossy().into_owned(),
            path_editing: false,
            path_edit_select_all_pending: false,
            tabs: vec![TabState {
                path: home.clone(),
                query: String::new(),
                list_scroll_offset: 0.0,
                icon_scroll_offset: 0.0,
                view_mode: preferences.view_mode,
                icon_card_size: preferences.icon_card_size,
                sort: preferences.sort,
                sort_descending: preferences.sort_descending,
                history: vec![home.clone()],
                history_index: 0,
                history_selection: vec![HistoryState::default()],
                selected: Vec::new(),
                focus_path: None,
            }],
            closed_tabs: Vec::new(),
            active_tab: 0,
            pending_tab_selection: None,
            entries: Arc::new(Vec::new()),
            search_results: None,
            search_generation: 0,
            search_epoch: Arc::new(AtomicU64::new(0)),
            listing_generation: 0,
            query: String::new(),
            type_select_query: String::new(),
            type_select_at: None,
            selected: HashSet::new(),
            context_menu_request: None,
            clipboard: Vec::new(),
            clipboard_available: false,
            next_clipboard_check: Instant::now(),
            cut: false,
            show_hidden: preferences.show_hidden,
            details_open: preferences.details_open,
            navigation_open: preferences.navigation_open,
            recent_locations_open: preferences.recent_locations_open,
            favorites_open: preferences.favorites_open,
            locations_open: preferences.locations_open,
            navigation_width: f32::from(preferences.navigation_width),
            details_width: f32::from(preferences.details_width),
            window_width: preferences.window_width,
            window_height: preferences.window_height,
            view_mode: preferences.view_mode,
            icon_card_size: preferences.icon_card_size,
            saved_preferences: preferences,
            icon_columns: 1,
            page_rows: 10,
            list_scroll_offset: 0.0,
            list_scroll_restore: None,
            icon_scroll_offset: 0.0,
            icon_scroll_target: None,
            sort: preferences.sort,
            sort_descending: preferences.sort_descending,
            selection_anchor: None,
            focus_path: None,
            confirm: None,
            name_edit: String::new(),
            new_folder_focus_pending: false,
            rename_focus_pending: false,
            rename_selection_end: 0,
            active_operations: 0,
            message: String::from("Ready"),
            external_drop_target: None,
            events_tx,
            events_rx,
        };
        this.remember_recent(home.clone());
        this.load(home);
        this
    }

    fn load(&mut self, path: PathBuf) {
        self.pending_tab_selection = None;
        self.list_scroll_restore = None;
        self.next_search_generation();
        self.listing_generation = self.listing_generation.wrapping_add(1);
        let generation = self.listing_generation;
        self.current = path.clone();
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.path = path.clone();
        }
        self.path_edit = path.to_string_lossy().into_owned();
        self.query.clear();
        self.type_select_query.clear();
        self.type_select_at = None;
        self.search_results = None;
        self.entries = Arc::new(Vec::new());
        self.selected.clear();
        self.context_menu_request = None;
        self.selection_anchor = None;
        self.focus_path = None;
        self.icon_scroll_target = None;
        self.message = "Loading…".into();
        let tx = self.events_tx.clone();
        let show_hidden = self.show_hidden;
        let sort = self.sort;
        let descending = self.sort_descending;
        thread::spawn(move || {
            let result =
                core::list_dir(&path, show_hidden, sort, descending).map_err(|e| e.to_string());
            let _ = tx.send(Event::Listed(Some(generation), path, result));
        });
    }

    fn refresh(&mut self) {
        let path = self.current.clone();
        let query = self.query.clone();
        let selected = self.selected.clone();
        let selection_anchor = self.selection_anchor.clone();
        let focus_path = self.focus_path.clone();
        self.load(path);
        self.selected = selected.clone();
        self.selection_anchor = selection_anchor.clone();
        self.focus_path = focus_path.clone();
        if !query.trim().is_empty() {
            self.query = query;
            self.search();
            self.selected = selected;
            self.selection_anchor = selection_anchor;
            self.focus_path = focus_path;
        }
    }

    fn navigate(&mut self, path: PathBuf) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.query.clear();
        }
        let changes_history = self
            .tabs
            .get(self.active_tab)
            .is_some_and(|tab| tab.history.get(tab.history_index) != Some(&path));
        if changes_history {
            self.save_active_history_selection();
            if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                tab.history.truncate(tab.history_index + 1);
                tab.history_selection.truncate(tab.history_index + 1);
                tab.history_selection.push(HistoryState::default());
                tab.history.push(path.clone());
                tab.history_index = tab.history.len() - 1;
                tab.query.clear();
            }
        }
        self.remember_recent(path.clone());
        self.icon_scroll_offset = 0.0;
        self.load(path);
        self.list_scroll_restore = Some(0.0);
    }

    fn navigate_selecting(&mut self, path: PathBuf, selected: PathBuf) {
        self.navigate(path.clone());
        self.pending_tab_selection = Some((path, vec![selected.clone()], Some(selected)));
    }

    fn navigate_to_parent(&mut self) {
        if let Some(parent) = core::parent(&self.current) {
            let selected = self.current.clone();
            self.navigate_selecting(parent, selected);
        }
    }

    fn go_back(&mut self) {
        self.save_active_history_selection();
        let restore = {
            let Some(tab) = self.tabs.get_mut(self.active_tab) else {
                return;
            };
            if tab.history_index > 0 {
                tab.history_index -= 1;
                tab.query.clear();
                let index = tab.history_index;
                Some((
                    tab.history[index].clone(),
                    tab.history_selection[index].clone(),
                ))
            } else {
                None
            }
        };
        if let Some((path, state)) = restore {
            self.remember_recent(path.clone());
            self.icon_scroll_offset = state.icon_scroll_offset;
            self.load(path.clone());
            self.list_scroll_restore = Some(state.list_scroll_offset);
            self.pending_tab_selection = Some((path, state.selected, state.focus_path));
        }
    }

    fn go_forward(&mut self) {
        self.save_active_history_selection();
        let restore = {
            let Some(tab) = self.tabs.get_mut(self.active_tab) else {
                return;
            };
            if tab.history_index + 1 < tab.history.len() {
                tab.history_index += 1;
                tab.query.clear();
                let index = tab.history_index;
                Some((
                    tab.history[index].clone(),
                    tab.history_selection[index].clone(),
                ))
            } else {
                None
            }
        };
        if let Some((path, state)) = restore {
            self.remember_recent(path.clone());
            self.icon_scroll_offset = state.icon_scroll_offset;
            self.load(path.clone());
            self.list_scroll_restore = Some(state.list_scroll_offset);
            self.pending_tab_selection = Some((path, state.selected, state.focus_path));
        }
    }

    fn open_tab(&mut self, path: PathBuf) {
        self.save_active_history_selection();
        self.remember_recent(path.clone());
        self.tabs.push(TabState {
            path: path.clone(),
            query: String::new(),
            list_scroll_offset: 0.0,
            icon_scroll_offset: 0.0,
            view_mode: self.view_mode,
            icon_card_size: self.icon_card_size,
            sort: self.sort,
            sort_descending: self.sort_descending,
            history: vec![path.clone()],
            history_index: 0,
            history_selection: vec![HistoryState::default()],
            selected: Vec::new(),
            focus_path: None,
        });
        self.active_tab = self.tabs.len() - 1;
        self.load(path);
        self.list_scroll_restore = Some(0.0);
    }

    fn cycle_tab(&mut self, reverse: bool) {
        if self.tabs.len() < 2 {
            return;
        }
        let count = self.tabs.len();
        let index = if reverse {
            (self.active_tab + count - 1) % count
        } else {
            (self.active_tab + 1) % count
        };
        self.switch_tab(index);
    }

    fn save_active_tab_selection(&mut self) {
        let visible_entries = self.visible_entries();
        let selected = visible_entries
            .iter()
            .filter(|entry| self.selected.contains(&entry.path))
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        let focus_path = self
            .focus_path
            .as_ref()
            .filter(|path| visible_entries.iter().any(|entry| &entry.path == *path))
            .cloned();
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.query = self.query.clone();
            tab.view_mode = self.view_mode;
            tab.icon_card_size = self.icon_card_size;
            tab.sort = self.sort;
            tab.sort_descending = self.sort_descending;
            tab.icon_scroll_offset = self.icon_scroll_offset;
            if self.message != "Loading…" {
                tab.selected = selected;
                tab.focus_path = focus_path;
                tab.list_scroll_offset = self.list_scroll_offset;
            }
        }
    }

    fn save_active_history_selection(&mut self) {
        self.save_active_tab_selection();
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        let index = tab.history_index;
        let selection = HistoryState {
            selected: tab.selected.clone(),
            focus_path: tab.focus_path.clone(),
            list_scroll_offset: tab.list_scroll_offset,
            icon_scroll_offset: tab.icon_scroll_offset,
        };
        if let Some(tab) = self.tabs.get_mut(self.active_tab)
            && let Some(saved) = tab.history_selection.get_mut(index)
        {
            *saved = selection;
        }
    }

    fn switch_tab(&mut self, index: usize) {
        if index >= self.tabs.len() || index == self.active_tab {
            return;
        }
        self.save_active_history_selection();
        self.active_tab = index;
        let tab = self.tabs[index].clone();
        self.sort = tab.sort;
        self.sort_descending = tab.sort_descending;
        self.icon_card_size = tab.icon_card_size;
        self.icon_scroll_offset = tab.icon_scroll_offset;
        self.load(tab.path.clone());
        self.list_scroll_restore = Some(tab.list_scroll_offset);
        self.view_mode = tab.view_mode;
        self.query = tab.query.clone();
        self.pending_tab_selection = Some((tab.path, tab.selected, tab.focus_path));
        if !self.query.trim().is_empty() {
            self.search();
        }
    }

    fn reorder_tab(&mut self, source: usize, target: usize, after: bool) {
        if source >= self.tabs.len() || target >= self.tabs.len() || source == target {
            return;
        }
        let active = self.active_tab;
        let tab = self.tabs.remove(source);
        let target = target.saturating_sub(usize::from(source < target));
        let insert = (target + usize::from(after)).min(self.tabs.len());
        self.tabs.insert(insert, tab);
        self.active_tab = if active == source {
            insert
        } else {
            let active = active.saturating_sub(usize::from(source < active));
            active + usize::from(insert <= active)
        };
        self.message = "Reordered tabs".into();
    }

    fn close_tab(&mut self, index: usize) {
        if self.tabs.len() == 1 {
            self.tabs[0] = TabState {
                path: self.home.clone(),
                query: String::new(),
                list_scroll_offset: 0.0,
                icon_scroll_offset: 0.0,
                view_mode: self.saved_preferences.view_mode,
                icon_card_size: self.saved_preferences.icon_card_size,
                sort: self.saved_preferences.sort,
                sort_descending: self.saved_preferences.sort_descending,
                history: vec![self.home.clone()],
                history_index: 0,
                history_selection: vec![HistoryState::default()],
                selected: Vec::new(),
                focus_path: None,
            };
            self.active_tab = 0;
            self.load(self.home.clone());
            self.list_scroll_restore = Some(0.0);
            return;
        }
        self.save_active_history_selection();
        let closing_active = index == self.active_tab;
        let closed = self.tabs.remove(index);
        self.remember_closed_tab(closed);
        if index < self.active_tab {
            self.active_tab -= 1;
        } else if index == self.active_tab {
            self.active_tab = self.active_tab.min(self.tabs.len() - 1);
        }
        if closing_active {
            let tab = self.tabs[self.active_tab].clone();
            self.sort = tab.sort;
            self.sort_descending = tab.sort_descending;
            self.icon_card_size = tab.icon_card_size;
            self.icon_scroll_offset = tab.icon_scroll_offset;
            self.load(tab.path.clone());
            self.list_scroll_restore = Some(tab.list_scroll_offset);
            self.view_mode = tab.view_mode;
            self.query = tab.query.clone();
            self.pending_tab_selection = Some((tab.path, tab.selected, tab.focus_path));
            if !self.query.trim().is_empty() {
                self.search();
            }
        }
    }

    fn close_other_tabs(&mut self, index: usize) {
        self.save_active_history_selection();
        let Some(tab) = self.tabs.get(index).cloned() else {
            return;
        };
        let closed_tabs = self
            .tabs
            .drain(..)
            .enumerate()
            .filter_map(|(other_index, tab)| (other_index != index).then_some(tab))
            .collect::<Vec<_>>();
        self.tabs = vec![tab];
        for tab in closed_tabs {
            self.remember_closed_tab(tab);
        }
        self.active_tab = 0;
        let tab = self.tabs[0].clone();
        self.sort = tab.sort;
        self.sort_descending = tab.sort_descending;
        self.icon_card_size = tab.icon_card_size;
        self.icon_scroll_offset = tab.icon_scroll_offset;
        self.load(tab.path.clone());
        self.list_scroll_restore = Some(tab.list_scroll_offset);
        self.view_mode = tab.view_mode;
        self.query = tab.query.clone();
        self.pending_tab_selection = Some((tab.path, tab.selected, tab.focus_path));
        if !self.query.trim().is_empty() {
            self.search();
        }
    }

    fn remember_closed_tab(&mut self, tab: TabState) {
        self.closed_tabs.push(tab);
        if self.closed_tabs.len() > 10 {
            self.closed_tabs.remove(0);
        }
    }

    fn reopen_closed_tab(&mut self) {
        let Some(tab) = self.closed_tabs.pop() else {
            return;
        };
        self.save_active_history_selection();
        self.tabs.push(tab.clone());
        self.active_tab = self.tabs.len() - 1;
        self.sort = tab.sort;
        self.sort_descending = tab.sort_descending;
        self.icon_card_size = tab.icon_card_size;
        self.icon_scroll_offset = tab.icon_scroll_offset;
        self.load(tab.path.clone());
        self.list_scroll_restore = Some(tab.list_scroll_offset);
        self.view_mode = tab.view_mode;
        self.query = tab.query.clone();
        self.pending_tab_selection = Some((tab.path, tab.selected, tab.focus_path));
        if !self.query.trim().is_empty() {
            self.search();
        }
    }

    fn search(&mut self) {
        let generation = self.next_search_generation();
        self.type_select_query.clear();
        self.type_select_at = None;
        if self.query.trim().is_empty() {
            self.search_results = None;
            self.selected.clear();
            self.selection_anchor = None;
            self.focus_path = None;
            self.message = item_count(self.entries.len());
            return;
        }
        self.search_results = None;
        self.selected.clear();
        self.selection_anchor = None;
        self.focus_path = None;
        let root = self.current.clone();
        let query = self.query.clone();
        let show_hidden = self.show_hidden;
        let tx = self.events_tx.clone();
        let q = query.clone();
        let search_epoch = self.search_epoch.clone();
        self.message = "Searching…".into();
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_millis(160));
            if search_epoch.load(Ordering::Relaxed) != generation {
                return;
            }
            let result = core::search(&root, &query, show_hidden).map_err(|e| e.to_string());
            let _ = tx.send(Event::Searched(generation, q, result));
        });
    }

    fn next_search_generation(&mut self) -> u64 {
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_epoch
            .store(self.search_generation, Ordering::Relaxed);
        self.search_generation
    }

    fn retain_selection(&mut self, entries: &[Entry]) {
        self.selected
            .retain(|path| entries.iter().any(|entry| &entry.path == path));
        let first_selected = entries
            .iter()
            .find(|entry| self.selected.contains(&entry.path))
            .map(|entry| entry.path.clone());
        if self
            .selection_anchor
            .as_ref()
            .is_some_and(|path| !self.selected.contains(path))
        {
            self.selection_anchor = first_selected.clone();
        }
        if self
            .focus_path
            .as_ref()
            .is_some_and(|path| !self.selected.contains(path))
        {
            self.focus_path = first_selected;
        }
    }

    fn toggle_favorite(&mut self, path: PathBuf) {
        let previous = self.favorites.clone();
        let pinned = if let Some(index) = self.favorites.iter().position(|item| item == &path) {
            self.favorites.remove(index);
            false
        } else {
            if !path.is_dir() {
                self.message = "Favorites can only contain existing folders".into();
                return;
            }
            self.favorites.push(path.clone());
            true
        };

        if let Err(error) = save_favorites(&self.favorites) {
            self.favorites = previous;
            self.message = format!("Could not save Favorites: {error}");
        } else {
            self.message = if pinned {
                format!("Added {} to Favorites", folder_name(&path))
            } else {
                format!("Removed {} from Favorites", folder_name(&path))
            };
        }
    }

    fn reorder_favorite(&mut self, source: &Path, target: &Path, after: bool) {
        let Some(source_index) = self.favorites.iter().position(|path| path == source) else {
            return;
        };
        if self.favorites.get(source_index).is_some_and(|path| path == target) {
            return;
        }
        let previous = self.favorites.clone();
        let moving = self.favorites.remove(source_index);
        let target_index = self
            .favorites
            .iter()
            .position(|path| path == target)
            .unwrap_or(self.favorites.len());
        self.favorites
            .insert((target_index + usize::from(after)).min(self.favorites.len()), moving);
        if let Err(error) = save_favorites(&self.favorites) {
            self.favorites = previous;
            self.message = format!("Could not reorder Favorites: {error}");
        } else {
            self.message = "Updated Favorites order".into();
        }
    }

    fn run_mutation(
        &mut self,
        label: impl Into<String>,
        clear_after: Option<Vec<PathBuf>>,
        select_after: Option<Vec<PathBuf>>,
        operation: impl FnOnce() -> anyhow::Result<Option<String>> + Send + 'static,
    ) {
        self.active_operations = self.active_operations.saturating_add(1);
        self.message = format!("{}…", label.into());
        let tx = self.events_tx.clone();
        let current = self.current.clone();
        let hidden = self.show_hidden;
        let sort = self.sort;
        let descending = self.sort_descending;
        let query = self.query.clone();
        let search_generation = self.search_generation;
        let search_epoch = self.search_epoch.clone();
        let selected_before = self.selected.clone();
        let focus_before = self.focus_path.clone();
        thread::spawn(move || {
            let operation_result = operation();
            let operation_succeeded = operation_result.is_ok();
            let clear_after = operation_result
                .as_ref()
                .is_ok_and(Option::is_none)
                .then_some(clear_after)
                .flatten();
            let mut message = match operation_result {
                Ok(Some(message)) => message,
                Ok(None) => "Operation completed".to_string(),
                Err(e) => format!("Error: {e}"),
            };
            let result =
                core::list_dir(&current, hidden, sort, descending).map_err(|e| e.to_string());
            let search_result = if query.trim().is_empty() {
                None
            } else {
                thread::sleep(std::time::Duration::from_millis(160));
                (search_epoch.load(Ordering::Relaxed) == search_generation)
                    .then(|| core::search(&current, &query, hidden).map_err(|e| e.to_string()))
            };
            if let Err(error) = &result {
                message = format!("{message}; refresh failed: {error}");
            }
            let _ = tx.send(Event::Listed(None, current, result));
            if let Some(search_result) = search_result {
                let _ = tx.send(Event::Searched(search_generation, query, search_result));
            }
            let _ = tx.send(Event::Finished(
                message,
                clear_after,
                operation_succeeded.then_some(select_after).flatten(),
                selected_before,
                focus_before,
            ));
        });
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.visible_entries()
            .iter()
            .filter(|entry| self.selected.contains(&entry.path))
            .map(|entry| entry.path.clone())
            .collect()
    }

    fn paste(&mut self) {
        self.paste_to(self.current.clone());
    }

    fn refresh_clipboard_state(&mut self) {
        let now = Instant::now();
        if now < self.next_clipboard_check {
            return;
        }
        self.next_clipboard_check = now + Duration::from_millis(500);
        self.clipboard_available = platform::read_file_clipboard()
            .map(|(paths, _)| !paths.is_empty())
            .unwrap_or(false)
            || !self.clipboard.is_empty();
    }

    fn paste_to(&mut self, destination: PathBuf) {
        let (paths, cut) =
            platform::read_file_clipboard().unwrap_or_else(|_| (self.clipboard.clone(), self.cut));
        if paths.is_empty() {
            self.message = "Clipboard has no file paths".into();
            return;
        }
        self.confirm = Some(Confirm::Conflict(
            paths,
            destination,
            if cut { Transfer::Move } else { Transfer::Copy },
        ));
    }

    fn request_drop(&mut self, paths: Vec<PathBuf>, destination: PathBuf, action: Transfer) {
        let destination_resolved = destination
            .canonicalize()
            .unwrap_or_else(|_| destination.clone());
        let mut candidates = Vec::new();
        for path in paths {
            let source_resolved = path.canonicalize().unwrap_or_else(|_| path.clone());
            if source_resolved == destination_resolved {
                self.message = "A folder cannot be dropped onto itself".into();
                return;
            }
            if path.is_dir() && destination_resolved.starts_with(&source_resolved) {
                self.message = "A folder cannot be dropped into itself".into();
                return;
            }
            let already_in_destination = path
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .is_some_and(|parent| parent == destination_resolved);
            if !already_in_destination {
                candidates.push(path);
            }
        }
        if candidates.is_empty() {
            self.message = "Items are already in this folder".into();
            return;
        }
        self.confirm = Some(Confirm::Conflict(candidates, destination, action));
    }

    fn on_key(&mut self, ctx: &egui::Context) {
        let text_input_focused = ctx.wants_keyboard_input();
        let search_focused =
            ctx.memory(|memory| memory.focused() == Some(egui::Id::new("search")));
        let dialog_open = self.confirm.is_some();
        let popup_open = egui::containers::Popup::is_any_open(ctx);
        ctx.input(|input| {
            let mut escape_consumed = false;
            if !dialog_open && !popup_open {
                if input.pointer.button_pressed(egui::PointerButton::Extra1) {
                    self.go_back();
                } else if input.pointer.button_pressed(egui::PointerButton::Extra2) {
                    self.go_forward();
                }
            }
            if dialog_open && input.key_pressed(egui::Key::Escape) {
                self.confirm = None;
                self.rename_focus_pending = false;
                self.new_folder_focus_pending = false;
                return;
            }
            if !dialog_open && !popup_open && input.key_pressed(egui::Key::Escape) {
                if self.path_editing {
                    self.path_editing = false;
                    self.path_edit_select_all_pending = false;
                    self.path_edit = self.current.to_string_lossy().into_owned();
                    ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::new("path")));
                    escape_consumed = true;
                } else if !self.query.is_empty() {
                    self.query.clear();
                    self.search();
                    if search_focused {
                        ctx.memory_mut(|memory| {
                            memory.surrender_focus(egui::Id::new("search"));
                        });
                    }
                    escape_consumed = true;
                } else if search_focused {
                    self.clear_selection();
                    ctx.memory_mut(|memory| {
                        memory.surrender_focus(egui::Id::new("search"));
                    });
                    escape_consumed = true;
                }
            }
            let handles_items =
                !text_input_focused && !dialog_open && !popup_open && self.confirm.is_none();
            if handles_items {
                if input.modifiers.shift && input.key_pressed(egui::Key::F10)
                    && let Some(path) = self.focus_path.clone()
                {
                    if !self.selected.contains(&path) {
                        self.selected.clear();
                        self.selected.insert(path.clone());
                        self.selection_anchor = Some(path.clone());
                    }
                    self.context_menu_request = Some(path);
                }
                if cfg!(target_os = "macos")
                    && input.modifiers.command
                    && input.key_pressed(egui::Key::ArrowDown)
                {
                    self.open_focused_entry();
                } else if input.key_pressed(egui::Key::ArrowDown) {
                    let delta = if self.view_mode == ViewMode::Icons {
                        self.icon_columns as isize
                    } else {
                        1
                    };
                    self.move_selection(delta, input.modifiers.shift);
                }
                if input.key_pressed(egui::Key::ArrowUp)
                    && (input.modifiers.alt
                        || (cfg!(target_os = "macos") && input.modifiers.command))
                {
                    self.navigate_to_parent();
                } else if input.key_pressed(egui::Key::ArrowUp) {
                    let delta = if self.view_mode == ViewMode::Icons {
                        -(self.icon_columns as isize)
                    } else {
                        -1
                    };
                    self.move_selection(delta, input.modifiers.shift);
                }
                if self.view_mode == ViewMode::Icons
                    && !input.modifiers.alt
                    && input.key_pressed(egui::Key::ArrowRight)
                {
                    self.move_selection(1, input.modifiers.shift);
                }
                if self.view_mode == ViewMode::Icons
                    && !input.modifiers.alt
                    && input.key_pressed(egui::Key::ArrowLeft)
                {
                    self.move_selection(-1, input.modifiers.shift);
                }
                if input.key_pressed(egui::Key::Home) {
                    self.move_selection_to_edge(false, input.modifiers.shift);
                }
                if input.key_pressed(egui::Key::End) {
                    self.move_selection_to_edge(true, input.modifiers.shift);
                }
                if input.key_pressed(egui::Key::PageDown) {
                    let page = self.page_rows as isize
                        * if self.view_mode == ViewMode::Icons {
                            self.icon_columns as isize
                        } else {
                            1
                        };
                    self.move_selection(page, input.modifiers.shift);
                }
                if input.key_pressed(egui::Key::PageUp) {
                    let page = self.page_rows as isize
                        * if self.view_mode == ViewMode::Icons {
                            self.icon_columns as isize
                        } else {
                            1
                        };
                    self.move_selection(-page, input.modifiers.shift);
                }
                let info_shortcut = if cfg!(target_os = "macos") {
                    input.modifiers.command && input.key_pressed(egui::Key::I)
                } else {
                    input.modifiers.alt && input.key_pressed(egui::Key::Enter)
                };
                if info_shortcut {
                    if !self.selected_paths().is_empty() {
                        self.details_open = true;
                    }
                } else if input.modifiers.command && input.key_pressed(egui::Key::Enter) {
                    self.open_focused_in_new_tab();
                } else if cfg!(target_os = "macos")
                    && input.modifiers.command
                    && input.key_pressed(egui::Key::O)
                {
                    self.open_focused_entry();
                } else if cfg!(target_os = "macos")
                    && !input.modifiers.command
                    && !input.modifiers.ctrl
                    && !input.modifiers.alt
                    && !input.modifiers.shift
                    && input.key_pressed(egui::Key::Enter)
                {
                    self.rename_from_keyboard();
                } else if input.key_pressed(egui::Key::Enter) {
                    self.open_focused_entry();
                }
                if input.key_pressed(egui::Key::Escape) && !escape_consumed {
                    self.clear_selection();
                }
                if input.modifiers.command && input.key_pressed(egui::Key::A) {
                    self.select_all_visible();
                }
                if input.modifiers.command && input.key_pressed(egui::Key::C) {
                    let paths = self.selected_paths();
                    if !paths.is_empty() {
                        self.clipboard = paths.clone();
                        self.clipboard_available = true;
                        self.cut = false;
                        self.message = platform::write_file_clipboard(&paths, false)
                            .map(|_| "Copied to clipboard".into())
                            .unwrap_or_else(|e| format!("Copied internally: {e}"));
                    }
                }
                if input.modifiers.command && input.key_pressed(egui::Key::X) {
                    let paths = self.selected_paths();
                    if !paths.is_empty() {
                        self.clipboard = paths.clone();
                        self.clipboard_available = true;
                        self.cut = true;
                        self.message = platform::write_file_clipboard(&paths, true)
                            .map(|_| "Cut to clipboard".into())
                            .unwrap_or_else(|e| format!("Cut internally: {e}"));
                    }
                }
                if input.modifiers.command && input.key_pressed(egui::Key::V) {
                    self.paste();
                }
                let delete_pressed = if cfg!(target_os = "macos") {
                    input.modifiers.command
                        && (input.key_pressed(egui::Key::Backspace)
                            || input.key_pressed(egui::Key::Delete))
                } else {
                    input.key_pressed(egui::Key::Delete)
                };
                if delete_pressed && !self.selected.is_empty() {
                    self.confirm = Some(Confirm::Delete(self.selected_paths()));
                }
                if input.key_pressed(egui::Key::F2) {
                    self.rename_from_keyboard();
                }
                let toggle_hidden_pressed = if cfg!(target_os = "macos") {
                    input.modifiers.command
                        && input.modifiers.shift
                        && input.key_pressed(egui::Key::Period)
                } else {
                    input.modifiers.command && input.key_pressed(egui::Key::H)
                };
                if toggle_hidden_pressed {
                    self.show_hidden = !self.show_hidden;
                    self.refresh();
                }
                let plain_typing = !input.modifiers.command
                    && !input.modifiers.ctrl
                    && !input.modifiers.alt;
                if plain_typing {
                    for event in &input.events {
                        let egui::Event::Text(text) = event else {
                            continue;
                        };
                        if text.chars().all(char::is_control) {
                            continue;
                        }
                        let now = Instant::now();
                        let mut fresh_query = self
                            .type_select_at
                            .is_none_or(|last| now.duration_since(last).as_millis() > 850);
                        if !fresh_query
                            && text.chars().count() == 1
                            && self.type_select_query.chars().all(|previous| {
                                previous.to_lowercase().to_string() == text.to_lowercase()
                            })
                        {
                            fresh_query = true;
                        }
                        if fresh_query {
                            self.type_select_query.clear();
                        }
                        self.type_select_query.push_str(text);
                        self.type_select_at = Some(now);
                        let entries = self.visible_entries().to_vec();
                        let current = self
                            .focus_path
                            .as_ref()
                            .and_then(|path| entries.iter().position(|entry| &entry.path == path));
                        let start = if fresh_query && !entries.is_empty() {
                            current.map_or(0, |index| (index + 1) % entries.len())
                        } else {
                            0
                        };
                        let query = self.type_select_query.to_lowercase();
                        let found = (0..entries.len())
                            .map(|offset| (start + offset) % entries.len().max(1))
                            .find(|index| entries[*index].name.to_lowercase().starts_with(&query));
                        if let Some(index) = found {
                            self.set_keyboard_selection(&entries, index, false, current);
                            self.message = format!("Selected {}", entries[index].name);
                        } else {
                            self.message = format!("No item starts with {}", self.type_select_query);
                        }
                    }
                }
            }
            if !dialog_open && !popup_open {
                let cycle_tab_pressed = if cfg!(target_os = "macos") {
                    input.modifiers.ctrl && input.key_pressed(egui::Key::Tab)
                } else {
                    input.modifiers.command && input.key_pressed(egui::Key::Tab)
                };
                if cycle_tab_pressed {
                    self.cycle_tab(input.modifiers.shift);
                }
                if (input.modifiers.command && input.key_pressed(egui::Key::F))
                    || (!cfg!(target_os = "macos") && input.key_pressed(egui::Key::F3))
                {
                    ctx.memory_mut(|mem| mem.request_focus(egui::Id::new("search")));
                }
                let go_to_folder_pressed = input.modifiers.command
                    && input.key_pressed(egui::Key::L)
                    || cfg!(target_os = "macos")
                        && input.modifiers.command
                        && input.modifiers.shift
                        && input.key_pressed(egui::Key::G)
                    || !cfg!(target_os = "macos")
                        && input.modifiers.alt
                        && input.key_pressed(egui::Key::D);
                if go_to_folder_pressed {
                    self.path_editing = true;
                    self.path_edit_select_all_pending = true;
                    ctx.memory_mut(|mem| mem.request_focus(egui::Id::new("path")));
                }
                if input.modifiers.command
                    && input.modifiers.shift
                    && input.key_pressed(egui::Key::T)
                {
                    self.reopen_closed_tab();
                } else if input.modifiers.command && input.key_pressed(egui::Key::T) {
                    self.open_tab(self.current.clone());
                }
                if input.modifiers.command
                    && input.modifiers.shift
                    && input.key_pressed(egui::Key::N)
                {
                    self.begin_new_folder();
                }
                if input.modifiers.command && input.key_pressed(egui::Key::W) {
                    self.close_tab(self.active_tab);
                }
                let go_back_pressed = if cfg!(target_os = "macos") {
                    input.modifiers.command && input.key_pressed(egui::Key::OpenBracket)
                } else {
                    input.modifiers.alt && input.key_pressed(egui::Key::ArrowLeft)
                };
                if !text_input_focused && go_back_pressed {
                    self.go_back();
                }
                let go_forward_pressed = if cfg!(target_os = "macos") {
                    input.modifiers.command && input.key_pressed(egui::Key::CloseBracket)
                } else {
                    input.modifiers.alt && input.key_pressed(egui::Key::ArrowRight)
                };
                if !text_input_focused && go_forward_pressed {
                    self.go_forward();
                }
                if input.key_pressed(egui::Key::F5) {
                    self.refresh();
                }
                if input.modifiers.alt
                    && input.modifiers.shift
                    && input.key_pressed(egui::Key::P)
                {
                    self.details_open = !self.details_open;
                }
            }
        });
    }

    fn visible_entries(&self) -> &[Entry] {
        if self.query.trim().is_empty() {
            &self.entries
        } else {
            self.search_results
                .as_deref()
                .map_or(&[][..], Vec::as_slice)
        }
    }

    fn drag_transfer(modifiers: egui::Modifiers) -> Transfer {
        let copy_modifier = if cfg!(target_os = "macos") {
            modifiers.alt
        } else {
            modifiers.ctrl
        };
        if copy_modifier {
            Transfer::Copy
        } else {
            Transfer::Move
        }
    }

    fn drag_transfer_to(
        paths: &[PathBuf],
        destination: &Path,
        modifiers: egui::Modifiers,
    ) -> Transfer {
        if Self::drag_transfer(modifiers) == Transfer::Copy {
            return Transfer::Copy;
        }
        if !paths.is_empty()
            && paths
                .iter()
                .all(|path| platform::same_volume(path, destination) == Some(true))
        {
            Transfer::Move
        } else {
            Transfer::Copy
        }
    }

    fn drag_cursor_to(
        paths: &[PathBuf],
        destination: &Path,
        modifiers: egui::Modifiers,
    ) -> egui::CursorIcon {
        match Self::drag_transfer_to(paths, destination, modifiers) {
            Transfer::Copy => egui::CursorIcon::Copy,
            Transfer::Move => egui::CursorIcon::Move,
        }
    }

    fn select_all_visible(&mut self) {
        let paths = self
            .visible_entries()
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        let focused = self
            .focus_path
            .as_ref()
            .filter(|path| paths.contains(path))
            .cloned()
            .or_else(|| paths.first().cloned());
        self.selection_anchor = focused.clone();
        self.focus_path = focused;
        self.selected = paths.into_iter().collect();
    }

    fn move_selection(&mut self, direction: isize, extend: bool) {
        self.type_select_query.clear();
        self.type_select_at = None;
        let entries = self.visible_entries().to_vec();
        if entries.is_empty() {
            return;
        }
        let current = self
            .focus_path
            .as_ref()
            .and_then(|path| entries.iter().position(|entry| &entry.path == path))
            .or_else(|| {
                self.selected
                    .iter()
                    .find_map(|path| entries.iter().position(|entry| &entry.path == path))
            });
        let next = current.map_or_else(
            || if direction >= 0 { 0 } else { entries.len() - 1 },
            |index| (index as isize + direction).clamp(0, entries.len() as isize - 1) as usize,
        );
        self.set_keyboard_selection(&entries, next, extend, current);
    }

    fn move_selection_to_edge(&mut self, end: bool, extend: bool) {
        self.type_select_query.clear();
        self.type_select_at = None;
        let entries = self.visible_entries().to_vec();
        if entries.is_empty() {
            return;
        }
        let current = self
            .focus_path
            .as_ref()
            .and_then(|path| entries.iter().position(|entry| &entry.path == path));
        let next = if end { entries.len() - 1 } else { 0 };
        self.set_keyboard_selection(&entries, next, extend, current);
    }

    fn set_keyboard_selection(
        &mut self,
        entries: &[Entry],
        index: usize,
        extend: bool,
        current: Option<usize>,
    ) {
        let anchor = if extend {
            self.selection_anchor
                .as_ref()
                .and_then(|path| entries.iter().position(|entry| &entry.path == path))
                .or(current)
                .unwrap_or(index)
        } else {
            index
        };
        self.selected.clear();
        for entry in &entries[anchor.min(index)..=anchor.max(index)] {
            self.selected.insert(entry.path.clone());
        }
        self.selection_anchor = Some(entries[anchor].path.clone());
        self.focus_path = Some(entries[index].path.clone());
        if self.view_mode == ViewMode::Icons {
            self.icon_scroll_target = self.focus_path.clone();
        }
    }

    fn open_focused_entry(&mut self) {
        let Some(path) = self
            .focus_path
            .clone()
            .or_else(|| self.selected.iter().next().cloned())
        else {
            return;
        };
        let Some(entry) = self.visible_entries().iter().find(|entry| entry.path == path) else {
            return;
        };
        if entry.is_dir {
            self.navigate(path);
        } else if let Err(error) = opener::open(&path) {
            self.message = format!("Could not open file: {error}");
        }
    }

    fn open_focused_in_new_tab(&mut self) {
        let Some(path) = self
            .focus_path
            .clone()
            .or_else(|| self.selected.iter().next().cloned())
        else {
            return;
        };
        if self
            .visible_entries()
            .iter()
            .any(|entry| entry.path == path && entry.is_dir)
        {
            self.open_tab(path);
        }
    }

    fn handle_events(&mut self) {
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                Event::Listed(generation, path, result)
                    if path == self.current
                        && generation.is_none_or(|generation| generation == self.listing_generation) =>
                match result {
                    Ok(mut entries) => {
                        core::sort_entries(&mut entries, self.sort, self.sort_descending);
                        if self.query.trim().is_empty() {
                            let restoring_tab = self
                                .pending_tab_selection
                                .as_ref()
                                .is_some_and(|(restore_path, _, _)| restore_path == &path);
                            if restoring_tab {
                                let (_, selected, focus_path) =
                                    self.pending_tab_selection.take().unwrap();
                                let available = entries
                                    .iter()
                                    .map(|entry| entry.path.clone())
                                    .collect::<HashSet<_>>();
                                self.selected = selected
                                    .into_iter()
                                    .filter(|path| available.contains(path))
                                    .collect();
                                self.focus_path = focus_path
                                    .filter(|path| available.contains(path))
                                    .or_else(|| {
                                        entries
                                            .iter()
                                            .find(|entry| self.selected.contains(&entry.path))
                                            .map(|entry| entry.path.clone())
                                    });
                                self.selection_anchor = self
                                    .focus_path
                                    .as_ref()
                                    .filter(|path| self.selected.contains(*path))
                                    .cloned()
                                    .or_else(|| {
                                        entries
                                            .iter()
                                            .find(|entry| self.selected.contains(&entry.path))
                                            .map(|entry| entry.path.clone())
                                    });
                                self.icon_scroll_target = self.focus_path.clone();
                            } else {
                                self.retain_selection(&entries);
                            }
                        }
                        self.entries = Arc::new(entries);
                        if self.query.trim().is_empty() {
                            self.message = item_count(self.entries.len());
                        }
                    }
                    Err(error) if self.query.trim().is_empty() => {
                        if self
                            .pending_tab_selection
                            .as_ref()
                            .is_some_and(|(restore_path, _, _)| restore_path == &path)
                        {
                            self.pending_tab_selection = None;
                        }
                        self.message = format!("Error: {error}");
                    }
                    Err(_) => {}
                },
                Event::Searched(generation, query, result)
                    if generation == self.search_generation && query == self.query =>
                match result {
                    Ok(entries) => {
                        let restoring_tab = self
                            .pending_tab_selection
                            .as_ref()
                            .is_some_and(|(restore_path, _, _)| restore_path == &self.current);
                        if restoring_tab {
                            let (_, selected, focus_path) =
                                self.pending_tab_selection.take().unwrap();
                            let available = entries
                                .iter()
                                .map(|entry| entry.path.clone())
                                .collect::<HashSet<_>>();
                            self.selected = selected
                                .into_iter()
                                .filter(|path| available.contains(path))
                                .collect();
                            self.focus_path = focus_path
                                .filter(|path| available.contains(path))
                                .or_else(|| {
                                    entries
                                        .iter()
                                        .find(|entry| self.selected.contains(&entry.path))
                                        .map(|entry| entry.path.clone())
                                });
                            self.selection_anchor = self
                                .focus_path
                                .as_ref()
                                .filter(|path| self.selected.contains(*path))
                                .cloned()
                                .or_else(|| {
                                    entries
                                        .iter()
                                        .find(|entry| self.selected.contains(&entry.path))
                                        .map(|entry| entry.path.clone())
                                });
                            self.icon_scroll_target = self.focus_path.clone();
                        } else {
                            self.retain_selection(&entries);
                        }
                        self.search_results = Some(Arc::new(entries));
                        self.message = search_result_count(
                            self.search_results.as_ref().map_or(0, |results| results.len()),
                        );
                    }
                    Err(error) => {
                        if self
                            .pending_tab_selection
                            .as_ref()
                            .is_some_and(|(restore_path, _, _)| restore_path == &self.current)
                        {
                            self.pending_tab_selection = None;
                        }
                        self.message = format!("Search error: {error}");
                    }
                },
                Event::Finished(
                    message,
                    clear_after,
                    select_after,
                    selected_before,
                    focus_before,
                ) => {
                    self.active_operations = self.active_operations.saturating_sub(1);
                    self.message = message;
                    if let Some(paths) = clear_after
                        && self.cut
                        && self.clipboard == paths
                    {
                        self.clipboard.clear();
                        self.cut = false;
                    }
                    let visible_entries = self.visible_entries();
                    let expected_selection = visible_entries
                        .iter()
                        .filter(|entry| selected_before.contains(&entry.path))
                        .map(|entry| entry.path.clone())
                        .collect::<HashSet<_>>();
                    let expected_focus = focus_before
                        .filter(|path| expected_selection.contains(path))
                        .or_else(|| {
                            visible_entries
                                .iter()
                                .find(|entry| expected_selection.contains(&entry.path))
                                .map(|entry| entry.path.clone())
                        });
                    if let Some(paths) = select_after
                        && paths.iter().all(|path| path.parent() == Some(self.current.as_path()))
                        && self.selected == expected_selection
                        && self.focus_path == expected_focus
                    {
                        let selected_paths = paths
                            .into_iter()
                            .filter(|path| visible_entries.iter().any(|entry| entry.path == *path))
                            .collect::<Vec<_>>();
                        if let Some(focused) = selected_paths.first().cloned() {
                            self.selected = selected_paths.into_iter().collect();
                            self.selection_anchor = Some(focused.clone());
                            self.focus_path = Some(focused.clone());
                            if self.view_mode == ViewMode::Icons {
                                self.icon_scroll_target = Some(focused);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

impl eframe::App for Browser {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.handle_events();
        self.refresh_clipboard_state();
        self.on_key(ctx);
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(9.0, 5.0);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(4);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(4);
        style.visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        style.visuals.widgets.hovered.bg_fill = if style.visuals.dark_mode {
            egui::Color32::from_gray(52)
        } else {
            egui::Color32::from_gray(235)
        };
        style.visuals.widgets.active.bg_fill = if style.visuals.dark_mode {
            egui::Color32::from_gray(65)
        } else {
            egui::Color32::from_gray(222)
        };
        ctx.set_style(style);

        let external_drop = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        if !external_drop.is_empty() {
            let destination = self
                .external_drop_target
                .take()
                .unwrap_or_else(|| self.current.clone());
            self.request_drop(external_drop, destination, Transfer::Copy);
        }
        self.external_drop_target = None;

        self.show_tabs(ctx);
        self.show_command_bar(ctx);
        self.show_address_bar(ctx);
        self.show_status_bar(ctx);

        let inner_width = ctx
            .input(|input| input.viewport().inner_rect.map(|rect| rect.width()))
            .unwrap_or(f32::from(self.window_width));
        let main_pane_min_width = 320.0;
        let navigation_min_width = 175.0;
        let details_min_width = 220.0;
        let navigation_max_width = (inner_width
            - main_pane_min_width
            - if self.details_open {
                details_min_width
            } else {
                0.0
            })
        .clamp(navigation_min_width, 320.0);
        let mut navigation_panel_width = 0.0;
        if self.navigation_open {
            let navigation_id = egui::Id::new("navigation");
            restore_panel_width(
                ctx,
                navigation_id,
                true,
                self.navigation_width,
                navigation_max_width,
            );
            let navigation_panel = egui::SidePanel::left("navigation")
                .default_width(self.navigation_width)
                .width_range(navigation_min_width..=navigation_max_width)
                .resizable(true)
                .show(ctx, |ui| self.show_navigation(ui));
            navigation_panel_width = navigation_panel.response.rect.width();
            if let Some(response) = ctx.read_response(navigation_id.with("__resize")) {
                if response.dragged() {
                    self.navigation_width = navigation_panel_width;
                }
                response.on_hover_text("Drag to resize Navigation pane");
            }
        }

        if self.details_open {
            let details_max_width = (inner_width
                - main_pane_min_width
                - navigation_panel_width)
            .clamp(details_min_width, 360.0);
            let details_id = egui::Id::new("details");
            restore_panel_width(
                ctx,
                details_id,
                false,
                self.details_width,
                details_max_width,
            );
            let details_panel = egui::SidePanel::right("details")
                .default_width(self.details_width)
                .width_range(details_min_width..=details_max_width)
                .resizable(true)
                .show(ctx, |ui| self.show_details(ui));
            if let Some(response) = ctx.read_response(details_id.with("__resize")) {
                if response.dragged() {
                    self.details_width = details_panel.response.rect.width();
                }
                response.on_hover_text("Drag to resize Details pane");
            }
        }

        let current_folder = folder_name(&self.current);
        let window_title = if cfg!(target_os = "macos") {
            current_folder
        } else {
            format!("{current_folder} — File Browser")
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(window_title));
        egui::CentralPanel::default().show(ctx, |ui| {
            let blank_click = ui.interact(
                ui.available_rect_before_wrap(),
                ui.id().with("file-pane-background"),
                egui::Sense::click(),
            );
            let available_size = ui.available_size();
            let (drop_zone, dropped) = ui.dnd_drop_zone::<InternalDrag, _>(
                egui::Frame::NONE,
                |ui| {
                    ui.set_min_size(available_size);
                    self.show_file_list(ui);
                },
            );
            let external_hovering = ui.input(|input| !input.raw.hovered_files.is_empty());
            if let Some(payload) = drop_zone.response.dnd_hover_payload::<InternalDrag>() {
                if let InternalDrag::Files(paths) = payload.as_ref() {
                    ctx.set_cursor_icon(Self::drag_cursor_to(
                        paths,
                        &self.current,
                        ui.input(|input| input.modifiers),
                    ));
                    let mut stroke = ui.visuals().selection.stroke;
                    stroke.width = 2.0;
                    ui.painter().rect_stroke(
                        drop_zone.response.rect.shrink(1.0),
                        6.0,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                } else {
                    ctx.set_cursor_icon(egui::CursorIcon::NotAllowed);
                }
            }
            if external_hovering && self.external_drop_target.is_none() {
                self.external_drop_target = Some(self.current.clone());
                let mut stroke = ui.visuals().selection.stroke;
                stroke.width = 2.0;
                ui.painter().rect_stroke(
                    drop_zone.response.rect.shrink(1.0),
                    6.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(payload) = dropped
                && let InternalDrag::Files(paths) = payload.as_ref()
            {
                self.request_drop(
                    paths.clone(),
                    self.current.clone(),
                    Self::drag_transfer_to(
                        paths,
                        &self.current,
                        ui.input(|input| input.modifiers),
                    ),
                );
            }
            if blank_click.clicked() {
                self.clear_selection();
            }
            drop_zone.response.context_menu(|ui| {
                if ui
                    .add(
                        egui::Button::new("New folder")
                            .right_text(format!("{}+Shift+N", command_modifier())),
                    )
                    .clicked()
                {
                    self.begin_new_folder();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.clipboard_available,
                        egui::Button::new("Paste")
                            .right_text(format!("{}+V", command_modifier())),
                    )
                    .on_hover_text(format!("Paste into this folder ({}+V)", command_modifier()))
                    .on_disabled_hover_text("No file paths in the clipboard")
                    .clicked()
                {
                    self.paste();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        !self.visible_entries().is_empty(),
                        egui::Button::new("Select all visible items")
                            .right_text(format!("{}+A", command_modifier())),
                    )
                    .clicked()
                {
                    self.select_all_visible();
                    ui.close();
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("Refresh").right_text("F5"))
                    .clicked()
                {
                    self.refresh();
                    ui.close();
                }
            });
        });

        self.show_confirmation(ctx);
        if let Some(size) = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size())) {
            self.window_width = size.x.round().clamp(800.0, 4096.0) as u16;
            self.window_height = size.y.round().clamp(560.0, 2160.0) as u16;
        }
        let preferences = GuiPreferences {
            view_mode: self.view_mode,
            icon_card_size: self.icon_card_size,
            show_hidden: self.show_hidden,
            details_open: self.details_open,
            navigation_open: self.navigation_open,
            recent_locations_open: self.recent_locations_open,
            favorites_open: self.favorites_open,
            locations_open: self.locations_open,
            sort: self.sort,
            sort_descending: self.sort_descending,
            navigation_width: self.navigation_width.round().clamp(175.0, 320.0) as u16,
            details_width: self.details_width.round().clamp(220.0, 360.0) as u16,
            window_width: self.window_width,
            window_height: self.window_height,
        };
        let pointer_down = ctx.input(|input| input.pointer.any_down());
        if preferences != self.saved_preferences && !pointer_down {
            self.saved_preferences = preferences;
            if let Err(error) = save_gui_preferences(preferences) {
                self.message = format!("Could not save view preferences: {error}");
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
    }
}

impl Browser {
    fn show_tabs(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("tabs")
            .exact_height(38.0)
                .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let mut close = None;
                    let mut close_others = None;
                    let mut new_tab = None;
                    let mut reorder = None;
                    let external_hovering = ui.input(|input| !input.raw.hovered_files.is_empty());
                    egui::ScrollArea::horizontal()
                        .id_salt("tab-strip")
                        .max_width((ui.available_width() - 42.0).max(80.0))
                        .auto_shrink([false, true])
                        .stick_to_right(true)
                        .show(ui, |ui| {
                            ui.horizontal_centered(|ui| {
                                for index in 0..self.tabs.len() {
                                    let path = self.tabs[index].path.clone();
                                    let folder_title = folder_name(&path);
                                    let duplicate_title = self.tabs.iter().enumerate().any(
                                        |(other_index, tab)| {
                                            other_index != index
                                                && folder_name(&tab.path) == folder_title
                                        },
                                    );
                                    let title = if duplicate_title {
                                        let parent = path.parent();
                                        let parent_title = parent
                                            .map(folder_name)
                                            .unwrap_or_else(|| path.display().to_string());
                                        let parent_is_ambiguous = self.tabs.iter().enumerate().any(
                                            |(other_index, tab)| {
                                                other_index != index
                                                    && folder_name(&tab.path) == folder_title
                                                    && tab.path.parent() != parent
                                                    && tab.path
                                                        .parent()
                                                        .is_some_and(|other_parent| {
                                                            folder_name(other_parent)
                                                                == parent_title
                                                        })
                                            },
                                        );
                                        let parent_label = if parent_is_ambiguous {
                                            parent
                                                .map(|path| path.display().to_string())
                                                .unwrap_or(parent_title)
                                        } else {
                                            parent_title
                                        };
                                        format!("{folder_title} — {parent_label}")
                                    } else {
                                        folder_title
                                    };
                                    let selected = index == self.active_tab;
                                    let fill = if selected {
                                        ui.visuals().selection.bg_fill.gamma_multiply(0.45)
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };
                                    let mut close_rect = egui::Rect::NOTHING;
                                    let (tab_area, dropped) = ui.dnd_drop_zone::<InternalDrag, _>(
                                        egui::Frame::NONE,
                                        |ui| {
                                            egui::Frame::new()
                                                .fill(fill)
                                                .corner_radius(5.0)
                                                .inner_margin(egui::Margin::symmetric(5, 2))
                                                .show(ui, |ui| {
                                                    ui.horizontal(|ui| {
                                                        paint_navigation_icon(
                                                            ui,
                                                            path == self.home,
                                                            path.parent().is_none(),
                                                            selected,
                                                        );
                                                        ui.add(
                                                            egui::Label::new(title)
                                                                .truncate()
                                                                .selectable(false),
                                                        );
                                                        let (_, rect) = ui.allocate_space(
                                                            egui::vec2(18.0, 22.0),
                                                        );
                                                        close_rect = rect;
                                                    });
                                                });
                                        },
                                    );
                                    let tab_response = ui
                                        .interact(
                                            tab_area.response.rect,
                                            ui.id().with(("tab-interaction", &path)),
                                            egui::Sense::click_and_drag(),
                                        )
                                        .on_hover_text(if cfg!(target_os = "macos") {
                                            format!(
                                                "{}\nDrag to reorder\nHover and click × to close",
                                                path.display()
                                            )
                                        } else {
                                            format!(
                                                "{}\nDrag to reorder\nClick × or middle-click to close",
                                                path.display()
                                            )
                                    });
                                    tab_response.dnd_set_drag_payload(InternalDrag::Tab(index));
                                    tab_response.context_menu(|ui| {
                                        if ui.button("Copy Path").clicked() {
                                            let ctx = ui.ctx().clone();
                                            self.copy_paths_to_text(&ctx, vec![path.clone()]);
                                            ui.close();
                                        }
                                        if ui.button("Open in New Tab").clicked() {
                                            new_tab = Some(path.clone());
                                            ui.close();
                                        }
                                        if ui.button("Close Tab").clicked() {
                                            close = Some(index);
                                            ui.close();
                                        }
                                        if self.tabs.len() > 1
                                            && ui.button("Close Other Tabs").clicked()
                                        {
                                            close_others = Some(index);
                                            ui.close();
                                        }
                                    });
                                    if tab_response.hovered() && !selected {
                                        let stroke = egui::Stroke::new(
                                            1.0,
                                            ui.visuals().widgets.hovered.bg_fill,
                                        );
                                        ui.painter().rect_stroke(
                                            tab_area.response.rect.shrink(0.5),
                                            5.0,
                                            stroke,
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    let close_visible = !cfg!(target_os = "macos")
                                        || tab_response.hovered()
                                        || ui.rect_contains_pointer(close_rect);
                                    let close_clicked = close_visible
                                        && ui
                                            .put(
                                                close_rect,
                                                egui::Button::new("×").small().frame(false),
                                            )
                                            .on_hover_text("Close tab")
                                            .clicked();
                                    if close_clicked || tab_response.middle_clicked() {
                                        close = Some(index);
                                    } else if tab_response.clicked() {
                                        let clicked_close_area = tab_response
                                            .interact_pointer_pos()
                                            .is_some_and(|position| close_rect.contains(position));
                                        if clicked_close_area {
                                            close = Some(index);
                                        } else if !selected {
                                            self.switch_tab(index);
                                        }
                                    }
                                    if let Some(payload) =
                                        tab_area.response.dnd_hover_payload::<InternalDrag>()
                                    {
                                        match payload.as_ref() {
                                            InternalDrag::Files(paths) if path.is_dir() => {
                                                ui.ctx().set_cursor_icon(Self::drag_cursor_to(
                                                    paths,
                                                    path.as_path(),
                                                    ui.input(|input| input.modifiers),
                                                ));
                                                let mut stroke = ui.visuals().selection.stroke;
                                                stroke.width = 2.0;
                                                ui.painter().rect_stroke(
                                                    tab_area.response.rect.shrink(1.0),
                                                    5.0,
                                                    stroke,
                                                    egui::StrokeKind::Inside,
                                                );
                                            }
                                            InternalDrag::Files(_) => {
                                                ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
                                            }
                                            InternalDrag::Tab(source) if *source != index => {
                                                let after = tab_area
                                                    .response
                                                    .interact_pointer_pos()
                                                    .is_some_and(|position| {
                                                        position.x > tab_area.response.rect.center().x
                                                    });
                                                let x = if after {
                                                    tab_area.response.rect.right()
                                                } else {
                                                    tab_area.response.rect.left()
                                                };
                                                let stroke = ui.visuals().selection.stroke;
                                                ui.painter().line_segment(
                                                    [
                                                        egui::pos2(x, tab_area.response.rect.top() + 4.0),
                                                        egui::pos2(x, tab_area.response.rect.bottom() - 4.0),
                                                    ],
                                                    egui::Stroke::new(2.0, stroke.color),
                                                );
                                                ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                                            }
                                            InternalDrag::Tab(_) => ui
                                                .ctx()
                                                .set_cursor_icon(egui::CursorIcon::Grabbing),
                                            InternalDrag::Favorite(_) => ui
                                                .ctx()
                                                .set_cursor_icon(egui::CursorIcon::NotAllowed),
                                        }
                                    }
                                    if external_hovering && tab_area.response.contains_pointer() {
                                        self.external_drop_target = Some(path.clone());
                                        let mut stroke = ui.visuals().selection.stroke;
                                        stroke.width = 2.0;
                                        ui.painter().rect_stroke(
                                            tab_area.response.rect.shrink(1.0),
                                            5.0,
                                            stroke,
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    if let Some(payload) = dropped {
                                        match payload.as_ref() {
                                            InternalDrag::Files(paths) => self.request_drop(
                                                paths.clone(),
                                                path.clone(),
                                                Self::drag_transfer_to(
                                                    paths,
                                                    path.as_path(),
                                                    ui.input(|input| input.modifiers),
                                                ),
                                            ),
                                            InternalDrag::Tab(source) => {
                                                let after = tab_area
                                                    .response
                                                    .interact_pointer_pos()
                                                    .is_some_and(|position| {
                                                        position.x > tab_area.response.rect.center().x
                                                    });
                                                reorder = Some((*source, index, after));
                                            }
                                            InternalDrag::Favorite(_) => {}
                                        }
                                    }
                                }
                            });
                        });
                    if let Some((source, target, after)) = reorder {
                        self.reorder_tab(source, target, after);
                    }
                    if ui
                        .button("+")
                        .on_hover_text(format!(
                            "New tab ({}+T)\nReopen closed tab ({}+Shift+T)",
                            command_modifier(),
                            command_modifier()
                        ))
                        .clicked()
                    {
                        self.open_tab(self.current.clone());
                    }
                    if let Some(path) = new_tab {
                        self.open_tab(path);
                    }
                    if let Some(index) = close_others {
                        self.close_other_tabs(index);
                    }
                    if let Some(index) = close {
                        self.close_tab(index);
                    }
                });
            });
    }

    fn show_command_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("commands")
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.menu_button("+ New", |ui| {
                        if ui
                            .button("Folder")
                            .on_hover_text(format!(
                                "Create a folder ({}+Shift+N)",
                                command_modifier()
                            ))
                            .clicked()
                        {
                            self.begin_new_folder();
                            ui.close();
                        }
                    });
                    ui.separator();
                    let has_selection = !self.selected.is_empty();
                    let can_rename = self.selected_paths().len() == 1;
                    if ui
                        .add_enabled(has_selection, egui::Button::new("Cut"))
                        .on_hover_text(format!(
                            "Cut selected items ({}+X)",
                            command_modifier()
                        ))
                        .on_disabled_hover_text(format!(
                            "Select one or more items to cut ({}+X)",
                            command_modifier()
                        ))
                        .clicked()
                    {
                        self.copy_or_cut(true);
                    }
                    if ui
                        .add_enabled(has_selection, egui::Button::new("Copy"))
                        .on_hover_text(format!(
                            "Copy selected items ({}+C)",
                            command_modifier()
                        ))
                        .on_disabled_hover_text(format!(
                            "Select one or more items to copy ({}+C)",
                            command_modifier()
                        ))
                        .clicked()
                    {
                        self.copy_or_cut(false);
                    }
                    if ui
                        .add_enabled(self.clipboard_available, egui::Button::new("Paste"))
                        .on_hover_text(format!("Paste into this folder ({}+V)", command_modifier()))
                        .on_disabled_hover_text("No file paths in the clipboard")
                        .clicked()
                    {
                        self.paste();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(can_rename, egui::Button::new("Rename"))
                        .on_hover_text(if can_rename {
                            "Rename selected item (F2)"
                        } else {
                            "Select one item to rename (F2)"
                        })
                        .on_disabled_hover_text("Select one item to rename (F2)")
                        .clicked()
                    {
                        self.begin_rename();
                    }
                    if ui
                        .add_enabled(
                            has_selection,
                            egui::Button::new(trash_action_label()),
                        )
                        .on_hover_text(format!(
                            "Move selected items to the {} ({})",
                            platform::trash_name(),
                            delete_key_hint(),
                        ))
                        .on_disabled_hover_text(format!(
                            "Select one or more items to move to the {} ({})",
                            platform::trash_name(),
                            delete_key_hint(),
                        ))
                        .clicked()
                    {
                        self.confirm = Some(Confirm::Delete(self.selected_paths()));
                    }
                    ui.separator();
                    ui.menu_button("Sort", |ui| {
                        self.sort_menu(ui, SortBy::Name, "Name");
                        self.sort_menu(ui, SortBy::Kind, "Type");
                        self.sort_menu(ui, SortBy::Size, "Size");
                        self.sort_menu(ui, SortBy::Modified, "Date modified");
                        ui.separator();
                        if ui
                            .checkbox(&mut self.sort_descending, "Descending")
                            .changed()
                        {
                            self.reload_sorted();
                        }
                    });
                    ui.menu_button("View", |ui| {
                        ui.label(egui::RichText::new("Layout").weak());
                        ui.radio_value(&mut self.view_mode, ViewMode::Details, "Details");
                        ui.radio_value(&mut self.view_mode, ViewMode::List, "List");
                        ui.radio_value(&mut self.view_mode, ViewMode::Icons, "Icons");
                        if self.view_mode == ViewMode::Icons {
                            ui.add(
                                egui::Slider::new(&mut self.icon_card_size, 112..=208)
                                    .step_by(8.0)
                                    .text("Icon size"),
                            )
                            .on_hover_text("Icon size is remembered for this tab");
                        }
                        ui.separator();
                        let hidden_changed = ui
                            .checkbox(&mut self.show_hidden, "Hidden items")
                            .on_hover_text(format!(
                                "Show hidden items ({})",
                                hidden_key_hint()
                            ))
                            .changed();
                        if hidden_changed {
                            self.refresh();
                        }
                        ui.checkbox(&mut self.navigation_open, "Navigation pane");
                        if ui
                            .checkbox(&mut self.details_open, "Details pane")
                            .on_hover_text(format!(
                                "Show or hide the Details pane ({})",
                                details_key_hint()
                            ))
                            .changed()
                        {
                            ui.close();
                        }
                    });
                    ui.separator();
                    ui.menu_button("Select", |ui| {
                        if ui
                            .button("Select all visible items")
                            .on_hover_text(format!(
                                "Select all visible items ({}+A)",
                                command_modifier()
                            ))
                            .clicked()
                        {
                            self.select_all_visible();
                            ui.close();
                        }
                        if ui
                            .add_enabled(has_selection, egui::Button::new("Clear selection"))
                            .on_hover_text("Clear selection (Esc)")
                            .clicked()
                        {
                            self.clear_selection();
                            ui.close();
                        }
                    });
                });
            });
    }

    fn show_address_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("address")
            .exact_height(42.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let compact = ui.available_width() < 720.0;
                    let navigation_button_size = if compact { 24.0 } else { 30.0 };
                    let minimum_path_width = if compact { 48.0 } else { 180.0 };
                    let reserved_search_width = if compact { 190.0 } else { 330.0 };
                    let minimum_search_width = if compact { 100.0 } else { 180.0 };
                    let tab = &self.tabs[self.active_tab];
                    let can_back = tab.history_index > 0;
                    let can_forward = tab.history_index + 1 < tab.history.len();
                    let (sidebar_rect, sidebar_response) = ui.allocate_exact_size(
                        egui::vec2(navigation_button_size, 28.0),
                        egui::Sense::click(),
                    );
                    let icon_rect = sidebar_rect.shrink(7.0);
                    let icon_stroke = egui::Stroke::new(1.2, ui.visuals().text_color());
                    ui.painter().rect_stroke(
                        icon_rect,
                        2.0,
                        icon_stroke,
                        egui::StrokeKind::Inside,
                    );
                    let divider_x = icon_rect.left() + icon_rect.width() * 0.38;
                    ui.painter().line_segment(
                        [
                            egui::pos2(divider_x, icon_rect.top() + 1.0),
                            egui::pos2(divider_x, icon_rect.bottom() - 1.0),
                        ],
                        icon_stroke,
                    );
                    if self.navigation_open {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_max(
                                icon_rect.left_top(),
                                egui::pos2(divider_x, icon_rect.bottom()),
                            )
                            .shrink(1.0),
                            1.0,
                            ui.visuals().selection.bg_fill,
                        );
                    }
                    let sidebar_response = sidebar_response.on_hover_text(if self.navigation_open {
                        "Hide navigation pane"
                    } else {
                        "Show navigation pane"
                    });
                    if sidebar_response.clicked() {
                        self.navigation_open = !self.navigation_open;
                    }
                    if ui
                        .add_enabled(
                            can_back,
                            egui::Button::new("←")
                                .min_size(egui::vec2(navigation_button_size, 28.0))
                                .frame(false),
                        )
                        .on_hover_text(format!("Back ({}, mouse side button)", back_key_hint()))
                        .clicked()
                    {
                        self.go_back();
                    }
                    if ui
                        .add_enabled(
                            can_forward,
                            egui::Button::new("→")
                                .min_size(egui::vec2(navigation_button_size, 28.0))
                                .frame(false),
                        )
                        .on_hover_text(format!(
                            "Forward ({}, mouse side button)",
                            forward_key_hint()
                        ))
                        .clicked()
                    {
                        self.go_forward();
                    }
                    let parent = core::parent(&self.current);
                    if ui
                        .add_enabled(
                            parent.is_some(),
                            egui::Button::new("↑")
                                .min_size(egui::vec2(navigation_button_size, 28.0))
                                .frame(false),
                        )
                        .on_hover_text(if parent.is_some() {
                            format!("Up one folder ({})", up_key_hint())
                        } else {
                            "This is the top-level folder".to_owned()
                        })
                        .clicked()
                        && let Some(parent) = parent
                    {
                        let selected = self.current.clone();
                        self.navigate_selecting(parent, selected);
                    }
                    if ui
                        .add(
                            egui::Button::new("↻")
                                .min_size(egui::vec2(navigation_button_size, 28.0))
                                .frame(false),
                        )
                        .on_hover_text("Refresh (F5)")
                        .clicked()
                    {
                        self.refresh();
                    }
                    ui.separator();

                    if self.path_editing {
                        let output = egui::TextEdit::singleline(&mut self.path_edit)
                            .id(egui::Id::new("path"))
                            .desired_width(
                                (ui.available_width() - reserved_search_width)
                                    .max(minimum_path_width),
                            )
                            .show(ui);
                        let response = output.response;
                        if self.path_edit_select_all_pending {
                            response.request_focus();
                            let mut state = output.state;
                            state.cursor.set_char_range(Some(
                                egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(self.path_edit.chars().count()),
                                ),
                            ));
                            state.store(ui.ctx(), response.id);
                            self.path_edit_select_all_pending = false;
                        }
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            self.path_editing = false;
                            let path = core::resolve_path_input(
                                &self.path_edit,
                                &self.current,
                                &self.home,
                            );
                            self.navigate(path);
                        }
                        if ui.button("Go").clicked() {
                            self.path_editing = false;
                            let path = core::resolve_path_input(
                                &self.path_edit,
                                &self.current,
                                &self.home,
                            );
                            self.navigate(path);
                        }
                    } else {
                        egui::ScrollArea::horizontal()
                            .max_width(
                                (ui.available_width() - reserved_search_width)
                                    .max(minimum_path_width),
                            )
                            .stick_to_right(true)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for (index, (label, path)) in
                                        breadcrumbs(&self.current).into_iter().enumerate()
                                    {
                                        if index > 0 {
                                            ui.label(egui::RichText::new("›").weak());
                                        }
                                        let breadcrumb_label = if path == self.current {
                                            egui::RichText::new(label).strong()
                                        } else {
                                            egui::RichText::new(label)
                                        };
                                        let breadcrumb = ui
                                            .small_button(breadcrumb_label)
                                            .on_hover_text(if path == self.current {
                                                format!(
                                                    "Click to edit address\n{}",
                                                    path.display()
                                                )
                                            } else {
                                                path.display().to_string()
                                            });
                                        if breadcrumb.clicked() {
                                            if path == self.current {
                                                self.path_editing = true;
                                                self.path_edit_select_all_pending = true;
                                                ui.ctx().memory_mut(|memory| {
                                                    memory.request_focus(egui::Id::new("path"));
                                                });
                                            } else {
                                                self.navigate(path.clone());
                                            }
                                        }
                                        breadcrumb.context_menu(|ui| {
                                            if path == self.current
                                                && ui.button("Edit Address").clicked()
                                            {
                                                self.path_editing = true;
                                                self.path_edit_select_all_pending = true;
                                                ui.ctx().memory_mut(|memory| {
                                                    memory.request_focus(egui::Id::new("path"));
                                                });
                                                ui.close();
                                            }
                                            if ui.button("Open in New Tab").clicked() {
                                                self.open_tab(path.clone());
                                                ui.close();
                                            }
                                            if ui.button("Copy Path").clicked() {
                                                let ctx = ui.ctx().clone();
                                                self.copy_paths_to_text(&ctx, vec![path.clone()]);
                                                ui.close();
                                            }
                                        });
                                    }
                                });
                            });
                    }

                    let pinned = self.favorites.contains(&self.current);
                    if ui
                        .button(if pinned { "★" } else { "☆" })
                        .on_hover_text(if pinned {
                            "Remove this folder from Favorites"
                        } else {
                            "Add this folder to Favorites"
                        })
                        .clicked()
                    {
                        self.toggle_favorite(self.current.clone());
                    }

                    ui.separator();
                    let clear_width = if self.query.is_empty() { 0.0 } else { 28.0 };
                    let search_width =
                        (ui.available_width() - clear_width).max(minimum_search_width);
                    let search = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .id(egui::Id::new("search"))
                            .desired_width(search_width)
                            .hint_text(format!("Search {}", folder_name(&self.current))),
                    ).on_hover_text(if cfg!(target_os = "macos") {
                        "Search this folder (⌘+F)"
                    } else {
                        "Search this folder (Ctrl+F or F3)"
                    });
                    if search.changed() {
                        self.search();
                    }
                    if !self.query.is_empty()
                        && ui
                            .small_button("×")
                            .on_hover_text("Clear search (Esc)")
                            .clicked()
                    {
                        self.query.clear();
                        self.search();
                    }
                });
            });
    }

    fn show_navigation(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("navigation-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(7.0);
                ui.heading("Browse");
                ui.add_space(5.0);
                self.navigation_item(ui, "Home", self.home.clone(), false, false, false);
                let recent_locations = self
                    .recent_locations
                    .iter()
                    .filter(|path| !self.favorites.contains(path))
                    .cloned()
                    .collect::<Vec<_>>();
                if !recent_locations.is_empty() {
                    ui.separator();
                    let (header, clear) = navigation_section_header(
                        ui,
                        "Recent Locations",
                        self.recent_locations_open,
                        "recent-locations",
                        self.recent_locations_open,
                    );
                    if clear.as_ref().is_some_and(egui::Response::clicked) {
                        self.clear_recent();
                    } else if header.clicked() {
                        self.recent_locations_open = !self.recent_locations_open;
                    }
                    if self.recent_locations_open {
                        for path in &recent_locations {
                            let label = navigation_location_label(path, &recent_locations);
                            self.navigation_item(ui, &label, path.clone(), false, true, false);
                        }
                    }
                }
                ui.separator();
                let (header, _) = navigation_section_header(
                    ui,
                    "Favorites",
                    self.favorites_open,
                    "favorites",
                    false,
                );
                if header.clicked() {
                    self.favorites_open = !self.favorites_open;
                }
                if self.favorites_open {
                    let favorites = self.favorites.clone();
                    for path in &favorites {
                        let label = navigation_location_label(path, &favorites);
                        self.navigation_item(ui, &label, path.clone(), true, false, false);
                    }
                }
                ui.separator();
                let (header, _) = navigation_section_header(
                    ui,
                    "Locations",
                    self.locations_open,
                    "locations",
                    false,
                );
                if header.clicked() {
                    self.locations_open = !self.locations_open;
                }
                if self.locations_open {
                    let locations = platform::locations();
                    for path in &locations {
                        let label = navigation_location_label(path, &locations);
                        self.navigation_item(ui, &label, path.clone(), false, false, true);
                    }
                }
            });
    }

    fn navigation_item(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        path: PathBuf,
        favorite: bool,
        recent: bool,
        location: bool,
    ) {
        let selected = self.current == path;
        let available = path.is_dir();
        let item_label = if available {
            egui::RichText::new(label)
        } else {
            egui::RichText::new(format!("{label} (Unavailable)")).weak()
        };
        let external_hovering = ui.input(|input| !input.raw.hovered_files.is_empty());
        let row_rect = egui::Rect::from_min_size(
            ui.next_widget_position(),
            egui::vec2(ui.available_width(), 28.0),
        );
        let row_fill = if selected {
            ui.visuals().selection.bg_fill
        } else if ui.rect_contains_pointer(row_rect) {
            ui.visuals().widgets.hovered.bg_fill
        } else {
            egui::Color32::TRANSPARENT
        };
        let frame = egui::Frame::new()
            .fill(row_fill)
            .stroke(egui::Stroke::NONE)
            .corner_radius(4)
            .inner_margin(egui::Margin::ZERO);
        let (item_zone, dropped) = ui.dnd_drop_zone::<InternalDrag, _>(
            frame,
            |ui| {
                ui.horizontal(|ui| {
                    let is_home = path == self.home;
                    let is_drive = location || (!is_home && path.parent().is_none());
                    paint_navigation_icon(ui, is_home, is_drive, selected);
                    let target_width = ui.available_width();
                    let button = egui::Button::selectable(selected, item_label)
                        .truncate()
                        .frame(false)
                        .right_text("");
                    let item = ui.add_sized(egui::vec2(target_width, 28.0), button);
                    item
                })
            },
        );
        let row_response = item_zone.inner.response.interact(if favorite {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::click()
        });
        let response = row_response
            .union(item_zone.inner.inner.clone())
            .on_hover_text(if available {
                path.display().to_string()
            } else {
                format!("Unavailable folder\n{}", path.display())
            });
        if favorite {
            response.dnd_set_drag_payload(InternalDrag::Favorite(path.clone()));
        }
        let drop_response = item_zone.response;
        if let Some(payload) = drop_response.dnd_hover_payload::<InternalDrag>() {
            match payload.as_ref() {
                InternalDrag::Files(paths) if available => {
                    ui.ctx().set_cursor_icon(Self::drag_cursor_to(
                        paths,
                        &path,
                        ui.input(|input| input.modifiers),
                    ));
                    let mut stroke = ui.visuals().selection.stroke;
                    stroke.width = 2.0;
                    ui.painter().rect_stroke(
                        drop_response.rect.shrink(1.0),
                        4.0,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                InternalDrag::Files(_) => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
                }
                InternalDrag::Favorite(source) if favorite && source != &path => {
                    let after = drop_response
                        .interact_pointer_pos()
                        .is_some_and(|position| position.y > drop_response.rect.center().y);
                    let y = if after {
                        drop_response.rect.bottom()
                    } else {
                        drop_response.rect.top()
                    };
                    let stroke = ui.visuals().selection.stroke;
                    ui.painter().line_segment(
                        [
                            egui::pos2(drop_response.rect.left() + 4.0, y),
                            egui::pos2(drop_response.rect.right() - 4.0, y),
                        ],
                        egui::Stroke::new(2.0, stroke.color),
                    );
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                }
                InternalDrag::Favorite(source) if favorite && source == &path => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                InternalDrag::Favorite(_) => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
                }
                InternalDrag::Tab(_) => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
                }
            }
        }
        if external_hovering && path.is_dir() && drop_response.contains_pointer() {
            self.external_drop_target = Some(path.clone());
            let mut stroke = ui.visuals().selection.stroke;
            stroke.width = 2.0;
            ui.painter().rect_stroke(
                drop_response.rect.shrink(1.0),
                4.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        if let Some(payload) = dropped {
            match payload.as_ref() {
                InternalDrag::Files(paths) if path.is_dir() => self.request_drop(
                    paths.clone(),
                    path.clone(),
                    Self::drag_transfer_to(
                        paths,
                        &path,
                        ui.input(|input| input.modifiers),
                    ),
                ),
                InternalDrag::Files(_) => {
                    self.message = "Drop destination is no longer available".into();
                }
                InternalDrag::Favorite(source) if favorite && source != &path => {
                    let after = drop_response
                        .interact_pointer_pos()
                        .is_some_and(|position| position.y > drop_response.rect.center().y);
                    self.reorder_favorite(source, &path, after);
                }
                InternalDrag::Favorite(_) | InternalDrag::Tab(_) => {}
            }
        }
        response.context_menu(|ui| {
            if ui.button("Copy Path").clicked() {
                let ctx = ui.ctx().clone();
                self.copy_paths_to_text(&ctx, vec![path.clone()]);
                ui.close();
            }
            if path.is_dir() && ui.button("Open in New Tab").clicked() {
                self.open_tab(path.clone());
                ui.close();
            }
            if favorite && ui.button("Remove from Favorites").clicked() {
                self.toggle_favorite(path.clone());
                ui.close();
            }
            if recent && ui.button("Remove from Recent Locations").clicked() {
                self.remove_recent(&path);
                ui.close();
            }
        });
        if response.clicked() {
            self.navigate(path);
        }
    }

    fn show_details(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("details-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(7.0);
                ui.horizontal(|ui| {
                    ui.heading("Details");
                    if ui.small_button("×").clicked() {
                        self.details_open = false;
                    }
                });
                ui.separator();
                let entries = self.search_results.as_ref().unwrap_or(&self.entries);
                let selected_entries = entries
                    .iter()
                    .filter(|entry| self.selected.contains(&entry.path))
                    .cloned()
                    .collect::<Vec<_>>();
                if selected_entries.len() > 1 {
                    ui.label(
                        egui::RichText::new(format!("{} items selected", selected_entries.len()))
                            .strong()
                            .size(16.0),
                    );
                    ui.add_space(8.0);
                    let file_entries = selected_entries
                        .iter()
                        .filter(|entry| !entry.is_dir)
                        .collect::<Vec<_>>();
                    if file_entries.is_empty() {
                        ui.label(
                            egui::RichText::new("Folder sizes are not included in totals.").weak(),
                        );
                    } else {
                        let file_size = file_entries.iter().map(|entry| entry.size).sum::<u64>();
                        ui.label(egui::RichText::new("Total file size").strong());
                        ui.label(display::format_size(file_size));
                        if file_entries.len() < selected_entries.len() {
                            ui.label(
                                egui::RichText::new("Folder sizes are excluded.").weak(),
                            );
                        }
                    }
                    let mut parents = selected_entries.iter().map(|entry| entry.path.parent());
                    if let Some(first_parent) = parents.next()
                        && parents.all(|parent| parent == first_parent)
                    {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Location").strong());
                        if let Some(parent) = first_parent {
                            let parent = parent.to_path_buf();
                            let location = parent.display().to_string();
                            let response = ui
                                .add(
                                    egui::Label::new(
                                        egui::RichText::new(&location)
                                            .color(ui.visuals().hyperlink_color)
                                            .underline(),
                                    )
                                    .truncate()
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_text(format!("Open containing folder\n{location}"));
                            if response.clicked() {
                                self.navigate(parent);
                            }
                        }
                    }
                    let paths = selected_entries
                        .iter()
                        .map(|entry| entry.path.clone())
                        .collect();
                    if ui.button("Copy Paths").clicked() {
                        let ctx = ui.ctx().clone();
                        self.copy_paths_to_text(&ctx, paths);
                    }
                } else if let Some(entry) = selected_entries.first() {
                    ui.horizontal(|ui| {
                        let cut_pending = self.cut && self.clipboard.contains(&entry.path);
                        paint_entry_icon_large(ui, entry, cut_pending, true, 42.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&entry.name).strong().size(16.0),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&entry.name);
                    });
                    ui.label(entry_type(entry));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Location").strong());
                    if let Some(parent) = entry.path.parent() {
                        let parent = parent.to_path_buf();
                        let location = parent.display().to_string();
                        let response = ui
                            .add(
                                egui::Label::new(
                                    egui::RichText::new(&location)
                                        .color(ui.visuals().hyperlink_color)
                                        .underline(),
                                )
                                .truncate()
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_text(format!("Open containing folder\n{location}"));
                        if response.clicked() {
                            self.navigate_selecting(parent, entry.path.clone());
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Size").strong());
                    ui.label(if entry.is_dir {
                        "—".into()
                    } else {
                        display::format_size(entry.size)
                    });
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Date modified").strong());
                    ui.label(format_modified(entry.modified));
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if entry.is_dir {
                            if ui.button("Open folder").clicked() {
                                self.navigate(entry.path.clone());
                            }
                        } else if ui.button("Open").clicked()
                            && let Err(error) = opener::open(&entry.path)
                        {
                            self.message = format!("Could not open file: {error}");
                        }
                        if ui.button("Copy Path").clicked() {
                            let ctx = ui.ctx().clone();
                            self.copy_paths_to_text(&ctx, vec![entry.path.clone()]);
                        }
                    });
                } else {
                    let current_name = folder_name(&self.current);
                    let current_entry = Entry {
                        path: self.current.clone(),
                        name: current_name.clone(),
                        is_dir: true,
                        is_symlink: false,
                        size: 0,
                        modified: None,
                    };
                    ui.label(
                        egui::RichText::new("Current folder")
                            .strong()
                            .size(16.0),
                    );
                    ui.horizontal(|ui| {
                        paint_entry_icon_large(ui, &current_entry, false, false, 42.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(current_name).strong().size(16.0),
                            )
                            .truncate(),
                        )
                        .on_hover_text(folder_name(&self.current));
                    });
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Location").strong());
                    let current_path = self.current.display().to_string();
                    ui.add(
                        egui::Label::new(&current_path)
                            .truncate()
                            .selectable(false),
                    )
                    .on_hover_text(&current_path);
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Contents").strong());
                    let contents = if !self.query.trim().is_empty() {
                        self.search_results.as_ref().map_or_else(
                            || "Searching…".to_owned(),
                            |results| search_result_count(results.len()),
                        )
                    } else {
                        item_count(self.entries.len())
                    };
                    ui.label(contents);
                    ui.add_space(8.0);
                    if ui.button("Copy Path").clicked() {
                        let ctx = ui.ctx().clone();
                        self.copy_paths_to_text(&ctx, vec![self.current.clone()]);
                    }
                }
            });
    }

    fn show_entry_context_menu(
        &mut self,
        response: &egui::Response,
        entry: &Entry,
        search_active: bool,
    ) {
        let keyboard_open = self.context_menu_request.as_ref() == Some(&entry.path);
        let open_state = if keyboard_open || response.secondary_clicked() {
            Some(egui::containers::SetOpenCommand::Bool(true))
        } else if response.clicked() {
            Some(egui::containers::SetOpenCommand::Bool(false))
        } else {
            None
        };
        if keyboard_open {
            self.context_menu_request = None;
        }
        egui::containers::Popup::menu(response)
            .open_memory(open_state)
            .show(|ui| {
                if ui
                    .add(egui::Button::new("Open").right_text(if cfg!(target_os = "macos") {
                        "⌘+O"
                    } else {
                        "Enter"
                    }))
                    .clicked()
                {
                    if entry.is_dir {
                        self.navigate(entry.path.clone());
                    } else if let Err(error) = opener::open(&entry.path) {
                        self.message = format!("Could not open file: {error}");
                    }
                    ui.close();
                }
                if search_active
                    && let Some(parent) = entry.path.parent()
                    && ui.button("Open Containing Folder").clicked()
                {
                    self.navigate_selecting(parent.to_path_buf(), entry.path.clone());
                    ui.close();
                }
                if ui.button("Copy Path").clicked() {
                    let ctx = ui.ctx().clone();
                    self.copy_paths_to_text(&ctx, vec![entry.path.clone()]);
                    ui.close();
                }
                if ui
                    .add(
                        egui::Button::new(if cfg!(target_os = "macos") {
                            "Get Info"
                        } else {
                            "Properties"
                        })
                        .right_text(if cfg!(target_os = "macos") {
                            "⌘+I"
                        } else {
                            "Alt+Enter"
                        }),
                    )
                    .on_hover_text(if cfg!(target_os = "macos") {
                        "Show item details (Command+I)"
                    } else {
                        "Show item details (Alt+Enter)"
                })
                .clicked()
                {
                    if !self.selected.contains(&entry.path) {
                        self.selected.clear();
                        self.selected.insert(entry.path.clone());
                        self.selection_anchor = Some(entry.path.clone());
                    }
                    self.focus_path = Some(entry.path.clone());
                    self.details_open = true;
                    ui.close();
                }
                if entry.is_dir {
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.clipboard_available,
                            egui::Button::new("Paste Into Folder"),
                        )
                        .on_hover_text(if self.clipboard_available {
                            "Paste clipboard items into this folder"
                        } else {
                            "No file paths in the clipboard"
                        })
                        .on_disabled_hover_text("No file paths in the clipboard")
                        .clicked()
                    {
                        self.paste_to(entry.path.clone());
                        ui.close();
                    }
                    if ui
                        .add(
                            egui::Button::new("Open in New Tab").right_text(
                                if cfg!(target_os = "macos") {
                                    "⌘+Enter"
                                } else {
                                    "Ctrl+Enter"
                                },
                            ),
                        )
                        .on_hover_text(if cfg!(target_os = "macos") {
                            "Command+Enter"
                        } else {
                            "Ctrl+Enter"
                        })
                        .clicked()
                    {
                        self.open_tab(entry.path.clone());
                        ui.close();
                    }
                    let pinned = self.favorites.contains(&entry.path);
                    if ui
                        .button(if pinned {
                            "Remove from Favorites"
                        } else {
                            "Add to Favorites"
                        })
                        .clicked()
                    {
                        self.toggle_favorite(entry.path.clone());
                        ui.close();
                    }
                }
                ui.separator();
                if ui
                    .add(
                        egui::Button::new("Cut")
                            .right_text(format!("{}+X", command_modifier())),
                    )
                    .clicked()
                {
                    self.selected.insert(entry.path.clone());
                    self.copy_or_cut(true);
                    ui.close();
                }
                if ui
                    .add(
                        egui::Button::new("Copy")
                            .right_text(format!("{}+C", command_modifier())),
                    )
                    .clicked()
                {
                    self.selected.insert(entry.path.clone());
                    self.copy_or_cut(false);
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Rename").right_text(if cfg!(target_os = "macos") {
                        "Return"
                    } else {
                        "F2"
                    }))
                    .clicked()
                {
                    self.begin_rename_path(entry.path.clone());
                    ui.close();
                }
                if ui
                    .add(
                        egui::Button::new(trash_action_label()).right_text(delete_key_hint()),
                    )
                    .clicked()
                {
                    let paths = self.selected_paths();
                    self.confirm = Some(Confirm::Delete(if paths.is_empty() {
                        vec![entry.path.clone()]
                    } else {
                        paths
                    }));
                    ui.close();
                }
            });
    }

    fn show_file_list(&mut self, ui: &mut egui::Ui) {
        let search_active = !self.query.trim().is_empty();
        let search_pending = search_active && self.search_results.is_none();
        let entries = if search_active {
            self.search_results.clone().unwrap_or_default()
        } else {
            self.entries.clone()
        };
        if entries.is_empty() {
            let query = self.query.clone();
            let message = self.message.clone();
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.32).max(30.0));
                if message == "Loading…" || search_pending {
                    ui.spinner();
                    ui.label(if search_pending {
                        "Searching…"
                    } else {
                        "Loading folder…"
                    });
                } else if message.starts_with("Search error:") {
                    ui.heading("Search failed");
                    ui.label(message);
                    if ui.button("Try again").clicked() {
                        self.search();
                    }
                } else if message.starts_with("Error:") {
                    ui.heading("Could not open this location");
                    ui.label(message);
                    if ui.button("Retry").clicked() {
                        self.load(self.current.clone());
                    }
                } else if search_active {
                    ui.heading("No results found");
                    ui.label(format!("No items match ‘{query}’"));
                    if ui.button("Clear search").clicked() {
                        self.query.clear();
                        self.search();
                    }
                } else {
                    ui.heading("This folder is empty");
                    ui.label("Create a folder to get started.");
                    if ui.button("New folder").clicked() {
                        self.begin_new_folder();
                    }
                }
            });
            return;
        }
        let modifiers = ui.input(|input| input.modifiers);
        let external_hovering = ui.input(|input| !input.raw.hovered_files.is_empty());
        if self.view_mode == ViewMode::Icons {
            self.show_icon_grid(ui, &entries, modifiers, external_hovering);
            return;
        }
        self.page_rows = ((ui.available_height() - 30.0) / 31.0).floor().max(1.0) as usize;
        let name_heading = if self.sort == SortBy::Name {
            sort_indicator(self.sort_descending, "Name")
        } else {
            "Name".into()
        };
        let modified_heading = if self.sort == SortBy::Modified {
            sort_indicator(self.sort_descending, "Date modified")
        } else {
            "Date modified".into()
        };
        let type_heading = if self.sort == SortBy::Kind {
            sort_indicator(self.sort_descending, "Type")
        } else {
            "Type".into()
        };
        let size_heading = if self.sort == SortBy::Size {
            sort_indicator(self.sort_descending, "Size")
        } else {
            "Size".into()
        };

        let details_view = self.view_mode == ViewMode::Details;
        let available_width = ui.available_width();
        let show_modified = details_view
            && available_width >= if search_active { 900.0 } else { 560.0 };
        let show_type = details_view
            && available_width >= if search_active { 1120.0 } else { 740.0 };
        let show_size = details_view
            && available_width >= if search_active { 1320.0 } else { 900.0 };
        let show_location = details_view && search_active && available_width >= 560.0;
        let has_metadata_columns = show_modified || show_type || show_size || show_location;
        let focused_row = self
            .focus_path
            .as_ref()
            .and_then(|path| entries.iter().position(|entry| &entry.path == path));
        let list_clip_right = ui.clip_rect().right();
        let row_painter = ui.painter().clone();
        let focus_stroke = ui.visuals().selection.stroke;
        let scroll_restore = self.list_scroll_restore.take();
        let mut table = TableBuilder::new(ui)
            .striped(false)
            .resizable(true)
            .sense(egui::Sense::click_and_drag())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(if details_view && has_metadata_columns {
                Column::initial(390.0).resizable(true)
            } else {
                Column::remainder()
            });
        if let Some(offset) = scroll_restore {
            table = table.vertical_scroll_offset(offset);
        } else if let Some(row) = focused_row {
            table = table.scroll_to_row(row, None);
        }
        if show_modified {
            table = table.column(if show_type || show_size || show_location {
                Column::initial(175.0).resizable(true)
            } else {
                Column::remainder()
            });
        }
        if show_type {
            table = table.column(if show_size {
                Column::initial(145.0).resizable(true)
            } else {
                Column::remainder()
            });
        }
        if show_size {
            table = table.column(Column::remainder());
        }
        if show_location {
            table = table.column(Column::remainder());
        }

        let output = table
            .header(30.0, |mut header| {
                header.col(|ui| {
                    if full_cell_button(ui, name_heading)
                        .on_hover_text(sort_hint(
                            SortBy::Name,
                            self.sort,
                            self.sort_descending,
                            "Name",
                        ))
                        .clicked()
                    {
                        self.set_sort(SortBy::Name);
                    }
                });
                if show_modified {
                    header.col(|ui| {
                        if full_cell_button(ui, modified_heading)
                            .on_hover_text(sort_hint(
                                SortBy::Modified,
                                self.sort,
                                self.sort_descending,
                                "Date modified",
                            ))
                            .clicked()
                        {
                            self.set_sort(SortBy::Modified);
                        }
                    });
                }
                if show_type {
                    header.col(|ui| {
                        if full_cell_button(ui, type_heading)
                            .on_hover_text(sort_hint(
                                SortBy::Kind,
                                self.sort,
                                self.sort_descending,
                                "Type",
                            ))
                            .clicked()
                        {
                            self.set_sort(SortBy::Kind);
                        }
                    });
                }
                if show_size {
                    header.col(|ui| {
                        if full_cell_button(ui, size_heading)
                            .on_hover_text(sort_hint(
                                SortBy::Size,
                                self.sort,
                                self.sort_descending,
                                "Size",
                            ))
                            .clicked()
                        {
                            self.set_sort(SortBy::Size);
                        }
                    });
                }
                if show_location {
                    header.col(|ui| {
                        ui.label("Location");
                    });
                }
            })
            .body(|body| {
                body.rows(31.0, entries.len(), |mut row| {
                        let index = row.index();
                        let Some(entry) = entries.get(index) else {
                            return;
                        };
                        let selected = self.selected.contains(&entry.path);
                        let cut_pending = self.cut && self.clipboard.contains(&entry.path);
                        let hover_text = if search_active {
                            entry.path.display().to_string()
                        } else {
                            entry.name.clone()
                        };
                        row.set_selected(selected);
                        let mut full_row_response = None;
                        row.col(|ui| {
                            let row_rect = egui::Rect::from_min_max(
                                ui.max_rect().left_top(),
                                egui::pos2(list_clip_right, ui.max_rect().bottom()),
                            );
                            let row_response = ui.interact(
                                row_rect,
                                ui.id().with(("entry-row", &entry.path)),
                                egui::Sense::click_and_drag(),
                            );
                            if selected {
                                ui.painter().rect_filled(
                                    row_rect,
                                    0.0,
                                    ui.visuals().selection.bg_fill,
                                );
                            } else if row_response.hovered() {
                                ui.painter().rect_filled(
                                    row_rect,
                                    0.0,
                                    ui.visuals().widgets.hovered.bg_fill,
                                );
                            }
                            full_row_response = Some(row_response);
                            ui.horizontal(|ui| {
                                paint_entry_icon(ui, entry, cut_pending, selected);
                                let name = if cut_pending {
                                    egui::RichText::new(&entry.name).weak()
                                } else {
                                    egui::RichText::new(&entry.name)
                                };
                                ui.add(egui::Label::new(name).truncate().selectable(false));
                            });
                        });
                        if show_modified {
                            row.col(|ui| {
                                ui.label(format_modified(entry.modified));
                            });
                        }
                        if show_type {
                            row.col(|ui| {
                                ui.label(entry_type(entry));
                            });
                        }
                        if show_size {
                            row.col(|ui| {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(if entry.is_dir {
                                            String::new()
                                        } else {
                                            display::format_size(entry.size)
                                        });
                                    },
                                );
                            });
                        }
                        if show_location {
                            row.col(|ui| {
                                let location = entry
                                    .path
                                    .parent()
                                    .unwrap_or(Path::new(""))
                                    .display()
                                    .to_string();
                                ui.add(egui::Label::new(&location).truncate().selectable(false))
                                    .on_hover_text(location);
                            });
                        }

                        let response = full_row_response
                            .expect("first row cell creates the full-width interaction")
                            .on_hover_text(format!(
                                "{hover_text}\nRight-click or Shift+F10 for actions"
                            ));
                        response.dnd_set_drag_payload(InternalDrag::Files(
                            self.drag_paths(&entry.path),
                        ));
                        let internal_drop_hovering = response.dnd_hover_payload::<InternalDrag>();
                        if external_hovering && entry.is_dir && response.contains_pointer() {
                            self.external_drop_target = Some(entry.path.clone());
                        }
                        if entry.is_dir
                            && (external_hovering && response.contains_pointer()
                                || internal_drop_hovering
                                    .as_deref()
                                    .is_some_and(|payload| {
                                        matches!(payload, InternalDrag::Files(_))
                                    }))
                        {
                            let mut stroke = focus_stroke;
                            stroke.width = 2.0;
                            row_painter.rect_stroke(
                                response.rect.shrink(1.0),
                                4.0,
                                stroke,
                                egui::StrokeKind::Inside,
                            );
                        }
                        if let Some(InternalDrag::Files(paths)) = response
                            .dnd_release_payload::<InternalDrag>()
                            .as_deref()
                            .cloned()
                        {
                            if entry.is_dir {
                                let action =
                                    Self::drag_transfer_to(&paths, &entry.path, modifiers);
                                self.request_drop(paths, entry.path.clone(), action);
                            } else {
                                self.message = "Drop items onto a folder or empty pane".into();
                            }
                        }
                        if self.focus_path.as_ref() == Some(&entry.path) {
                            let mut stroke = focus_stroke;
                            stroke.width = 2.0;
                            row_painter.rect_stroke(
                                response.rect.shrink(1.0),
                                2.0,
                                stroke,
                                egui::StrokeKind::Inside,
                            );
                        }
                        if let Some(payload) =
                            response.dnd_hover_payload::<InternalDrag>()
                        {
                            match payload.as_ref() {
                                InternalDrag::Files(paths) if entry.is_dir => response
                                    .ctx
                                    .set_cursor_icon(Self::drag_cursor_to(
                                        paths,
                                        &entry.path,
                                        modifiers,
                                    )),
                                _ => response
                                    .ctx
                                    .set_cursor_icon(egui::CursorIcon::NotAllowed),
                            }
                        }
                        if response.secondary_clicked() {
                            if !self.selected.contains(&entry.path) {
                                self.selected.clear();
                                self.selected.insert(entry.path.clone());
                                self.selection_anchor = Some(entry.path.clone());
                            }
                            self.focus_path = Some(entry.path.clone());
                        }
                        if response.clicked() {
                            self.select_row(index, &entries, modifiers);
                        }
                        if response.double_clicked() {
                            if entry.is_dir {
                                self.navigate(entry.path.clone());
                            } else if let Err(error) = opener::open(&entry.path) {
                                self.message = format!("Could not open file: {error}");
                            }
                        }
                        self.show_entry_context_menu(&response, entry, search_active);
                    });
            });
        self.list_scroll_offset = output.state.offset.y;
    }

    fn show_icon_grid(
        &mut self,
        ui: &mut egui::Ui,
        entries: &[Entry],
        modifiers: egui::Modifiers,
        external_hovering: bool,
    ) {
        let search_active = !self.query.trim().is_empty();
        let card_width = f32::from(self.icon_card_size);
        let card_height = card_width - 20.0 + if search_active { 16.0 } else { 0.0 };
        self.page_rows = (ui.available_height() / (card_height + 8.0))
            .floor()
            .max(1.0) as usize;
        let card_size = egui::vec2(card_width, card_height);
        let large_icon_size = card_width * (48.0 / 136.0);
        let columns = ((ui.available_width() + 8.0) / (card_size.x + 8.0))
            .floor()
            .max(1.0) as usize;
        self.icon_columns = columns;
        let total_rows = entries.len().div_ceil(columns);
        let previous_spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        let target_row = self
            .icon_scroll_target
            .take()
            .or_else(|| self.focus_path.clone())
            .and_then(|path| {
                entries
                    .iter()
                    .position(|entry| entry.path == path)
                    .map(|index| index / columns)
            });
        let row_height = card_size.y + 8.0;
        let viewport_height = ui.available_height();

        let mut scroll_area = egui::ScrollArea::vertical()
            .id_salt(("icon-grid", self.current.clone()))
            .auto_shrink([false, false]);
        if let Some(row) = target_row {
            let row_top = row as f32 * row_height;
            let row_bottom = row_top + card_size.y;
            let viewport_bottom = self.icon_scroll_offset + viewport_height;
            if row_top < self.icon_scroll_offset {
                self.icon_scroll_offset = row_top;
                scroll_area = scroll_area.vertical_scroll_offset(self.icon_scroll_offset);
            } else if row_bottom > viewport_bottom {
                self.icon_scroll_offset = (row_bottom - viewport_height).max(0.0);
                scroll_area = scroll_area.vertical_scroll_offset(self.icon_scroll_offset);
            }
        }

        let output = scroll_area.show_rows(ui, card_size.y, total_rows, |ui, row_range| {
                for grid_row in row_range {
                    ui.horizontal(|ui| {
                        for column in 0..columns {
                            let index = grid_row * columns + column;
                            let Some(entry) = entries.get(index) else {
                                break;
                            };
                            let selected = self.selected.contains(&entry.path);
                            let cut_pending = self.cut && self.clipboard.contains(&entry.path);
                            let frame = egui::Frame::new()
                                .fill(egui::Color32::TRANSPARENT)
                                .inner_margin(egui::Margin::ZERO);
                            let (inner, dropped) = ui.dnd_drop_zone::<InternalDrag, _>(
                                frame,
                                |ui| {
                                    let (rect, response) = ui.allocate_exact_size(
                                        card_size,
                                        egui::Sense::click_and_drag(),
                                    );
                                    let visuals = ui.visuals();
                                    let fill = if selected {
                                        visuals.selection.bg_fill.gamma_multiply(0.38)
                                    } else if response.hovered() {
                                        visuals.widgets.hovered.bg_fill
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };
                                    ui.painter().rect_filled(rect, 6.0, fill);
                                    if selected {
                                        ui.painter().rect_stroke(
                                            rect,
                                            6.0,
                                            visuals.selection.stroke,
                                            egui::StrokeKind::Middle,
                                        );
                                    }

                                    let mut content = ui.new_child(
                                        egui::UiBuilder::new()
                                            .max_rect(rect.shrink(5.0))
                                            .layout(egui::Layout::top_down(
                                                egui::Align::Center,
                                            )),
                                    );
                                    content.add_space(2.0);
                                    paint_entry_icon_large(
                                        &mut content,
                                        entry,
                                        cut_pending,
                                        selected,
                                        large_icon_size,
                                    );
                                    content.add_space(4.0);
                                    let name = if cut_pending {
                                        egui::RichText::new(&entry.name).weak()
                                    } else {
                                        egui::RichText::new(&entry.name)
                                    };
                                    content.add(
                                        egui::Label::new(name)
                                            .truncate()
                                            .selectable(false),
                                    );
                                    if search_active {
                                        let location = entry
                                            .path
                                            .parent()
                                            .unwrap_or(Path::new(""))
                                            .display()
                                            .to_string();
                                        content
                                            .add(
                                                egui::Label::new(
                                                    egui::RichText::new(&location).small().weak(),
                                                )
                                                .truncate()
                                                .selectable(false),
                                            )
                                            .on_hover_text(location);
                                    }
                                    response
                                },
                            );
                            let hover_text = if search_active {
                                entry.path.display().to_string()
                            } else {
                                entry.name.clone()
                            };
                            let response = inner.inner.on_hover_text(format!(
                                "{hover_text}\nRight-click or Shift+F10 for actions"
                            ));
                            if self.focus_path.as_ref() == Some(&entry.path) {
                                ui.scroll_to_rect(response.rect, None);
                                let mut stroke = ui.visuals().selection.stroke;
                                stroke.width = if selected { 2.0 } else { 1.0 };
                                ui.painter().rect_stroke(
                                    response.rect.shrink(1.0),
                                    6.0,
                                    stroke,
                                    egui::StrokeKind::Inside,
                                );
                            }
                            response.dnd_set_drag_payload(InternalDrag::Files(
                                self.drag_paths(&entry.path),
                            ));
                            if let Some(payload) =
                                response.dnd_hover_payload::<InternalDrag>()
                            {
                                match payload.as_ref() {
                                    InternalDrag::Files(paths) if entry.is_dir => response
                                        .ctx
                                        .set_cursor_icon(Self::drag_cursor_to(
                                            paths,
                                            &entry.path,
                                            modifiers,
                                        )),
                                    _ => response
                                        .ctx
                                        .set_cursor_icon(egui::CursorIcon::NotAllowed),
                                }
                            }
                            let internal_drop_hovering = entry.is_dir
                                && response
                                    .dnd_hover_payload::<InternalDrag>()
                                    .is_some_and(|payload| {
                                        matches!(payload.as_ref(), InternalDrag::Files(_))
                                    });
                            if external_hovering && entry.is_dir && response.contains_pointer() {
                                self.external_drop_target = Some(entry.path.clone());
                            }
                            if entry.is_dir
                                && ((external_hovering && response.contains_pointer())
                                    || internal_drop_hovering)
                            {
                                let mut stroke = ui.visuals().selection.stroke;
                                stroke.width = 2.0;
                                ui.painter().rect_stroke(
                                    response.rect.shrink(1.0),
                                    6.0,
                                    stroke,
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if let Some(InternalDrag::Files(paths)) =
                                dropped.as_deref().cloned()
                            {
                                if entry.is_dir {
                                    let action = Self::drag_transfer_to(
                                        &paths,
                                        &entry.path,
                                        modifiers,
                                    );
                                    self.request_drop(
                                        paths,
                                        entry.path.clone(),
                                        action,
                                    );
                                } else {
                                    self.message = "Drop items onto a folder or empty pane".into();
                                }
                            }
                            if response.secondary_clicked() {
                                if !selected {
                                    self.selected.clear();
                                    self.selected.insert(entry.path.clone());
                                    self.selection_anchor = Some(entry.path.clone());
                                }
                                self.focus_path = Some(entry.path.clone());
                            }
                            if response.clicked() {
                                self.select_row(index, entries, modifiers);
                            }
                            if response.double_clicked() {
                                if entry.is_dir {
                                    self.navigate(entry.path.clone());
                                } else if let Err(error) = opener::open(&entry.path) {
                                    self.message = format!("Could not open file: {error}");
                                }
                            }
                            self.show_entry_context_menu(&response, entry, search_active);
                        }
                    });
                }
            });
        self.icon_scroll_offset = output.state.offset.y;
        ui.spacing_mut().item_spacing = previous_spacing;
    }

    fn show_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(28.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let search_active = !self.query.trim().is_empty();
                    let entries: &[Entry] = if !search_active {
                        &self.entries
                    } else {
                        self.search_results
                            .as_deref()
                            .map_or(&[][..], Vec::as_slice)
                    };
                    let summary = if search_active {
                        self.search_results.as_ref().map_or_else(
                            || "Searching…".to_owned(),
                            |results| search_result_count(results.len()),
                        )
                    } else {
                        item_count(entries.len())
                    };
                    let (selected_count, selected_file_count, selected_file_size) = entries
                        .iter()
                        .filter(|entry| self.selected.contains(&entry.path))
                        .fold((0, 0, 0_u64), |(count, files, size), entry| {
                            if entry.is_dir {
                                (count + 1, files, size)
                            } else {
                                (count + 1, files + 1, size.saturating_add(entry.size))
                            }
                        });
                    ui.label(&summary);
                    if selected_count > 0 {
                        ui.separator();
                        let label = if selected_file_count > 0 {
                            format!(
                                "{} item{} selected · {}",
                                selected_count,
                                if selected_count == 1 { "" } else { "s" },
                                display::format_size(selected_file_size)
                            )
                        } else {
                            format!(
                                "{} item{} selected",
                                selected_count,
                                if selected_count == 1 { "" } else { "s" }
                            )
                        };
                        let full_selection_label = label.clone();
                        let selection_hint = if selected_count > selected_file_count {
                            format!(
                                "{full_selection_label}\nFolder sizes are not included in the total."
                            )
                        } else {
                            full_selection_label
                        };
                        let font_id = egui::TextStyle::Body.resolve(ui.style());
                        let text_color = ui.visuals().text_color();
                        let label_width = ui
                            .fonts_mut(|fonts| {
                                fonts
                                    .layout_no_wrap(label.clone(), font_id, text_color)
                                    .size()
                                    .x
                            })
                            .min(ui.available_width())
                            .min(280.0);
                        let response = ui.add_sized(
                            egui::vec2(label_width, ui.spacing().interact_size.y),
                            egui::Label::new(egui::RichText::new(&label))
                                .truncate()
                                .selectable(false),
                        );
                        response.on_hover_text(selection_hint);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .selectable_label(self.details_open, "Details")
                            .on_hover_text(format!(
                                "{} the Details pane ({})",
                                if self.details_open { "Hide" } else { "Show" },
                                details_key_hint()
                            ))
                            .clicked()
                        {
                            self.details_open = !self.details_open;
                        }
                        if self.message != summary {
                            if self.active_operations > 0 || self.message.ends_with('…') {
                                let detail = if self.active_operations > 1 {
                                    format!(
                                        "{} file operations are in progress\n{}",
                                        self.active_operations, self.message
                                    )
                                } else {
                                    self.message.clone()
                                };
                                ui.spinner().on_hover_text(detail);
                            }
                            let status_color = status_message_color(ui, &self.message);
                            let status_text = status_color.map_or_else(
                                || egui::RichText::new(&self.message),
                                |color| egui::RichText::new(&self.message).color(color),
                            );
                            ui.add(egui::Label::new(status_text).truncate())
                                .on_hover_text(&self.message);
                        }
                    });
                });
            });
    }

    fn sort_menu(&mut self, ui: &mut egui::Ui, sort: SortBy, label: &str) {
        let selected = self.sort == sort;
        let hint = sort_hint(sort, self.sort, self.sort_descending, label);
        let text = if selected {
            sort_indicator(self.sort_descending, label)
        } else {
            label.to_owned()
        };
        if ui
            .selectable_label(selected, text)
            .on_hover_text(hint)
            .clicked()
        {
            self.set_sort(sort);
            ui.close();
        }
    }

    fn set_sort(&mut self, sort: SortBy) {
        if self.sort == sort {
            self.sort_descending = !self.sort_descending;
        } else {
            self.sort = sort;
            self.sort_descending = false;
        }
        self.reload_sorted();
    }

    fn reload_sorted(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.sort = self.sort;
            tab.sort_descending = self.sort_descending;
        }
        let entries: &mut Vec<Entry> = Arc::make_mut(&mut self.entries);
        core::sort_entries(entries, self.sort, self.sort_descending);
        if let Some(results) = &mut self.search_results {
            let results: &mut Vec<Entry> = Arc::make_mut(results);
            core::sort_entries(results, self.sort, self.sort_descending);
        }
        if self.view_mode == ViewMode::Icons {
            self.icon_scroll_target = self.focus_path.clone();
        }
    }

    fn select_row(&mut self, index: usize, entries: &[Entry], modifiers: egui::Modifiers) {
        self.type_select_query.clear();
        self.type_select_at = None;
        let Some(entry) = entries.get(index) else {
            return;
        };
        if modifiers.shift {
            let anchor = self
                .selection_anchor
                .as_ref()
                .and_then(|path| entries.iter().position(|item| &item.path == path))
                .unwrap_or(index);
            self.selection_anchor = Some(entries[anchor].path.clone());
            self.selected.clear();
            for item in &entries[anchor.min(index)..=anchor.max(index)] {
                self.selected.insert(item.path.clone());
            }
            self.focus_path = Some(entry.path.clone());
        } else if modifiers.command || modifiers.ctrl {
            if !self.selected.remove(&entry.path) {
                self.selected.insert(entry.path.clone());
            }
            self.selection_anchor = Some(entry.path.clone());
            self.focus_path = Some(entry.path.clone());
        } else {
            self.selected.clear();
            self.selected.insert(entry.path.clone());
            self.selection_anchor = Some(entry.path.clone());
            self.focus_path = Some(entry.path.clone());
        }
    }

    fn clear_selection(&mut self) {
        self.selected.clear();
        self.selection_anchor = None;
        self.focus_path = None;
        self.type_select_query.clear();
        self.type_select_at = None;
    }

    fn begin_rename(&mut self) {
        if let Some(path) = self
            .focus_path
            .as_ref()
            .filter(|path| self.selected.contains(*path))
            .cloned()
            .or_else(|| self.selected.iter().next().cloned())
        {
            self.begin_rename_path(path);
        }
    }

    fn rename_from_keyboard(&mut self) {
        match self.selected_paths().len() {
            1 => self.begin_rename(),
            count if count > 1 => self.message = "Select one item to rename".into(),
            _ => {}
        }
    }

    fn begin_rename_path(&mut self, path: PathBuf) {
        self.selected.clear();
        self.selected.insert(path.clone());
        self.selection_anchor = Some(path.clone());
        self.focus_path = Some(path.clone());
        self.name_edit = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.rename_selection_end = if path.is_file() && path.extension().is_some() {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().chars().count())
                .unwrap_or_else(|| self.name_edit.chars().count())
        } else {
            self.name_edit.chars().count()
        };
        self.rename_focus_pending = true;
        self.confirm = Some(Confirm::Rename(path));
    }

    fn begin_new_folder(&mut self) {
        self.name_edit.clear();
        self.new_folder_focus_pending = true;
        self.confirm = Some(Confirm::NewFolder);
    }

    fn drag_paths(&self, path: &PathBuf) -> Vec<PathBuf> {
        if self.selected.contains(path) {
            self.selected_paths()
        } else {
            vec![path.clone()]
        }
    }

    fn remember_recent(&mut self, path: PathBuf) {
        if !path.is_dir() {
            return;
        }
        self.recent_locations.retain(|recent| recent != &path);
        if !self.favorites.contains(&path) {
            self.recent_locations.insert(0, path);
        }
        self.recent_locations.truncate(8);
        let _ = save_recent_locations(&self.recent_locations);
    }

    fn remove_recent(&mut self, path: &Path) {
        self.recent_locations.retain(|recent| recent != path);
        if let Err(error) = save_recent_locations(&self.recent_locations) {
            self.message = format!("Could not save recent locations: {error}");
        }
    }

    fn clear_recent(&mut self) {
        self.recent_locations.clear();
        if let Err(error) = save_recent_locations(&self.recent_locations) {
            self.message = format!("Could not save recent locations: {error}");
        }
    }

    fn copy_or_cut(&mut self, cut: bool) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.clipboard = paths.clone();
        self.clipboard_available = true;
        self.cut = cut;
        self.message = match platform::write_file_clipboard(&paths, cut) {
            Ok(()) => if cut {
                "Cut to clipboard"
            } else {
                "Copied to clipboard"
            }
            .into(),
            Err(error) => format!("Saved to app clipboard: {error}"),
        };
    }

    fn copy_paths_to_text(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let count = paths.len();
        let text = paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        ctx.copy_text(text);
        self.clipboard.clear();
        self.clipboard_available = false;
        self.cut = false;
        self.message = if count == 1 {
            "Path copied to clipboard".into()
        } else {
            format!("Copied {count} paths to clipboard")
        };
    }

    fn show_confirmation(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        match confirm {
            Confirm::Delete(paths) => {
                let trash_name = platform::trash_name();
                let modal = egui::Modal::new(egui::Id::new("delete-confirmation"))
                    .show(ctx, |ui| {
                        ui.heading(format!("Move to {trash_name}?"));
                        ui.label(if paths.len() == 1 {
                            format!("Move this item to the {trash_name}?")
                        } else {
                            format!("Move these {} items to the {trash_name}?", paths.len())
                        });
                        egui::ScrollArea::vertical()
                            .max_height(160.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for path in &paths {
                                    ui.label(item_name(path))
                                        .on_hover_text(path.display().to_string());
                                }
                            });
                        ui.horizontal(|ui| {
                            if ui.button(format!("Move to {trash_name}")).clicked() {
                                let paths = paths.clone();
                                self.run_mutation(
                                    format!("Moving to {trash_name}"),
                                    None,
                                    None,
                                    move || {
                                    core::delete_to_trash(&paths).map(|_| None)
                                    },
                                );
                            } else if ui.button("Cancel").clicked() {
                                self.confirm = None;
                            } else {
                                self.confirm = Some(Confirm::Delete(paths.clone()));
                            }
                        });
                    });
                if modal.should_close() {
                    self.confirm = None;
                }
            }
            Confirm::Conflict(paths, destination, action) => {
                let conflicts = paths
                    .iter()
                    .filter(|path| {
                        core::path_exists(&destination.join(path.file_name().unwrap_or_default()))
                    })
                    .collect::<Vec<_>>();
                if !conflicts.is_empty() {
                    let modal = egui::Modal::new(egui::Id::new("transfer-conflicts"))
                        .show(ctx, |ui| {
                            ui.heading("Replace existing items?");
                            ui.label(format!(
                                "{} {} item(s) to:",
                                if action == Transfer::Move { "Move" } else { "Copy" },
                                paths.len()
                            ));
                            ui.label(destination.display().to_string());
                            ui.separator();
                            ui.label(format!(
                                "{} destination item(s) already exist:",
                                conflicts.len()
                            ));
                            egui::ScrollArea::vertical()
                                .max_height(160.0)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for path in &conflicts {
                                        ui.label(item_name(path))
                                            .on_hover_text(path.display().to_string());
                                    }
                                });
                            let mut choice = None;
                            ui.horizontal(|ui| {
                                for (label, selected_choice) in [
                                    ("Replace", ConflictChoice::Replace),
                                    ("Skip", ConflictChoice::Skip),
                                    ("Cancel", ConflictChoice::Cancel),
                                ] {
                                    let button = ui.button(label).on_hover_text(
                                        match selected_choice {
                                            ConflictChoice::Replace => {
                                                "Replace existing destination items"
                                            }
                                            ConflictChoice::Skip => {
                                                "Skip conflicts and transfer other items"
                                            }
                                            ConflictChoice::Cancel => "Cancel this transfer",
                                        },
                                    );
                                    if button.clicked() {
                                        choice = Some(selected_choice);
                                    }
                                }
                            });
                            if let Some(choice) = choice {
                                if choice != ConflictChoice::Cancel {
                                    self.apply_transfer(
                                        paths.clone(),
                                        destination.clone(),
                                        action,
                                        choice,
                                    );
                                } else {
                                    self.confirm = None;
                                }
                            } else {
                                self.confirm = Some(Confirm::Conflict(
                                    paths.clone(),
                                    destination.clone(),
                                    action,
                                ));
                            }
                        });
                    if modal.should_close() {
                        self.confirm = None;
                    }
                } else {
                    self.apply_transfer(paths, destination, action, ConflictChoice::Cancel);
                }
            }
            Confirm::NewFolder => {
                let modal = egui::Modal::new(egui::Id::new("new-folder-dialog")).show(ctx, |ui| {
                        ui.heading("New folder");
                        let response = egui::TextEdit::singleline(&mut self.name_edit)
                            .id(egui::Id::new("new-folder-input"))
                            .show(ui)
                            .response;
                        if self.new_folder_focus_pending {
                            response.request_focus();
                            self.new_folder_focus_pending = false;
                        }
                        let submit_by_key = response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        ui.horizontal(|ui| {
                            if ui.button("Create").clicked() || submit_by_key {
                                let path = self.current.clone();
                                let name = self.name_edit.clone();
                                let select_after = path.join(&name);
                                self.confirm = None;
                                self.new_folder_focus_pending = false;
                                self.run_mutation(
                                    "Creating folder",
                                    None,
                                    Some(vec![select_after]),
                                    move || {
                                        core::create_folder(&path, &name)?;
                                        Ok(None)
                                    },
                                );
                            } else if ui.button("Cancel").clicked() {
                                self.confirm = None;
                                self.new_folder_focus_pending = false;
                            } else {
                                self.confirm = Some(Confirm::NewFolder);
                            }
                        });
                    });
                if modal.should_close() {
                    self.confirm = None;
                    self.new_folder_focus_pending = false;
                }
            }
            Confirm::Rename(path) => {
                let modal = egui::Modal::new(egui::Id::new("rename-dialog")).show(ctx, |ui| {
                        ui.heading("Rename");
                        let output = egui::TextEdit::singleline(&mut self.name_edit)
                            .id(egui::Id::new("rename-input"))
                            .show(ui);
                        if self.rename_focus_pending {
                            output.response.request_focus();
                            let mut state = output.state;
                            state.cursor.set_char_range(Some(
                                egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(self.rename_selection_end),
                                ),
                            ));
                            state.store(ui.ctx(), output.response.id);
                            self.rename_focus_pending = false;
                        }
                        let submit_by_key = output.response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        ui.horizontal(|ui| {
                            if ui.button("Rename").clicked() || submit_by_key {
                                let name = self.name_edit.clone();
                                let select_after = path.with_file_name(&name);
                                self.confirm = None;
                                self.rename_focus_pending = false;
                                self.run_mutation(
                                    "Renaming",
                                    None,
                                    Some(vec![select_after]),
                                    move || {
                                        core::rename(&path, &name)?;
                                        Ok(None)
                                    },
                                );
                            } else if ui.button("Cancel").clicked() {
                                self.confirm = None;
                                self.rename_focus_pending = false;
                            } else {
                                self.confirm = Some(Confirm::Rename(path.clone()));
                            }
                        });
                    });
                if modal.should_close() {
                    self.confirm = None;
                    self.rename_focus_pending = false;
                }
            }
        }
    }

    fn apply_transfer(
        &mut self,
        paths: Vec<PathBuf>,
        destination: PathBuf,
        action: Transfer,
        choice: ConflictChoice,
    ) {
        self.confirm = None;
        let clear_after = (action == Transfer::Move
            && paths.iter().all(|path| self.clipboard.contains(path)))
        .then(|| paths.clone());
        let select_after = paths
            .iter()
            .filter(|path| {
                choice != ConflictChoice::Skip
                    || !core::path_exists(
                        &destination.join(path.file_name().unwrap_or_default()),
                    )
            })
            .filter_map(|path| path.file_name().map(|name| destination.join(name)))
            .collect();
        let verb = match action {
            Transfer::Copy => "Copying",
            Transfer::Move => "Moving",
        };
        let target_name = folder_name(&destination);
        let label = if paths.len() == 1 {
            let source_name = paths[0]
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| paths[0].display().to_string());
            format!("{verb} {source_name} to {target_name}")
        } else {
            format!("{verb} {} items to {target_name}", paths.len())
        };
        self.run_mutation(
            label,
            clear_after,
            Some(select_after),
            move || {
                let skipped = core::transfer(&paths, &destination, action, choice)?;
                Ok((!skipped.is_empty())
                    .then(|| format!("Skipped conflicting items: {}", skipped.join(", "))))
            },
        );
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

fn status_message_color(ui: &egui::Ui, message: &str) -> Option<egui::Color32> {
    if message.starts_with("Error:")
        || message.starts_with("Search error:")
        || message.starts_with("Could not ")
        || message.starts_with("Drop destination")
        || message.contains("refresh failed:")
    {
        Some(ui.visuals().error_fg_color)
    } else if message.starts_with("Skipped conflicting items:")
        || message.starts_with("Saved to app clipboard:")
        || message.starts_with("Clipboard has no file paths")
        || message.starts_with("A folder cannot be dropped")
        || message.starts_with("Drop items onto a folder")
    {
        Some(ui.visuals().warn_fg_color)
    } else {
        None
    }
}

fn navigation_location_label(path: &Path, peers: &[PathBuf]) -> String {
    let name = folder_name(path);
    let has_duplicate = peers
        .iter()
        .any(|peer| peer != path && folder_name(peer) == name);
    if !has_duplicate {
        return name;
    }
    let parent = path.parent();
    let parent_name = parent
        .map(folder_name)
        .unwrap_or_else(|| path.display().to_string());
    let parent_is_ambiguous = peers.iter().any(|peer| {
        peer != path
            && folder_name(peer) == name
            && peer.parent() != parent
            && peer
                .parent()
                .is_some_and(|other_parent| folder_name(other_parent) == parent_name)
    });
    let parent_label = if parent_is_ambiguous {
        parent
            .map(|parent| parent.display().to_string())
            .unwrap_or(parent_name)
    } else {
        parent_name
    };
    format!("{name} — {parent_label}")
}

fn item_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn breadcrumbs(path: &Path) -> Vec<(String, PathBuf)> {
    let mut current = PathBuf::new();
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
                parts.push((
                    prefix.as_os_str().to_string_lossy().into_owned(),
                    current.clone(),
                ));
            }
            Component::RootDir => {
                current.push(component.as_os_str());
                if let Some((_, target)) = parts.last_mut() {
                    *target = current.clone();
                } else {
                    parts.push(("/".into(), current.clone()));
                }
            }
            Component::Normal(name) => {
                current.push(name);
                parts.push((name.to_string_lossy().into_owned(), current.clone()));
            }
            Component::CurDir | Component::ParentDir => {
                current.push(component.as_os_str());
                parts.push((
                    component.as_os_str().to_string_lossy().into_owned(),
                    current.clone(),
                ));
            }
        }
    }
    parts
}

fn known_folders() -> Vec<(&'static str, PathBuf)> {
    [
        ("Desktop", dirs::desktop_dir()),
        ("Documents", dirs::document_dir()),
        ("Downloads", dirs::download_dir()),
        ("Pictures", dirs::picture_dir()),
        ("Music", dirs::audio_dir()),
        ("Videos", dirs::video_dir()),
    ]
    .into_iter()
    .filter_map(|(name, path)| path.filter(|path| path.is_dir()).map(|path| (name, path)))
    .collect()
}

fn favorites_file() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("filebrowser").join("favorites.json"))
}

fn load_favorites() -> Vec<PathBuf> {
    let defaults = || {
        known_folders()
            .into_iter()
            .map(|(_, path)| path)
            .collect()
    };
    let Some(path) = favorites_file() else {
        return defaults();
    };
    std::fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_else(defaults)
}

fn save_favorites(favorites: &[PathBuf]) -> anyhow::Result<()> {
    let path = favorites_file()
        .ok_or_else(|| anyhow::anyhow!("configuration folder unavailable"))?;
    let directory = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("configuration folder unavailable"))?;
    std::fs::create_dir_all(directory)?;
    std::fs::write(path, serde_json::to_vec_pretty(favorites)?)?;
    Ok(())
}

fn recent_locations_file() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("filebrowser").join("recent-locations.json"))
}

fn load_recent_locations() -> Vec<PathBuf> {
    let Some(path) = recent_locations_file() else {
        return Vec::new();
    };
    let Ok(data) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(paths) = serde_json::from_slice::<Vec<PathBuf>>(&data) else {
        return Vec::new();
    };
    let mut recent = Vec::new();
    for path in paths {
        if path.is_dir() && !recent.contains(&path) {
            recent.push(path);
        }
        if recent.len() == 8 {
            break;
        }
    }
    recent
}

fn save_recent_locations(paths: &[PathBuf]) -> anyhow::Result<()> {
    let path = recent_locations_file()
        .ok_or_else(|| anyhow::anyhow!("configuration folder unavailable"))?;
    let directory = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("configuration folder unavailable"))?;
    std::fs::create_dir_all(directory)?;
    std::fs::write(path, serde_json::to_vec_pretty(paths)?)?;
    Ok(())
}

fn preferences_file() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join("filebrowser").join("preferences.json"))
}

fn load_gui_preferences() -> GuiPreferences {
    let Some(path) = preferences_file() else {
        return GuiPreferences::default();
    };
    let Ok(data) = std::fs::read(path) else {
        return GuiPreferences::default();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
        return GuiPreferences::default();
    };
    let mut preferences = GuiPreferences::default();
    preferences.view_mode = match value.get("view_mode").and_then(|value| value.as_str()) {
        Some("list") => ViewMode::List,
        Some("icons") => ViewMode::Icons,
        _ => ViewMode::Details,
    };
    preferences.icon_card_size = value
        .get("icon_card_size")
        .and_then(|value| value.as_u64())
        .map(|size| size.clamp(112, 208) as u16)
        .unwrap_or(preferences.icon_card_size);
    preferences.show_hidden = value
        .get("show_hidden")
        .and_then(|value| value.as_bool())
        .unwrap_or(preferences.show_hidden);
    preferences.details_open = value
        .get("details_open")
        .and_then(|value| value.as_bool())
        .unwrap_or(preferences.details_open);
    preferences.navigation_open = value
        .get("navigation_open")
        .and_then(|value| value.as_bool())
        .unwrap_or(preferences.navigation_open);
    preferences.recent_locations_open = value
        .get("recent_locations_open")
        .and_then(|value| value.as_bool())
        .unwrap_or(preferences.recent_locations_open);
    preferences.favorites_open = value
        .get("favorites_open")
        .and_then(|value| value.as_bool())
        .unwrap_or(preferences.favorites_open);
    preferences.locations_open = value
        .get("locations_open")
        .and_then(|value| value.as_bool())
        .unwrap_or(preferences.locations_open);
    preferences.sort = match value.get("sort").and_then(|value| value.as_str()) {
        Some("type") => SortBy::Kind,
        Some("size") => SortBy::Size,
        Some("modified") => SortBy::Modified,
        _ => SortBy::Name,
    };
    preferences.sort_descending = value
        .get("sort_descending")
        .and_then(|value| value.as_bool())
        .unwrap_or(preferences.sort_descending);
    preferences.navigation_width = value
        .get("navigation_width")
        .and_then(|value| value.as_u64())
        .map(|width| width.clamp(175, 320) as u16)
        .unwrap_or(preferences.navigation_width);
    preferences.details_width = value
        .get("details_width")
        .and_then(|value| value.as_u64())
        .map(|width| width.clamp(220, 360) as u16)
        .unwrap_or(preferences.details_width);
    preferences.window_width = value
        .get("window_width")
        .and_then(|value| value.as_u64())
        .map(|width| width.clamp(800, 4096) as u16)
        .unwrap_or(preferences.window_width);
    preferences.window_height = value
        .get("window_height")
        .and_then(|value| value.as_u64())
        .map(|height| height.clamp(560, 2160) as u16)
        .unwrap_or(preferences.window_height);
    preferences
}

fn save_gui_preferences(preferences: GuiPreferences) -> anyhow::Result<()> {
    let path = preferences_file()
        .ok_or_else(|| anyhow::anyhow!("configuration folder unavailable"))?;
    let directory = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("configuration folder unavailable"))?;
    std::fs::create_dir_all(directory)?;
    let view_mode = match preferences.view_mode {
        ViewMode::Details => "details",
        ViewMode::List => "list",
        ViewMode::Icons => "icons",
    };
    let sort = match preferences.sort {
        SortBy::Name => "name",
        SortBy::Kind => "type",
        SortBy::Size => "size",
        SortBy::Modified => "modified",
    };
    let data = serde_json::json!({
        "view_mode": view_mode,
        "icon_card_size": preferences.icon_card_size,
        "show_hidden": preferences.show_hidden,
        "details_open": preferences.details_open,
        "navigation_open": preferences.navigation_open,
        "recent_locations_open": preferences.recent_locations_open,
        "favorites_open": preferences.favorites_open,
        "locations_open": preferences.locations_open,
        "sort": sort,
        "sort_descending": preferences.sort_descending,
        "navigation_width": preferences.navigation_width,
        "details_width": preferences.details_width,
        "window_width": preferences.window_width,
        "window_height": preferences.window_height,
    });
    std::fs::write(path, serde_json::to_vec_pretty(&data)?)?;
    Ok(())
}

fn paint_entry_icon(ui: &mut egui::Ui, entry: &Entry, dimmed: bool, selected: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    paint_entry_icon_in_rect(ui, entry, rect, dimmed, selected);
}

fn paint_entry_icon_large(
    ui: &mut egui::Ui,
    entry: &Entry,
    dimmed: bool,
    selected: bool,
    size: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    paint_entry_icon_in_rect(ui, entry, rect, dimmed, selected);
}

fn paint_entry_icon_in_rect(
    ui: &egui::Ui,
    entry: &Entry,
    rect: egui::Rect,
    dimmed: bool,
    selected: bool,
) {
    let painter = ui.painter();
    let scale = rect.width() / 18.0;
    let offset = |x: f32, y: f32| egui::vec2(x * scale, y * scale);
    if entry.is_dir {
        let folder = fade_icon_color(folder_icon_color(ui.visuals().dark_mode), dimmed);
        let body = egui::Rect::from_min_size(
            rect.min + offset(1.0, 6.0),
            egui::vec2(16.0, 10.0) * scale,
        );
        let tab = egui::Rect::from_min_size(
            rect.min + offset(2.0, 3.0),
            egui::vec2(7.0, 5.0) * scale,
        );
        painter.rect_filled(body, 2.0 * scale, folder);
        painter.rect_filled(tab, 1.5 * scale, folder);
        let outline = folder_outline_color(ui.visuals().dark_mode, selected);
        painter.rect_stroke(
            body,
            2.0 * scale,
            egui::Stroke::new(0.8 * scale, outline),
            egui::StrokeKind::Inside,
        );
        painter.rect_stroke(
            tab,
            1.5 * scale,
            egui::Stroke::new(0.8 * scale, outline),
            egui::StrokeKind::Inside,
        );
        if entry.is_symlink {
            let link = fade_icon_color(
                if ui.visuals().dark_mode {
                    egui::Color32::from_rgb(116, 190, 220)
                } else {
                    egui::Color32::from_rgb(35, 126, 164)
                },
                dimmed,
            );
            painter.line_segment(
                [rect.min + offset(10.0, 15.0), rect.min + offset(15.0, 10.0)],
                egui::Stroke::new(1.5 * scale, link),
            );
            painter.line_segment(
                [rect.min + offset(12.0, 10.0), rect.min + offset(15.0, 10.0)],
                egui::Stroke::new(1.5 * scale, link),
            );
            painter.line_segment(
                [rect.min + offset(15.0, 10.0), rect.min + offset(15.0, 13.0)],
                egui::Stroke::new(1.5 * scale, link),
            );
        }
    } else if entry.is_symlink {
        let color = fade_icon_color(if ui.visuals().dark_mode {
            egui::Color32::from_rgb(116, 190, 220)
        } else {
            egui::Color32::from_rgb(35, 126, 164)
        }, dimmed);
        painter.line_segment(
            [
                rect.left_bottom() + offset(4.0, -4.0),
                rect.right_top() + offset(-4.0, 4.0),
            ],
            egui::Stroke::new(1.8 * scale, color),
        );
        painter.line_segment(
            [
                rect.right_top() + offset(-8.0, 4.0),
                rect.right_top() + offset(-4.0, 4.0),
            ],
            egui::Stroke::new(1.8 * scale, color),
        );
    } else {
        let page = rect.shrink(2.0 * scale);
        let fill = fade_icon_color(if ui.visuals().dark_mode {
            egui::Color32::from_gray(48)
        } else {
            egui::Color32::WHITE
        }, dimmed);
        let stroke = fade_icon_color(if ui.visuals().dark_mode {
            egui::Color32::from_gray(175)
        } else {
            egui::Color32::from_gray(125)
        }, dimmed);
        painter.rect_filled(page, 1.5 * scale, fill);
        painter.rect_stroke(
            page,
            1.5 * scale,
            egui::Stroke::new(1.0 * scale, stroke),
            egui::StrokeKind::Middle,
        );
        let accent = fade_icon_color(file_icon_accent(entry, ui.visuals().dark_mode), dimmed);
        for y in [5.0, 8.0, 11.0] {
            painter.line_segment(
                [rect.min + offset(5.0, y), rect.min + offset(13.0, y)],
                egui::Stroke::new(1.0 * scale, accent),
            );
        }
    }
}

fn file_icon_accent(entry: &Entry, dark_mode: bool) -> egui::Color32 {
    let color = entry
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match color.as_deref() {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic") => {
            egui::Color32::from_rgb(155, 105, 205)
        }
        Some("mp4" | "mov" | "mkv" | "avi" | "webm") => {
            egui::Color32::from_rgb(208, 83, 80)
        }
        Some("mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg") => {
            egui::Color32::from_rgb(40, 150, 139)
        }
        Some("zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz") => {
            egui::Color32::from_rgb(188, 126, 56)
        }
        Some("pdf" | "doc" | "docx" | "rtf" | "txt" | "odt") => {
            egui::Color32::from_rgb(59, 124, 197)
        }
        Some("xls" | "xlsx" | "csv" | "ods") => egui::Color32::from_rgb(65, 145, 83),
        Some("rs" | "js" | "ts" | "py" | "toml" | "json" | "html" | "css" | "sh") => {
            egui::Color32::from_rgb(206, 128, 55)
        }
        _ => {
            if dark_mode {
                egui::Color32::from_gray(175)
            } else {
                egui::Color32::from_gray(125)
            }
        }
    }
}

fn fade_icon_color(color: egui::Color32, dimmed: bool) -> egui::Color32 {
    if dimmed {
        color.gamma_multiply(0.55)
    } else {
        color
    }
}

fn navigation_section_header(
    ui: &mut egui::Ui,
    label: &str,
    expanded: bool,
    id: &str,
    show_clear: bool,
) -> (egui::Response, Option<egui::Response>) {
    let (_, rect) = ui.allocate_space(egui::vec2(
        ui.available_width(),
        ui.spacing().interact_size.y,
    ));
    let response = ui.interact(rect, ui.id().with(id), egui::Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 3.0, ui.visuals().widgets.hovered.bg_fill);
    }
    let mut content = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    content.add_space(4.0);
    content.label(if expanded { "▾" } else { "▸" });
    content.label(egui::RichText::new(label).strong());
    let clear = show_clear.then(|| {
        let clear_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 52.0, rect.top()),
            egui::vec2(48.0, rect.height()),
        );
        content
            .put(clear_rect, egui::Button::new("Clear").small())
            .on_hover_text("Clear recent locations")
    });
    (
        response.on_hover_text(if expanded {
            format!("Collapse {label}")
        } else {
            format!("Expand {label}")
        }),
        clear,
    )
}

fn full_cell_button(ui: &mut egui::Ui, label: String) -> egui::Response {
    let rect = ui.max_rect();
    let target = ui.interact(
        rect,
        ui.id().with("full-cell-button"),
        egui::Sense::click(),
    );
    if target.hovered() {
        ui.painter()
            .rect_filled(rect, 3.0, ui.visuals().widgets.hovered.bg_fill);
    }
    let label = ui.add(egui::Button::new(label).frame(false));
    target.union(label)
}

fn paint_navigation_icon(
    ui: &mut egui::Ui,
    home: bool,
    drive: bool,
    selected: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    let painter = ui.painter();
    let color = if ui.visuals().dark_mode {
        egui::Color32::from_gray(170)
    } else {
        egui::Color32::from_gray(105)
    };
    if home {
        let roof_left = rect.left_top() + egui::vec2(2.0, 8.0);
        let roof_top = rect.center_top() + egui::vec2(0.0, 2.0);
        let roof_right = rect.right_top() + egui::vec2(-2.0, 8.0);
        painter.line_segment([roof_left, roof_top], egui::Stroke::new(1.4, color));
        painter.line_segment([roof_top, roof_right], egui::Stroke::new(1.4, color));
        let body = egui::Rect::from_min_max(
            rect.min + egui::vec2(4.0, 7.0),
            rect.max - egui::vec2(4.0, 1.0),
        );
        painter.rect_stroke(
            body,
            1.0,
            egui::Stroke::new(1.2, color),
            egui::StrokeKind::Middle,
        );
        painter.rect_filled(
            egui::Rect::from_min_size(body.center_bottom() - egui::vec2(1.5, 4.0), egui::vec2(3.0, 4.0)),
            0.5,
            color,
        );
    } else if drive {
        let body = egui::Rect::from_min_size(
            rect.min + egui::vec2(1.0, 3.0),
            egui::vec2(14.0, 10.0),
        );
        painter.rect_stroke(
            body,
            2.0,
            egui::Stroke::new(1.2, color),
            egui::StrokeKind::Middle,
        );
        painter.line_segment(
            [
                body.left_bottom() + egui::vec2(1.0, -3.0),
                body.right_bottom() + egui::vec2(-1.0, -3.0),
            ],
            egui::Stroke::new(1.2, color),
        );
        painter.circle_filled(body.right_bottom() + egui::vec2(-3.0, -1.0), 0.8, color);
    } else {
        let folder = folder_icon_color(ui.visuals().dark_mode);
        let body = egui::Rect::from_min_size(
            rect.min + egui::vec2(1.0, 6.0),
            egui::vec2(14.0, 8.0),
        );
        let tab = egui::Rect::from_min_size(
            rect.min + egui::vec2(2.0, 4.0),
            egui::vec2(6.0, 4.0),
        );
        painter.rect_filled(body, 2.0, folder);
        painter.rect_filled(tab, 1.5, folder);
        let outline = folder_outline_color(ui.visuals().dark_mode, selected);
        painter.rect_stroke(
            body,
            2.0,
            egui::Stroke::new(0.9, outline),
            egui::StrokeKind::Inside,
        );
        painter.rect_stroke(
            tab,
            1.5,
            egui::Stroke::new(0.9, outline),
            egui::StrokeKind::Inside,
        );
    }
    response
}

fn folder_icon_color(dark_mode: bool) -> egui::Color32 {
    if cfg!(target_os = "macos") {
        if dark_mode {
            egui::Color32::from_rgb(104, 169, 255)
        } else {
            egui::Color32::from_rgb(54, 124, 232)
        }
    } else if dark_mode {
        egui::Color32::from_rgb(226, 179, 74)
    } else {
        egui::Color32::from_rgb(176, 118, 18)
    }
}

fn folder_outline_color(dark_mode: bool, selected: bool) -> egui::Color32 {
    match (dark_mode, selected) {
        (true, true) => egui::Color32::from_gray(245),
        (true, false) => egui::Color32::from_gray(132),
        (false, true) => egui::Color32::from_gray(24),
        (false, false) => egui::Color32::from_rgb(83, 60, 22),
    }
}

fn entry_type(entry: &Entry) -> String {
    if entry.is_symlink && entry.is_dir {
        "Folder link".into()
    } else if entry.is_dir {
        if cfg!(target_os = "windows") {
            "File folder".into()
        } else {
            "Folder".into()
        }
    } else if entry.is_symlink {
        "Symbolic link".into()
    } else if let Some(extension) = entry.path.extension().and_then(|value| value.to_str()) {
        format!("{} file", extension.to_uppercase())
    } else {
        "File".into()
    }
}

fn format_modified(modified: Option<std::time::SystemTime>) -> String {
    crate::platform::format_modified(modified)
}

fn restore_panel_width(
    ctx: &egui::Context,
    id: egui::Id,
    left: bool,
    preferred_width: f32,
    max_width: f32,
) {
    if preferred_width > max_width {
        return;
    }
    ctx.data_mut(|data| {
        if let Some(mut state) =
            data.get_persisted::<egui::containers::panel::PanelState>(id)
            && state.rect.width() + 0.5 < preferred_width
        {
            if left {
                state.rect.max.x = state.rect.min.x + preferred_width;
            } else {
                state.rect.min.x = state.rect.max.x - preferred_width;
            }
            data.insert_persisted(id, state);
        }
    });
}

fn item_count(count: usize) -> String {
    if count == 1 {
        "1 item".into()
    } else {
        format!("{count} items")
    }
}

fn search_result_count(count: usize) -> String {
    if count == 1 {
        "1 search result".into()
    } else {
        format!("{count} search results")
    }
}

fn sort_indicator(descending: bool, label: &str) -> String {
    let direction = if descending { "↓" } else { "↑" };
    format!("{label} {direction}")
}

fn sort_hint(sort: SortBy, current_sort: SortBy, descending: bool, label: &str) -> String {
    if sort == current_sort {
        let direction = if descending { "descending" } else { "ascending" };
        format!("Sorted by {label} ({direction}). Click again to reverse.")
    } else {
        format!("Sort by {label}. Click again to reverse direction.")
    }
}

fn command_modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    }
}

fn delete_key_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘+Backspace"
    } else {
        "Delete"
    }
}

fn details_key_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Option+Shift+P"
    } else {
        "Alt+Shift+P"
    }
}

fn trash_action_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "Delete"
    } else {
        "Move to Trash"
    }
}

fn back_key_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘+["
    } else {
        "Alt+←"
    }
}

fn forward_key_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘+]"
    } else {
        "Alt+→"
    }
}

fn up_key_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘+↑"
    } else {
        "Alt+↑"
    }
}

fn hidden_key_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘+Shift+."
    } else {
        "Ctrl+H"
    }
}
