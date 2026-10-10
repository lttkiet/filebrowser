use std::{
    collections::HashSet,
    io::{self, stdout},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

use crossterm::{
    event::{self, Event as TerminalEvent, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    core::{self, ConflictChoice, Entry, SortBy, Transfer},
    display,
    platform,
};

enum Event {
    Listed(Option<u64>, PathBuf, Result<Vec<Entry>, String>),
    Searched(u64, String, Result<Vec<Entry>, String>),
    TextLoaded(u64, PathBuf, Result<core::TextFile, String>),
    TextSaved(u64, PathBuf, Result<core::FileRevision, String>),
    ArchiveListed(u64, PathBuf, String, Result<Vec<core::ArchiveEntry>, String>),
    ArchiveTextLoaded(u64, PathBuf, String, Result<String, String>),
    PropertiesLoaded(PathBuf, Result<core::FileProperties, String>),
    Indexed(u64, PathBuf, Result<crate::content_index::ContentIndex, String>),
    Finished(String, Option<Vec<PathBuf>>),
}

enum Prompt {
    GoToFolder,
    NewFolder,
    Rename(PathBuf),
    Properties(PathBuf, bool),
    Delete(Vec<PathBuf>),
    Conflict(Vec<PathBuf>, PathBuf, Transfer),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DocumentMode {
    Preview,
    Edit,
}

struct TuiDocument {
    generation: u64,
    path: PathBuf,
    archive_member: Option<String>,
    mode: DocumentMode,
    contents: String,
    original_contents: String,
    revision: Option<core::FileRevision>,
    caret: usize,
    scroll: u16,
    loading: bool,
    saving: bool,
    error: Option<String>,
}

struct TuiArchive {
    generation: u64,
    path: PathBuf,
    directory: String,
    entries: Vec<core::ArchiveEntry>,
    loading: bool,
    error: Option<String>,
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let guard = Self;
        if let Err(error) = execute!(stdout(), EnterAlternateScreen) {
            drop(guard);
            return Err(error.into());
        }
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
    }
}

pub fn run(home: PathBuf) -> anyhow::Result<()> {
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(home, tx, rx);
    let result = app.run(&mut terminal);
    drop(terminal);
    result
}

struct App {
    home: PathBuf,
    current: PathBuf,
    entries: Vec<Entry>,
    visible: Vec<Entry>,
    selected: HashSet<PathBuf>,
    cursor: usize,
    query: String,
    edit_cursor: usize,
    search_generation: u64,
    search_epoch: Arc<AtomicU64>,
    listing_generation: u64,
    pending_cursor: Option<PathBuf>,
    searching: bool,
    search_results: Option<Vec<Entry>>,
    content_index: Option<crate::content_index::ContentIndex>,
    content_search: bool,
    index_generation: u64,
    index_building: Option<u64>,
    document: Option<TuiDocument>,
    document_generation: u64,
    archive: Option<TuiArchive>,
    archive_generation: u64,
    prompt: Option<Prompt>,
    prompt_text: String,
    clipboard: Vec<PathBuf>,
    cut: bool,
    show_hidden: bool,
    sort: SortBy,
    sort_descending: bool,
    page_size: usize,
    active_operations: usize,
    quit_when_idle: bool,
    message: String,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    list_state: ListState,
    quit: bool,
}

impl App {
    fn new(home: PathBuf, tx: Sender<Event>, rx: Receiver<Event>) -> Self {
        let mut app = Self {
            home: home.clone(),
            current: home.clone(),
            entries: Vec::new(),
            visible: Vec::new(),
            selected: HashSet::new(),
            cursor: 0,
            query: String::new(),
            edit_cursor: 0,
            search_generation: 0,
            search_epoch: Arc::new(AtomicU64::new(0)),
            listing_generation: 0,
            pending_cursor: None,
            searching: false,
            search_results: None,
            content_index: None,
            content_search: false,
            index_generation: 0,
            index_building: None,
            document: None,
            document_generation: 0,
            archive: None,
            archive_generation: 0,
            prompt: None,
            prompt_text: String::new(),
            clipboard: Vec::new(),
            cut: false,
            show_hidden: false,
            sort: SortBy::Name,
            sort_descending: false,
            page_size: 1,
            active_operations: 0,
            quit_when_idle: false,
            message: "Ready".into(),
            tx,
            rx,
            list_state: ListState::default(),
            quit: false,
        };
        app.load(home);
        app
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
        while !self.quit {
            self.receive();
            self.update_visible();
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(100))?
                && let TerminalEvent::Key(key) = event::read()?
            {
                self.key(key);
            }
        }
        Ok(())
    }

    fn load(&mut self, path: PathBuf) {
        self.next_search_generation();
        self.listing_generation = self.listing_generation.wrapping_add(1);
        let generation = self.listing_generation;
        self.current = path.clone();
        self.query.clear();
        self.edit_cursor = 0;
        self.search_results = None;
        self.entries.clear();
        self.visible.clear();
        self.selected.clear();
        self.cursor = 0;
        self.pending_cursor = None;
        self.list_state.select(None);
        self.message = "Loading…".into();
        let tx = self.tx.clone();
        let hidden = self.show_hidden;
        let sort = self.sort;
        let descending = self.sort_descending;
        thread::spawn(move || {
            let result = core::list_dir(&path, hidden, sort, descending)
                .map_err(|error| error.to_string());
            let _ = tx.send(Event::Listed(Some(generation), path, result));
        });
    }

    fn refresh(&mut self) {
        let path = self.current.clone();
        let query = self.query.clone();
        let edit_cursor = self.edit_cursor;
        let selected = self.selected.clone();
        let focused_path = self.visible.get(self.cursor).map(|entry| entry.path.clone());
        self.load(path);
        if !query.trim().is_empty() {
            self.query = query;
            self.edit_cursor = edit_cursor.min(self.query.len());
            self.search();
        }
        self.selected = selected;
        self.pending_cursor = focused_path;
    }

    fn search(&mut self) {
        let generation = self.next_search_generation();
        let query = self.query.clone();
        if query.trim().is_empty() {
            self.search_results = None;
            self.message = if self.searching {
                "Search: type a name".into()
            } else {
                item_count(self.entries.len())
            };
            return;
        }
        self.search_results = None;
        self.selected.clear();
        self.cursor = 0;
        let root = self.current.clone();
        let hidden = self.show_hidden;
        let content_search = self.content_search;
        let content_index = self.content_index.clone();
        let sort = self.sort;
        let descending = self.sort_descending;
        let tx = self.tx.clone();
        let search_epoch = self.search_epoch.clone();
        self.message = "Searching…".into();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(160));
            if search_epoch.load(Ordering::Relaxed) != generation {
                return;
            }
            let result = if content_search {
                content_index
                    .as_ref()
                    .filter(|index| index.root() == root)
                    .map(|index| index.search(&query, sort, descending))
                    .ok_or_else(|| "Index this folder with i before searching contents".to_owned())
            } else {
                core::search(&root, &query, hidden).map_err(|error| error.to_string())
            };
            let _ = tx.send(Event::Searched(generation, query, result));
        });
    }

    fn index_current_folder(&mut self) {
        if self.index_building.is_some() {
            self.message = "An index build is already in progress".into();
            return;
        }
        self.index_generation = self.index_generation.wrapping_add(1);
        let generation = self.index_generation;
        self.index_building = Some(generation);
        let root = self.current.clone();
        let hidden = self.show_hidden;
        let tx = self.tx.clone();
        self.message = "Indexing text files…".into();
        thread::spawn(move || {
            let result = crate::content_index::ContentIndex::build(&root, hidden)
                .map_err(|error| error.to_string());
            let _ = tx.send(Event::Indexed(generation, root, result));
        });
    }

    fn invalidate_content_index(&mut self) {
        self.content_index = None;
        self.index_generation = self.index_generation.wrapping_add(1);
    }

    fn open_document(&mut self, path: PathBuf, mode: DocumentMode) {
        self.document_generation = self.document_generation.wrapping_add(1);
        let generation = self.document_generation;
        self.document = Some(TuiDocument {
            generation,
            path: path.clone(),
            archive_member: None,
            mode,
            contents: String::new(),
            original_contents: String::new(),
            revision: None,
            caret: 0,
            scroll: 0,
            loading: true,
            saving: false,
            error: None,
        });
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = core::read_text(&path, 1_048_576).map_err(|error| error.to_string());
            let _ = tx.send(Event::TextLoaded(generation, path, result));
        });
    }

    fn open_archive(&mut self, path: PathBuf) {
        self.archive = Some(TuiArchive {
            generation: 0,
            path,
            directory: String::new(),
            entries: Vec::new(),
            loading: false,
            error: None,
        });
        self.selected.clear();
        self.load_archive_directory(String::new());
    }

    fn load_archive_directory(&mut self, directory: String) {
        let Some(archive) = self.archive.as_mut() else {
            return;
        };
        self.archive_generation = self.archive_generation.wrapping_add(1);
        let generation = self.archive_generation;
        let path = archive.path.clone();
        archive.generation = generation;
        archive.directory = directory.clone();
        archive.entries.clear();
        archive.loading = true;
        archive.error = None;
        self.cursor = 0;
        self.message = "Reading archive…".into();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = core::list_archive(&path, &directory).map_err(|error| error.to_string());
            let _ = tx.send(Event::ArchiveListed(generation, path, directory, result));
        });
    }

    fn open_archive_text(&mut self, archive_path: PathBuf, member: String) {
        self.document_generation = self.document_generation.wrapping_add(1);
        let generation = self.document_generation;
        self.document = Some(TuiDocument {
            generation,
            path: archive_path.clone(),
            archive_member: Some(member.clone()),
            mode: DocumentMode::Preview,
            contents: String::new(),
            original_contents: String::new(),
            revision: None,
            caret: 0,
            scroll: 0,
            loading: true,
            saving: false,
            error: None,
        });
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = core::read_archive_text(&archive_path, &member, 1_048_576)
                .map_err(|error| error.to_string());
            let _ = tx.send(Event::ArchiveTextLoaded(
                generation,
                archive_path,
                member,
                result,
            ));
        });
    }

    fn save_document(&mut self) {
        let Some(document) = self.document.as_mut() else {
            return;
        };
        let Some(revision) = document.revision else {
            return;
        };
        let generation = document.generation;
        let path = document.path.clone();
        let contents = document.contents.clone();
        document.saving = true;
        document.error = None;
        self.active_operations = self.active_operations.saturating_add(1);
        self.message = "Saving text file…".into();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = core::write_text(&path, revision, &contents)
                .and_then(|_| core::read_text(&path, 1_048_576).map(|file| file.revision))
                .map_err(|error| error.to_string());
            let _ = tx.send(Event::TextSaved(generation, path, result));
        });
    }

    fn begin_properties(&mut self) {
        let Some(path) = self.selected_paths().first().cloned() else {
            return;
        };
        self.active_operations = self.active_operations.saturating_add(1);
        self.message = "Loading properties…".into();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = core::file_properties(&path).map_err(|error| error.to_string());
            let _ = tx.send(Event::PropertiesLoaded(path, result));
        });
    }

    fn next_search_generation(&mut self) -> u64 {
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_epoch
            .store(self.search_generation, Ordering::Relaxed);
        self.search_generation
    }

    fn mutation(
        &mut self,
        name: &str,
        clear_after: Option<Vec<PathBuf>>,
        operation: impl FnOnce() -> anyhow::Result<Option<String>> + Send + 'static,
    ) {
        self.active_operations = self.active_operations.saturating_add(1);
        self.message = format!("{name}…");
        let tx = self.tx.clone();
        let path = self.current.clone();
        let hidden = self.show_hidden;
        let sort = self.sort;
        let descending = self.sort_descending;
        let query = self.query.clone();
        let search_generation = self.search_generation;
        let search_epoch = self.search_epoch.clone();
        thread::spawn(move || {
            let operation_result = operation();
            let clear_after = operation_result
                .as_ref()
                .is_ok_and(Option::is_none)
                .then_some(clear_after)
                .flatten();
            let mut message = match operation_result {
                Ok(Some(message)) => message,
                Ok(None) => "Operation completed".into(),
                Err(error) => format!("Error: {error}"),
            };
            let result = core::list_dir(&path, hidden, sort, descending)
                .map_err(|error| error.to_string());
            let search_result = if query.trim().is_empty() {
                None
            } else {
                thread::sleep(Duration::from_millis(160));
                (search_epoch.load(Ordering::Relaxed) == search_generation)
                    .then(|| core::search(&path, &query, hidden).map_err(|error| error.to_string()))
            };
            if let Err(error) = &result {
                message = format!("{message}; refresh failed: {error}");
            }
            let _ = tx.send(Event::Listed(None, path, result));
            if let Some(search_result) = search_result {
                let _ = tx.send(Event::Searched(search_generation, query, search_result));
            }
            let _ = tx.send(Event::Finished(message, clear_after));
        });
    }

    fn receive(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Listed(generation, path, result)
                    if path == self.current
                        && generation.is_none_or(|generation| generation == self.listing_generation) =>
                        match result {
                            Ok(mut entries) => {
                        core::sort_entries(&mut entries, self.sort, self.sort_descending);
                        if self.query.trim().is_empty() {
                            self.selected
                                .retain(|path| entries.iter().any(|entry| &entry.path == path));
                        }
                        self.entries = entries;
                        if self.query.trim().is_empty() {
                            self.message = item_count(self.entries.len());
                            self.update_visible();
                            self.restore_cursor();
                        }
                    }
                    Err(error) if self.query.trim().is_empty() => {
                        self.message = format!("Error: {error}");
                    }
                    Err(_) => {}
                },
                Event::Searched(generation, query, result)
                    if generation == self.search_generation && query == self.query =>
                match result {
                    Ok(mut entries) => {
                        core::sort_entries(&mut entries, self.sort, self.sort_descending);
                        self.selected
                            .retain(|path| entries.iter().any(|entry| &entry.path == path));
                        self.search_results = Some(entries);
                        self.message = match_count(
                            self.search_results.as_ref().map_or(0, Vec::len),
                        );
                        self.update_visible();
                        self.restore_cursor();
                    }
                    Err(error) => self.message = format!("Search error: {error}"),
                },
                Event::Indexed(generation, root, result) => {
                    if self.index_building == Some(generation) {
                        self.index_building = None;
                    }
                    if generation == self.index_generation && root == self.current {
                        match result {
                            Ok(index) => {
                                self.message = index.summary();
                                self.content_index = Some(index);
                                if self.content_search && !self.query.trim().is_empty() {
                                    self.search();
                                }
                            }
                            Err(error) => self.message = format!("Index error: {error}"),
                        }
                    }
                }
                Event::PropertiesLoaded(path, result) => {
                    self.active_operations = self.active_operations.saturating_sub(1);
                    match result {
                        Ok(properties) => {
                            self.prompt_text = properties.modified.map_or_else(
                                String::new,
                                |modified| {
                                    let local: chrono::DateTime<chrono::Local> = modified.into();
                                    local.format("%Y-%m-%d %H:%M:%S").to_string()
                                },
                            );
                            self.edit_cursor = self.prompt_text.len();
                            self.prompt = Some(Prompt::Properties(path, properties.read_only));
                        }
                        Err(error) => self.message = format!("Properties error: {error}"),
                    }
                    if self.quit_when_idle && self.active_operations == 0 {
                        self.quit = true;
                    }
                }
                Event::TextLoaded(generation, path, result)
                    if self.document.as_ref().is_some_and(|document| {
                        document.generation == generation
                            && document.path == path
                            && document.archive_member.is_none()
                    }) =>
                {
                    if let Some(document) = self.document.as_mut() {
                        document.loading = false;
                        match result {
                            Ok(file) => {
                                document.contents = file.contents;
                                document.original_contents = document.contents.clone();
                                document.revision = Some(file.revision);
                                document.caret = document.contents.len();
                            }
                            Err(error) => document.error = Some(error),
                        }
                    }
                }
                Event::ArchiveListed(generation, path, directory, result)
                    if self.archive.as_ref().is_some_and(|archive| {
                        archive.generation == generation
                            && archive.path == path
                            && archive.directory == directory
                    }) =>
                {
                    if let Some(archive) = self.archive.as_mut() {
                        archive.loading = false;
                        match result {
                            Ok(entries) => {
                                self.message = item_count(entries.len());
                                archive.entries = entries;
                            }
                            Err(error) => {
                                self.message = format!("Archive error: {error}");
                                archive.error = Some(error);
                            }
                        }
                        self.update_visible();
                    }
                }
                Event::ArchiveTextLoaded(generation, path, member, result)
                    if self.document.as_ref().is_some_and(|document| {
                        document.generation == generation
                            && document.path == path
                            && document.archive_member.as_deref() == Some(member.as_str())
                    }) =>
                {
                    if let Some(document) = self.document.as_mut() {
                        document.loading = false;
                        match result {
                            Ok(contents) => document.contents = contents,
                            Err(error) => document.error = Some(error),
                        }
                    }
                }
                Event::TextSaved(generation, path, result) => {
                    self.active_operations = self.active_operations.saturating_sub(1);
                    self.content_index = None;
                    self.index_generation = self.index_generation.wrapping_add(1);
                    if let Some(document) = self.document.as_mut().filter(|document| {
                        document.generation == generation && document.path == path
                    }) {
                        document.saving = false;
                        match result {
                            Ok(revision) => {
                                document.revision = Some(revision);
                                document.original_contents = document.contents.clone();
                                self.message = "File saved".into();
                            }
                            Err(error) => document.error = Some(error),
                        }
                    }
                    self.refresh();
                    if self.quit_when_idle && self.active_operations == 0 {
                        self.quit = true;
                    }
                }
                Event::Finished(message, clear_after) => {
                    self.active_operations = self.active_operations.saturating_sub(1);
                    self.message = message;
                    self.content_index = None;
                    self.index_generation = self.index_generation.wrapping_add(1);
                    if let Some(paths) = clear_after
                        && self.cut
                        && self.clipboard == paths
                    {
                        self.clipboard.clear();
                        self.cut = false;
                    }
                    if self.content_search && !self.query.trim().is_empty() {
                        self.search();
                    }
                    if self.quit_when_idle && self.active_operations == 0 {
                        self.quit = true;
                    }
                }
                _ => {}
            }
        }
    }

    fn restore_cursor(&mut self) {
        if let Some(path) = self.pending_cursor.take()
            && let Some(index) = self.visible.iter().position(|entry| entry.path == path)
        {
            self.cursor = index;
            self.list_state.select(Some(index));
        }
    }

    fn update_visible(&mut self) {
        self.visible = if let Some(archive) = &self.archive {
            archive
                .entries
                .iter()
                .map(|entry| Entry {
                    path: archive.path.join(&entry.member_path),
                    name: entry.name.clone(),
                    is_dir: entry.is_dir,
                    is_symlink: false,
                    size: entry.size,
                    modified: None,
                })
                .collect()
        } else if self.query.trim().is_empty() {
            self.entries.clone()
        } else {
            self.search_results.clone().unwrap_or_default()
        };
        if self.visible.is_empty() {
            self.cursor = 0;
            self.list_state.select(None);
        } else {
            self.cursor = self.cursor.min(self.visible.len() - 1);
            self.list_state.select(Some(self.cursor));
        }
    }

    fn sort_visible(&mut self) {
        let focused_path = self.visible.get(self.cursor).map(|entry| entry.path.clone());
        core::sort_entries(&mut self.entries, self.sort, self.sort_descending);
        if let Some(results) = &mut self.search_results {
            core::sort_entries(results, self.sort, self.sort_descending);
        }
        self.update_visible();
        if let Some(path) = focused_path
            && let Some(index) = self.visible.iter().position(|entry| entry.path == path)
        {
            self.cursor = index;
            self.list_state.select(Some(index));
        }
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let width = frame.area().width;
        let footer_height = match &self.prompt {
            Some(Prompt::Delete(_)) | Some(Prompt::Conflict(_, _, _)) => 5,
            Some(Prompt::GoToFolder | Prompt::NewFolder | Prompt::Rename(_) | Prompt::Properties(_, _)) if width < 40 => 5,
            Some(Prompt::GoToFolder | Prompt::NewFolder | Prompt::Rename(_) | Prompt::Properties(_, _)) if width < 60 => 4,
            Some(_) => 3,
            None if self.document.is_some() => 4,
            None if self.archive.is_some() => 4,
            None if width >= 80 => 6,
            None if width >= 52 => 7,
            None if width >= 40 => 9,
            None => 10,
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(2),
                Constraint::Length(footer_height),
            ])
            .split(frame.area());
        self.page_size = chunks[1].height.saturating_sub(2).max(1) as usize;
        let location_text = self.archive.as_ref().map_or_else(
            || compact_path(&self.current, chunks[0].width.saturating_sub(2) as usize),
            |archive| {
                let inside = if archive.directory.is_empty() {
                    String::new()
                } else {
                    archive.directory.trim_end_matches('/').to_owned()
                };
                truncate_to_display_width(
                    &sanitize_terminal_text(&format!("{}!/{inside}", archive.path.display())),
                    chunks[0].width.saturating_sub(2) as usize,
                )
            },
        );
        let location = Paragraph::new(location_text)
            .block(Block::default().title("Path").borders(Borders::ALL));
        frame.render_widget(location, chunks[0]);

        let items = if self.visible.is_empty() {
            let message = if let Some(archive) = &self.archive {
                if archive.loading {
                    "Reading archive…"
                } else if let Some(error) = archive.error.as_deref() {
                    error
                } else {
                    "This archive folder is empty"
                }
            } else if self.message.starts_with("Error:")
                || self.message.starts_with("Search error:")
            {
                self.message.as_str()
            } else if self.message == "Loading…" {
                "Loading folder…"
            } else if self.message == "Searching…" {
                "Searching…"
            } else if self.query.trim().is_empty() {
                "This folder is empty"
            } else {
                "No matches found"
            };
            vec![ListItem::new(Line::from(Span::styled(
                sanitize_terminal_text(message),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            let show_size = width >= 52;
            let content_width = chunks[1].width.saturating_sub(2) as usize;
            let prefix_width = 6;
            let size_width = if show_size { 10 } else { 0 };
            let name_width = content_width
                .saturating_sub(prefix_width + size_width + if show_size { 2 } else { 0 });
            self.visible
                .iter()
                .map(|entry| {
                    let cut_pending = self.cut && self.clipboard.contains(&entry.path);
                    let marker = if self.selected.contains(&entry.path) {
                        "*"
                    } else {
                        " "
                    };
                    let icon = if entry.is_symlink {
                        "[L]"
                    } else if entry.is_dir {
                        "[D]"
                    } else {
                        "[F]"
                    };
                    let size = if entry.is_dir {
                        String::new()
                    } else {
                        display::format_size(entry.size)
                    };
                    let display_name = sanitize_terminal_text(&entry.name);
                    let name = truncate_to_display_width(&display_name, name_width);
                    let padding = name_width.saturating_sub(UnicodeWidthStr::width(name.as_str()));
                    let marker_style = if cut_pending {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::DIM)
                    } else {
                        Style::default().fg(Color::Cyan)
                    };
                    let icon_color = if entry.is_symlink {
                        Some(Color::Magenta)
                    } else if entry.is_dir {
                        Some(if cfg!(target_os = "macos") {
                            Color::LightBlue
                        } else {
                            Color::Yellow
                        })
                    } else {
                        None
                    };
                    let icon_style = if let Some(color) = icon_color {
                        Style::default().fg(color)
                    } else {
                        Style::default()
                    };
                    let icon_style = if cut_pending {
                        icon_style.add_modifier(Modifier::DIM)
                    } else {
                        icon_style
                    };
                    let item_style = if cut_pending {
                        Style::default().add_modifier(Modifier::DIM)
                    } else {
                        Style::default()
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{marker} "), marker_style),
                        Span::styled(format!("{icon} "), icon_style),
                        Span::styled(name, item_style),
                        Span::raw(" ".repeat(padding + if show_size { 2 } else { 0 })),
                        if show_size {
                            Span::styled(format!("{size:>size_width$}"), item_style)
                        } else {
                            Span::raw(String::new())
                        },
                    ]))
                })
                .collect::<Vec<_>>()
        };
        let sort_indicator = if self.sort_descending { "↓" } else { "↑" };
        let sort_title = format!("{} {sort_indicator}", sort_name(self.sort));
        let title = if let Some(archive) = &self.archive {
            format!("Archive  ·  {}", sanitize_terminal_text(&archive.path.display().to_string()))
        } else if self.searching || !self.query.trim().is_empty() {
            let search_suffix = format!("  ·  {sort_title}");
            let search_label = if self.content_search { "Content: " } else { "Search: " };
            let query_width = (width.saturating_sub(2) as usize)
                .saturating_sub(UnicodeWidthStr::width(search_label))
                .saturating_sub(UnicodeWidthStr::width(search_suffix.as_str()));
            let query = if self.searching {
                cursor_input_text(&self.query, self.edit_cursor, query_width)
            } else {
                truncate_to_display_width(&sanitize_terminal_text(&self.query), query_width)
            };
            format!("{search_label}{query}{search_suffix}")
        } else {
            format!("Files  ·  {sort_title}")
        };
        let list = List::new(items)
            .block(
                Block::default()
                    .title(sanitize_terminal_text(&title))
                    .borders(Borders::ALL),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        if let Some(document) = &self.document {
            let lines = if document.loading {
                vec![Line::from("Loading text file…")]
            } else if let Some(error) = &document.error {
                vec![Line::from(sanitize_terminal_text(error))]
            } else {
                let contents = if document.mode == DocumentMode::Edit {
                    let caret = document.caret.min(document.contents.len());
                    format!(
                        "{}▏{}",
                        &document.contents[..caret],
                        &document.contents[caret..]
                    )
                } else {
                    document.contents.clone()
                };
                contents
                    .lines()
                    .map(|line| Line::from(sanitize_terminal_text(line)))
                    .collect::<Vec<_>>()
            };
            let source = document.archive_member.as_ref().map_or_else(
                || document.path.display().to_string(),
                |member| format!("{}!/{member}", document.path.display()),
            );
            let title = format!(
                "{} — {}",
                if document.mode == DocumentMode::Edit { "Edit" } else { "Preview" },
                sanitize_terminal_text(&source)
            );
            frame.render_widget(
                Paragraph::new(lines)
                    .block(Block::default().title(title).borders(Borders::ALL))
                    .wrap(Wrap { trim: false })
                    .scroll((document.scroll, 0)),
                chunks[1],
            );
        } else {
            frame.render_stateful_widget(list, chunks[1], &mut self.list_state);
        }

        let status = if let Some(prompt) = &self.prompt {
            match prompt {
                Prompt::GoToFolder => edit_prompt_status(
                    "Go to folder: ",
                    &self.prompt_text,
                    self.edit_cursor,
                    "Enter go · Ctrl+A clear · Esc cancel",
                    chunks[2].width as usize,
                ),
                Prompt::NewFolder => edit_prompt_status(
                    "New folder name: ",
                    &self.prompt_text,
                    self.edit_cursor,
                    "Enter create · Ctrl+A clear · Esc cancel",
                    chunks[2].width as usize,
                ),
                Prompt::Rename(_) => edit_prompt_status(
                    "New name: ",
                    &self.prompt_text,
                    self.edit_cursor,
                    "Enter rename · Ctrl+A clear · Esc cancel",
                    chunks[2].width as usize,
                ),
                Prompt::Properties(_, read_only) => format!(
                    "{}\nCtrl+R read-only: {} · Enter save · Esc cancel",
                    edit_prompt_status(
                        "Modified: ",
                        &self.prompt_text,
                        self.edit_cursor,
                        "YYYY-MM-DD HH:MM:SS",
                        chunks[2].width as usize,
                    ),
                    if *read_only { "on" } else { "off" }
                ),
                Prompt::Delete(paths) => {
                    format!(
                        "Move {} to the {}? [y/N, Esc/Ctrl+C cancel]\n{}",
                        item_count(paths.len()),
                        platform::trash_name(),
                        prompt_names(paths, 2, chunks[2].width.saturating_sub(4) as usize)
                    )
                }
                Prompt::Conflict(paths, destination, action) => {
                    let conflicts = paths
                        .iter()
                        .filter(|path| {
                            core::path_exists(
                                &destination.join(path.file_name().unwrap_or_default()),
                            )
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    let width = chunks[2].width.saturating_sub(4) as usize;
                    let destination = compact_path(
                        destination,
                        width.saturating_sub(24).max(8),
                    );
                    format!(
                        "{} {} to {}\n{} conflict(s): {}\n[r]Replace [s]Skip [Esc/Ctrl+C]Cancel",
                        if *action == Transfer::Move { "Move" } else { "Copy" },
                        item_count(paths.len()),
                        destination,
                        conflicts.len(),
                        prompt_names(&conflicts, 2, width.saturating_sub(17))
                    )
                }
            }
        } else if let Some(document) = &self.document {
            if document.loading {
                format!("Loading {}…\nEsc close", document.path.display())
            } else if let Some(error) = &document.error {
                format!(
                    "{}\n{}",
                    sanitize_terminal_text(error),
                    if document.mode == DocumentMode::Edit {
                        "Ctrl+S retry · r reload · Esc close"
                    } else {
                        "r reload · Esc close"
                    }
                )
            } else if document.mode == DocumentMode::Edit {
                let dirty = document.contents != document.original_contents;
                format!(
                    "{}{}\n{}",
                    if document.saving { "Saving… · " } else { "" },
                    if dirty { "Unsaved edits" } else { "Saved" },
                    "Text entry · Enter newline · Ctrl+S save · Esc close · x discard"
                )
            } else {
                format!("Preview · {}\n↑/↓ scroll · Esc close", document.path.display())
            }
        } else if let Some(archive) = &self.archive {
            format!(
                "{}\n↑/↓ move · Enter open · v preview · Backspace up · Esc close archive",
                if archive.loading {
                    "Reading archive…".to_owned()
                } else if let Some(error) = &archive.error {
                    format!("Archive error: {}", sanitize_terminal_text(error))
                } else {
                    format!("{} entries", archive.entries.len())
                }
            )
        } else {
            let help = if self.searching && width >= 80 {
                "Type to update · ←/→ edit · Home/End · Ctrl+A clear · Enter keep · Esc cancel · Ctrl+C quit"
            } else if self.searching && width >= 52 {
                "←/→ edit · Home/End · Ctrl+A clear · Enter keep · Esc cancel"
            } else if self.searching && width >= 40 {
                "←/→ edit · Home/End\nCtrl+A clear · Enter keep · Esc cancel"
            } else if self.searching {
                "←/→ edit\nCtrl+A clear · Enter keep\nEsc cancel · Ctrl+C quit"
            } else if width >= 80 {
                "↑/↓ move  PgUp/PgDn page  Enter open  Backspace up  F5 refresh\ng path  / search  n new  F2 rename  Ctrl+A all  Space select\nc/x/p transfer  d trash  v preview  e edit  i index  I content\nP properties  h hidden  s/S sort  q/Ctrl+C quit"
            } else if width >= 52 {
                "↑/↓ move  PgUp/PgDn page  Enter open  Backspace up\ng path  / search  n new  F2 rename  Space select\nCtrl+A all  c/x/p transfer  d trash  v preview\ne edit  i index  I content  P properties\nh hidden  s/S sort  q/Ctrl+C quit"
            } else if width >= 40 {
                "↑/↓ move  PgUp/PgDn page\nEnter open  Backspace up\ng path  / search  n new\nF2 rename  Space select  Ctrl+A all\nc/x/p transfer  d trash  v preview\ne edit  i index  I content\nP props  h hidden  s/S sort\nq/Ctrl+C quit"
            } else {
                "↑↓/PgUp/PgDn move\nEnter open / Backspace up\ng path / / search / n new\nF2 rename / Space select\nCtrl+A all / c/x/p transfer\nd trash / v view / e edit\ni index / I content search\nP props / h hidden / s/S sort\nq / Ctrl+C quit"
            };
            let status = if self.cut && !self.clipboard.is_empty() {
                let cut_count = item_count(self.clipboard.len());
                if self.message == "Cut selected items" {
                    format!("Cut: {cut_count}")
                } else {
                    format!("{} · Cut: {cut_count}", self.message)
                }
            } else {
                self.message.clone()
            };
            format!("{status}\n{help}")
        };
        let status_style = if self.prompt.is_some() {
            Style::default()
        } else {
            footer_status_style(&self.message)
        };
        let status_lines = sanitize_terminal_text(&status)
            .lines()
            .enumerate()
            .map(|(index, line)| {
                if index == 0 {
                    Line::from(Span::styled(line.to_owned(), status_style))
                } else {
                    Line::from(line.to_owned())
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(status_lines)
                .wrap(Wrap { trim: true })
                .block(Block::default().borders(Borders::ALL)),
            chunks[2],
        );
    }

    fn key(&mut self, key: KeyEvent) {
        if self.quit_when_idle {
            return;
        }
        if self.document.is_some() {
            self.document_key(key);
            return;
        }
        if self.archive.is_some() {
            self.archive_key(key);
            return;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.prompt.is_some() {
                self.prompt = None;
                self.prompt_text.clear();
                self.edit_cursor = 0;
            } else {
                self.request_quit();
            }
            return;
        }
        if self.prompt.is_some() {
            self.prompt_key(key);
            return;
        }
        if self.searching {
            match key.code {
                KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.clear();
                    self.edit_cursor = 0;
                    self.search();
                }
                KeyCode::Esc => {
                    self.searching = false;
                    self.query.clear();
                    self.edit_cursor = 0;
                    self.search_results = None;
                    self.message = item_count(self.entries.len());
                }
                KeyCode::Enter => {
                    self.searching = false;
                    self.search();
                }
                KeyCode::Backspace => {
                    remove_previous_grapheme(&mut self.query, &mut self.edit_cursor);
                    self.search();
                }
                KeyCode::Delete => {
                    remove_next_grapheme(&mut self.query, &mut self.edit_cursor);
                    self.search();
                }
                KeyCode::Left => {
                    self.edit_cursor = previous_grapheme_boundary(&self.query, self.edit_cursor);
                }
                KeyCode::Right => {
                    self.edit_cursor = next_grapheme_boundary(&self.query, self.edit_cursor);
                }
                KeyCode::Home => self.edit_cursor = 0,
                KeyCode::End => self.edit_cursor = self.query.len(),
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.insert(self.edit_cursor, ch);
                    self.edit_cursor = grapheme_boundary_at_or_after(
                        &self.query,
                        self.edit_cursor + ch.len_utf8(),
                    );
                    self.search();
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('a' | 'A') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.select_all_visible();
            }
            KeyCode::Char('q') => self.request_quit(),
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-(self.page_size as isize)),
            KeyCode::PageDown => self.move_cursor(self.page_size as isize),
            KeyCode::Char('g') => {
                self.prompt_text.clear();
                self.edit_cursor = 0;
                self.prompt = Some(Prompt::GoToFolder);
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.prompt_text.clear();
                self.edit_cursor = 0;
                self.prompt = Some(Prompt::GoToFolder);
            }
            KeyCode::Enter | KeyCode::Right => self.open_selected(),
            KeyCode::Backspace | KeyCode::Left => {
                let previous = self.current.clone();
                if let Some(parent) = core::parent(&previous) {
                    self.load(parent);
                    self.pending_cursor = Some(previous);
                }
            }
            KeyCode::Char('/') => {
                self.searching = true;
                self.query.clear();
                self.edit_cursor = 0;
                self.message = "Search: type a name".into();
            }
            KeyCode::Char(' ') => self.toggle_selection(),
            KeyCode::Char('h') => {
                self.show_hidden = !self.show_hidden;
                self.invalidate_content_index();
                self.refresh();
            }
            KeyCode::F(5) => self.refresh(),
            KeyCode::Char('s') => {
                self.sort = match self.sort {
                    SortBy::Name => SortBy::Kind,
                    SortBy::Kind => SortBy::Size,
                    SortBy::Size => SortBy::Modified,
                    SortBy::Modified => SortBy::Name,
                };
                self.sort_descending = false;
                self.sort_visible();
            }
            KeyCode::Char('S') => {
                self.sort_descending = !self.sort_descending;
                self.sort_visible();
            }
            KeyCode::Char('n') => {
                self.prompt_text.clear();
                self.edit_cursor = 0;
                self.prompt = Some(Prompt::NewFolder);
            }
            KeyCode::F(2) => self.begin_rename(),
            KeyCode::Char('v') => {
                if let Some(path) = self.selected_paths().first().cloned() {
                    self.open_document(path, DocumentMode::Preview);
                }
            }
            KeyCode::Char('e') => {
                if let Some(path) = self.selected_paths().first().cloned() {
                    self.open_document(path, DocumentMode::Edit);
                }
            }
            KeyCode::Char('i') => self.index_current_folder(),
            KeyCode::Char('I') => {
                self.content_search = !self.content_search;
                self.message = if self.content_search {
                    "Content search enabled · press / to search indexed text".into()
                } else {
                    "Name search enabled".into()
                };
            }
            KeyCode::Char('P') => self.begin_properties(),
            KeyCode::Char('a') => {
                if let Some(path) = self.selected_paths().first().cloned()
                    && path.extension().is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("zip")
                    })
                {
                    self.open_archive(path);
                } else {
                    self.message = "Select a ZIP archive to browse".into();
                }
            }
            KeyCode::Char('c') => self.copy_or_cut(false),
            KeyCode::Char('x') => self.copy_or_cut(true),
            KeyCode::Char('p') => self.paste(),
            KeyCode::Char('d') => {
                let paths = self.selected_paths();
                if !paths.is_empty() {
                    self.prompt = Some(Prompt::Delete(paths));
                }
            }
            KeyCode::Home => {
                self.cursor = 0;
            }
            KeyCode::End => {
                self.cursor = self.visible.len().saturating_sub(1);
            }
            KeyCode::Esc => {
                self.selected.clear();
                self.message = "Selection cleared".into();
            }
            _ => {}
        }
    }

    fn request_quit(&mut self) {
        if self.active_operations == 0 {
            self.quit = true;
        } else {
            self.quit_when_idle = true;
            self.message = "Waiting for file operations to finish before quitting".into();
        }
    }

    fn document_key(&mut self, key: KeyEvent) {
        let Some(document) = self.document.as_mut() else {
            return;
        };
        if document.loading || document.saving {
            if key.code == KeyCode::Esc && !document.saving {
                self.document = None;
            }
            return;
        }
        if key.code == KeyCode::Char('r') && document.error.is_some() {
            let path = document.path.clone();
            let member = document.archive_member.clone();
            let mode = document.mode;
            if let Some(member) = member {
                self.open_archive_text(path, member);
            } else {
                self.open_document(path, mode);
            }
            return;
        }
        match document.mode {
            DocumentMode::Preview => match key.code {
                KeyCode::Esc => self.document = None,
                KeyCode::Up | KeyCode::Char('k') => {
                    document.scroll = document.scroll.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    document.scroll = document.scroll.saturating_add(1)
                }
                KeyCode::PageUp => document.scroll = document.scroll.saturating_sub(self.page_size as u16),
                KeyCode::PageDown => document.scroll = document.scroll.saturating_add(self.page_size as u16),
                KeyCode::Home => document.scroll = 0,
                _ => {}
            },
            DocumentMode::Edit => {
                if key.code == KeyCode::Char('s')
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.save_document();
                    return;
                }
                if key.code == KeyCode::Char('x') && !key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.document = None;
                    return;
                }
                if key.code == KeyCode::Esc {
                    if document.contents == document.original_contents {
                        self.document = None;
                    } else {
                        self.message = "Unsaved edits: Ctrl+S to save or x to discard".into();
                    }
                    return;
                }
                match key.code {
                    KeyCode::Left => {
                        document.caret = previous_grapheme_boundary(
                            &document.contents,
                            document.caret,
                        );
                    }
                    KeyCode::Right => {
                        document.caret = next_grapheme_boundary(
                            &document.contents,
                            document.caret,
                        );
                    }
                    KeyCode::Home => document.caret = 0,
                    KeyCode::End => document.caret = document.contents.len(),
                    KeyCode::Backspace => {
                        remove_previous_grapheme(&mut document.contents, &mut document.caret);
                    }
                    KeyCode::Delete => {
                        remove_next_grapheme(&mut document.contents, &mut document.caret);
                    }
                    KeyCode::Enter => {
                        document.contents.insert(document.caret, '\n');
                        document.caret += 1;
                    }
                    KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        document.contents.insert(document.caret, ch);
                        document.caret = grapheme_boundary_at_or_after(
                            &document.contents,
                            document.caret + ch.len_utf8(),
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    fn prompt_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('a')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(
                &self.prompt,
                Some(
                    Prompt::GoToFolder
                        | Prompt::NewFolder
                        | Prompt::Rename(_)
                        | Prompt::Properties(_, _)
                )
            )
        {
            self.prompt_text.clear();
            self.edit_cursor = 0;
            return;
        }
        if matches!(
            self.prompt.as_ref(),
            Some(
                Prompt::GoToFolder
                    | Prompt::NewFolder
                    | Prompt::Rename(_)
                    | Prompt::Properties(_, _)
            )
        ) && edit_text_input(&mut self.prompt_text, &mut self.edit_cursor, key)
        {
            return;
        }
        let prompt = self.prompt.take().unwrap();
        match prompt {
            Prompt::GoToFolder => match key.code {
                KeyCode::Esc => {
                    self.prompt_text.clear();
                    self.edit_cursor = 0;
                }
                KeyCode::Enter if !self.prompt_text.trim().is_empty() => {
                    let path = core::resolve_path_input(
                        &self.prompt_text,
                        &self.current,
                        &self.home,
                    );
                    self.prompt_text.clear();
                    self.edit_cursor = 0;
                    self.load(path);
                }
                KeyCode::Enter => self.prompt = Some(Prompt::GoToFolder),
                _ => self.prompt = Some(Prompt::GoToFolder),
            },
            Prompt::NewFolder => match key.code {
                KeyCode::Esc => self.edit_cursor = 0,
                KeyCode::Enter => {
                    let parent = self.current.clone();
                    let name = self.prompt_text.clone();
                    self.mutation("Creating folder", None, move || {
                        core::create_folder(&parent, &name)?;
                        Ok(None)
                    });
                    self.prompt_text.clear();
                    self.edit_cursor = 0;
                }
                _ => self.prompt = Some(Prompt::NewFolder),
            },
            Prompt::Rename(path) => match key.code {
                KeyCode::Esc => self.edit_cursor = 0,
                KeyCode::Enter => {
                    let name = self.prompt_text.clone();
                    self.mutation("Renaming", None, move || {
                        core::rename(&path, &name)?;
                        Ok(None)
                    });
                    self.prompt_text.clear();
                    self.edit_cursor = 0;
                }
                _ => self.prompt = Some(Prompt::Rename(path)),
            },
            Prompt::Properties(path, read_only) => {
                if key.code == KeyCode::Char('r')
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.prompt = Some(Prompt::Properties(path, !read_only));
                } else {
                    match key.code {
                        KeyCode::Esc => {
                            self.prompt_text.clear();
                            self.edit_cursor = 0;
                        }
                        KeyCode::Enter => {
                            let parsed = chrono::NaiveDateTime::parse_from_str(
                                &self.prompt_text,
                                "%Y-%m-%d %H:%M:%S",
                            )
                            .ok()
                            .and_then(|value| {
                                value.and_local_timezone(chrono::Local).single()
                            });
                            if let Some(modified) = parsed {
                                let modified = modified.into();
                                self.mutation("Updating properties", None, move || {
                                    core::set_file_properties(&path, read_only, modified)?;
                                    Ok(None)
                                });
                                self.prompt_text.clear();
                                self.edit_cursor = 0;
                            } else {
                                self.message = "Use YYYY-MM-DD HH:MM:SS for modified time".into();
                                self.prompt = Some(Prompt::Properties(path, read_only));
                            }
                        }
                        _ => self.prompt = Some(Prompt::Properties(path, read_only)),
                    }
                }
            }
            Prompt::Delete(paths) => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.mutation("Moving to trash", None, move || {
                        core::delete_to_trash(&paths).map(|_| None)
                    })
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {}
                _ => self.prompt = Some(Prompt::Delete(paths)),
            },
            Prompt::Conflict(paths, destination, action) => match key.code {
                KeyCode::Char('r') => {
                    self.do_transfer(paths, destination, action, ConflictChoice::Replace)
                }
                KeyCode::Char('s') => {
                    self.do_transfer(paths, destination, action, ConflictChoice::Skip)
                }
                KeyCode::Esc => {}
                _ => self.prompt = Some(Prompt::Conflict(paths, destination, action)),
            },
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        self.cursor =
            (self.cursor as isize + delta).clamp(0, self.visible.len() as isize - 1) as usize;
    }

    fn archive_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.archive = None;
                self.update_visible();
                self.cursor = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-(self.page_size as isize)),
            KeyCode::PageDown => self.move_cursor(self.page_size as isize),
            KeyCode::Enter | KeyCode::Right => self.open_archive_selected(),
            KeyCode::Char('v') => {
                let selected = self.archive.as_ref().and_then(|archive| {
                    archive.entries.get(self.cursor).map(|entry| {
                        (archive.path.clone(), entry.clone())
                    })
                });
                if let Some((path, entry)) = selected {
                    if !entry.is_dir {
                        self.open_archive_text(path, entry.lookup_path);
                    }
                }
            }
            KeyCode::F(5) => {
                if let Some(archive) = self.archive.as_ref() {
                    self.load_archive_directory(archive.directory.clone());
                }
            }
            KeyCode::Backspace | KeyCode::Left => {
                let parent = self.archive.as_ref().map(|archive| {
                    archive
                        .directory
                        .trim_end_matches('/')
                        .rsplit_once('/')
                        .map_or_else(String::new, |(parent, _)| parent.to_owned())
                });
                if let Some(parent) = parent {
                    if self
                        .archive
                        .as_ref()
                        .is_some_and(|archive| archive.directory.is_empty())
                    {
                        self.archive = None;
                        self.update_visible();
                    } else {
                        self.load_archive_directory(parent);
                    }
                }
            }
            _ => {}
        }
    }

    fn open_archive_selected(&mut self) {
        let selected = self.archive.as_ref().and_then(|archive| {
            archive
                .entries
                .get(self.cursor)
                .map(|entry| (archive.path.clone(), entry.clone()))
        });
        if let Some((path, entry)) = selected {
            if entry.is_dir {
                self.load_archive_directory(entry.member_path);
            } else {
                self.open_archive_text(path, entry.lookup_path);
            }
        }
    }

    fn open_selected(&mut self) {
        if self.archive.is_some() {
            self.open_archive_selected();
            return;
        }
        let Some(entry) = self.visible.get(self.cursor).cloned() else {
            return;
        };
        if entry.is_dir {
            self.load(entry.path);
        } else if let Err(error) = opener::open(&entry.path) {
            self.message = format!("Could not open file: {error}");
        }
    }

    fn toggle_selection(&mut self) {
        if let Some(entry) = self.visible.get(self.cursor)
            && !self.selected.remove(&entry.path)
        {
            self.selected.insert(entry.path.clone());
        }
        self.message = selection_count(self.selected.len());
    }

    fn select_all_visible(&mut self) {
        self.selected = self.visible.iter().map(|entry| entry.path.clone()).collect();
        self.message = selection_count(self.selected.len());
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        if self.archive.is_some() {
            return Vec::new();
        }
        if self.selected.is_empty() {
            self.visible
                .get(self.cursor)
                .map(|entry| vec![entry.path.clone()])
                .unwrap_or_default()
        } else {
            self.visible
                .iter()
                .filter(|entry| self.selected.contains(&entry.path))
                .map(|entry| entry.path.clone())
                .collect()
        }
    }

    fn begin_rename(&mut self) {
        let Some(path) = self.selected_paths().first().cloned() else {
            return;
        };
        self.prompt_text = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.edit_cursor = self.prompt_text.len();
        self.prompt = Some(Prompt::Rename(path));
    }

    fn copy_or_cut(&mut self, cut: bool) {
        self.clipboard = self.selected_paths();
        self.cut = cut;
        self.message = if self.clipboard.is_empty() {
            "Nothing selected"
        } else if cut {
            "Cut selected items"
        } else {
            "Copied selected items"
        }
        .into();
    }

    fn paste(&mut self) {
        if self.clipboard.is_empty() {
            self.message = "Clipboard is empty".into();
            return;
        }
        let action = if self.cut {
            Transfer::Move
        } else {
            Transfer::Copy
        };
        let paths = self.clipboard.clone();
        let dest = self.current.clone();
        if paths
            .iter()
            .any(|path| core::path_exists(&dest.join(path.file_name().unwrap_or_default())))
        {
            self.prompt = Some(Prompt::Conflict(paths, dest, action));
        } else {
            self.do_transfer(paths, dest, action, ConflictChoice::Cancel);
        }
    }

    fn do_transfer(
        &mut self,
        paths: Vec<PathBuf>,
        dest: PathBuf,
        action: Transfer,
        choice: ConflictChoice,
    ) {
        self.prompt = None;
        let clear_after = (action == Transfer::Move && self.cut && self.clipboard == paths)
            .then(|| paths.clone());
        self.mutation("Transferring items", clear_after, move || {
            let skipped = core::transfer(&paths, &dest, action, choice)?;
            Ok((!skipped.is_empty())
                .then(|| format!("Skipped conflicting items: {}", skipped.join(", "))))
        });
    }
}

