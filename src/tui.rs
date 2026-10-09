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
    Finished(String, Option<Vec<PathBuf>>),
}

enum Prompt {
    GoToFolder,
    NewFolder,
    Rename(PathBuf),
    Delete(Vec<PathBuf>),
    Conflict(Vec<PathBuf>, PathBuf, Transfer),
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
        let tx = self.tx.clone();
        let search_epoch = self.search_epoch.clone();
        self.message = "Searching…".into();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(160));
            if search_epoch.load(Ordering::Relaxed) != generation {
                return;
            }
            let result = core::search(&root, &query, hidden).map_err(|error| error.to_string());
            let _ = tx.send(Event::Searched(generation, query, result));
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
                Event::Finished(message, clear_after) => {
                    self.active_operations = self.active_operations.saturating_sub(1);
                    self.message = message;
                    if let Some(paths) = clear_after
                        && self.cut
                        && self.clipboard == paths
                    {
                        self.clipboard.clear();
                        self.cut = false;
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
        self.visible = if self.query.trim().is_empty() {
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
            Some(Prompt::GoToFolder | Prompt::NewFolder | Prompt::Rename(_)) if width < 40 => 5,
            Some(Prompt::GoToFolder | Prompt::NewFolder | Prompt::Rename(_)) if width < 60 => 4,
            Some(_) => 3,
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
        let location = Paragraph::new(compact_path(
            &self.current,
            chunks[0].width.saturating_sub(2) as usize,
        ))
            .block(Block::default().title("Path").borders(Borders::ALL));
        frame.render_widget(location, chunks[0]);

        let items = if self.visible.is_empty() {
            let message = if self.message.starts_with("Error:")
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
                    let item_style = if cut_pending {
                        Style::default().add_modifier(Modifier::DIM)
                    } else {
                        Style::default()
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{marker} {icon} "), marker_style),
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
        let title = if self.searching || !self.query.trim().is_empty() {
            let search_suffix = format!("  ·  {sort_title}");
            let query_width = (width.saturating_sub(2) as usize)
                .saturating_sub(UnicodeWidthStr::width("Search: "))
                .saturating_sub(UnicodeWidthStr::width(search_suffix.as_str()));
            let query = if self.searching {
                cursor_input_text(&self.query, self.edit_cursor, query_width)
            } else {
                truncate_to_display_width(&sanitize_terminal_text(&self.query), query_width)
            };
            format!("Search: {query}{search_suffix}")
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
        frame.render_stateful_widget(list, chunks[1], &mut self.list_state);

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
                "↑/↓ move  PgUp/PgDn page  Enter open  Backspace up  F5 refresh\ng path  / search  n new  F2 rename  Ctrl+A all  Space select\nc/x/p copy/cut/paste  d trash  h hidden  s/S sort  q/Ctrl+C quit"
            } else if width >= 52 {
                "↑/↓ move  PgUp/PgDn page  Enter open  Backspace up\ng path  / search  n new  F2 rename  Space select\nCtrl+A all  c/x/p copy/cut/paste  d trash\nh hidden  s/S sort  q/Ctrl+C quit"
            } else if width >= 40 {
                "↑/↓ move  PgUp/PgDn page\nEnter open  Backspace up\ng path  / search  n new\nF2 rename  Space select  Ctrl+A all\nc/x/p copy/cut/paste  d trash\nh hidden  s/S sort  q/Ctrl+C quit"
            } else {
                "↑↓/PgUp/PgDn move\nEnter open / Backspace up\ng path / / search / n new\nF2 rename / Space select\nCtrl+A all / c/x/p transfer\nd trash / h hidden / s/S sort\nq / Ctrl+C quit"
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

    fn prompt_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('a')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(
                &self.prompt,
                Some(Prompt::GoToFolder | Prompt::NewFolder | Prompt::Rename(_))
            )
        {
            self.prompt_text.clear();
            self.edit_cursor = 0;
            return;
        }
        if matches!(
            self.prompt.as_ref(),
            Some(Prompt::GoToFolder | Prompt::NewFolder | Prompt::Rename(_))
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

    fn open_selected(&mut self) {
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