fn item_count(count: usize) -> String {
    if count == 1 {
        "1 item".into()
    } else {
        format!("{count} items")
    }
}

fn selection_count(count: usize) -> String {
    match count {
        0 => "Selection cleared".into(),
        1 => "1 item selected".into(),
        _ => format!("{count} items selected"),
    }
}

fn match_count(count: usize) -> String {
    if count == 1 {
        "1 match".into()
    } else {
        format!("{count} matches")
    }
}

fn sort_name(sort: SortBy) -> &'static str {
    match sort {
        SortBy::Name => "Name",
        SortBy::Kind => "Type",
        SortBy::Size => "Size",
        SortBy::Modified => "Date modified",
    }
}

fn prompt_names(paths: &[PathBuf], limit: usize, width: usize) -> String {
    if paths.is_empty() {
        return "None".into();
    }
    let name_width = width.saturating_sub(18).max(6);
    let mut names = paths
        .iter()
        .take(limit)
        .map(|path| {
            compact_path(
                Path::new(path.file_name().unwrap_or(path.as_os_str())),
                name_width,
            )
        })
        .collect::<Vec<_>>();
    if paths.len() > limit {
        names.push(format!("+{} more", paths.len() - limit));
    }
    names.join(", ")
}

fn truncate_to_display_width(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for grapheme in value.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if used + grapheme_width > width.saturating_sub(1) {
            break;
        }
        result.push_str(grapheme);
        used += grapheme_width;
    }
    result.push('…');
    result
}

fn previous_grapheme_boundary(value: &str, cursor: usize) -> usize {
    value[..cursor.min(value.len())]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_grapheme_boundary(value: &str, cursor: usize) -> usize {
    let cursor = cursor.min(value.len());
    value[cursor..]
        .graphemes(true)
        .next()
        .map_or(cursor, |grapheme| cursor + grapheme.len())
}

fn grapheme_boundary_at_or_after(value: &str, cursor: usize) -> usize {
    let cursor = cursor.min(value.len());
    let mut boundary = 0;
    for grapheme in value.graphemes(true) {
        boundary += grapheme.len();
        if boundary >= cursor {
            return boundary;
        }
    }
    value.len()
}

fn remove_previous_grapheme(value: &mut String, cursor: &mut usize) {
    let previous = previous_grapheme_boundary(value, *cursor);
    if previous < *cursor {
        value.replace_range(previous..*cursor, "");
        *cursor = previous;
    }
}

fn remove_next_grapheme(value: &mut String, cursor: &mut usize) {
    let next = next_grapheme_boundary(value, *cursor);
    if next > *cursor {
        value.replace_range(*cursor..next, "");
    }
}

fn edit_text_input(value: &mut String, cursor: &mut usize, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Left => *cursor = previous_grapheme_boundary(value, *cursor),
        KeyCode::Right => *cursor = next_grapheme_boundary(value, *cursor),
        KeyCode::Home => *cursor = 0,
        KeyCode::End => *cursor = value.len(),
        KeyCode::Backspace => remove_previous_grapheme(value, cursor),
        KeyCode::Delete => remove_next_grapheme(value, cursor),
        KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            value.insert(*cursor, character);
            *cursor = grapheme_boundary_at_or_after(value, *cursor + character.len_utf8());
        }
        _ => return false,
    }
    true
}

fn edit_prompt_status(label: &str, value: &str, cursor: usize, actions: &str, width: usize) -> String {
    let content_width = width.saturating_sub(2);
    let label_width = UnicodeWidthStr::width(label);
    let action_width = UnicodeWidthStr::width(actions);
    if content_width < 34 {
        let input_width = content_width.saturating_sub(label_width).max(1);
        return format!(
            "{label}{}\nEnter · Ctrl+A clear\nEsc cancel",
            cursor_input_text(value, cursor, input_width)
        );
    }
    if content_width < label_width + action_width + 2 {
        let input_width = content_width.saturating_sub(label_width).max(1);
        return format!(
            "{label}{}\n{actions}",
            cursor_input_text(value, cursor, input_width)
        );
    }
    let input_width = content_width - label_width - action_width - 1;
    format!(
        "{label}{} {actions}",
        cursor_input_text(value, cursor, input_width)
    )
}

fn cursor_input_text(value: &str, cursor: usize, max_width: usize) -> String {
    let cursor = cursor.min(value.len());
    let before = sanitize_terminal_text(&value[..cursor]);
    let after = sanitize_terminal_text(&value[cursor..]);
    if UnicodeWidthStr::width(before.as_str()) + UnicodeWidthStr::width(after.as_str()) + 1
        <= max_width
    {
        return format!("{before}▏{after}");
    }
    if max_width < 3 {
        return truncate_to_display_width("▏", max_width);
    }
    let text_width = max_width - 3;
    let left_width = text_width
        .min(UnicodeWidthStr::width(before.as_str()))
        .div_ceil(2);
    let left = suffix_within_width(&before, left_width);
    let right = prefix_within_width(
        &after,
        text_width.saturating_sub(UnicodeWidthStr::width(left.as_str())),
    );
    let left_marker = if UnicodeWidthStr::width(left.as_str()) < UnicodeWidthStr::width(before.as_str()) {
        "…"
    } else {
        ""
    };
    let right_marker = if UnicodeWidthStr::width(right.as_str()) < UnicodeWidthStr::width(after.as_str()) {
        "…"
    } else {
        ""
    };
    format!("{left_marker}{left}▏{right}{right_marker}")
}

fn prefix_within_width(value: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut width = 0;
    for grapheme in value.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width + grapheme_width > max_width {
            break;
        }
        result.push_str(grapheme);
        width += grapheme_width;
    }
    result
}

fn suffix_within_width(value: &str, max_width: usize) -> String {
    let mut graphemes = Vec::new();
    let mut width = 0;
    for grapheme in value.graphemes(true).rev() {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width + grapheme_width > max_width {
            break;
        }
        graphemes.push(grapheme);
        width += grapheme_width;
    }
    graphemes.into_iter().rev().collect()
}

fn footer_status_style(message: &str) -> Style {
    if message.starts_with("Error:")
        || message.starts_with("Search error:")
        || message.starts_with("Could not ")
        || message.contains("refresh failed:")
    {
        Style::default().fg(Color::LightRed)
    } else if message.starts_with("Skipped conflicting items:")
        || message.starts_with("Clipboard is empty")
    {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    }
}

fn sanitize_terminal_text(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control()
            || matches!(
                character,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{206f}'
            )
        {
            sanitized.extend(character.escape_default());
        } else {
            sanitized.push(character);
        }
    }
    sanitized
}

fn compact_path(path: &std::path::Path, width: usize) -> String {
    let full = sanitize_terminal_text(&path.to_string_lossy());
    if UnicodeWidthStr::width(full.as_str()) <= width {
        return full;
    }
    if width == 0 {
        return String::new();
    }
    let mut suffix = Vec::new();
    let mut used = 1;
    for grapheme in full.graphemes(true).rev() {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if used + grapheme_width > width {
            break;
        }
        suffix.push(grapheme);
        used += grapheme_width;
    }
    let mut compacted = String::from("…");
    for grapheme in suffix.into_iter().rev() {
        compacted.push_str(grapheme);
    }
    compacted
}
