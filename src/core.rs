use std::{
    collections::BTreeMap,
    fs,
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);
const MAX_TEXT_BYTES: u64 = 1_048_576;

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRevision {
    pub size: u64,
    pub modified: Option<SystemTime>,
    fingerprint: u64,
}

#[derive(Clone, Debug)]
pub struct TextFile {
    pub contents: String,
    pub revision: FileRevision,
}

#[derive(Clone, Copy, Debug)]
pub struct FileProperties {
    pub read_only: bool,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct ArchiveEntry {
    pub member_path: String,
    pub lookup_path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

fn normalize_archive_path(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/') {
        return None;
    }
    let normalized = normalized.trim_end_matches('/');
    if normalized.is_empty() {
        return None;
    }
    let mut components = normalized.split('/');
    let first = components.next()?;
    if first.as_bytes().get(1) == Some(&b':')
        || components
            .clone()
            .any(|component| component.is_empty() || component == "." || component == "..")
        || first == "."
        || first == ".."
    {
        return None;
    }
    Some(normalized.to_owned())
}

pub fn list_archive(path: &Path, directory: &str) -> anyhow::Result<Vec<ArchiveEntry>> {
    let mut archive = zip::ZipArchive::new(fs::File::open(path)?)?;
    if archive.len() > 100_000 {
        anyhow::bail!("Archive has too many entries to browse safely");
    }
    let normalized_directory = if directory.is_empty() {
        String::new()
    } else {
        normalize_archive_path(directory)
            .ok_or_else(|| anyhow::anyhow!("Archive folder path is invalid"))?
    };
    let prefix = normalized_directory.trim_matches('/');
    let prefix = if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}/")
    };
    let mut entries = BTreeMap::<String, ArchiveEntry>::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let Some(enclosed) = file.enclosed_name() else {
            continue;
        };
        let Some(enclosed_name) = enclosed.to_str() else {
            continue;
        };
        let Some(name) = normalize_archive_path(enclosed_name) else {
            continue;
        };
        let Some(relative) = name.strip_prefix(&prefix) else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }
        let (child_name, nested) = relative
            .split_once('/')
            .map_or((relative, false), |(name, _)| (name, true));
        if child_name.is_empty() {
            continue;
        }
        let is_dir = nested || file.is_dir();
        let member_path = if is_dir {
            format!("{}{child_name}/", prefix)
        } else {
            format!("{}{child_name}", prefix)
        };
        let lookup_path = if is_dir {
            member_path.clone()
        } else {
            file.name()?.into_owned()
        };
        entries.entry(member_path.clone()).or_insert_with(|| ArchiveEntry {
            member_path,
            lookup_path,
            name: child_name.to_owned(),
            is_dir,
            size: if is_dir { 0 } else { file.size() },
        });
    }
    Ok(entries.into_values().collect())
}

pub fn read_archive_text(path: &Path, member: &str, limit: u64) -> anyhow::Result<String> {
    normalize_archive_path(member)
        .ok_or_else(|| anyhow::anyhow!("Archive member path is invalid"))?;
    let mut archive = zip::ZipArchive::new(fs::File::open(path)?)?;
    let file = archive.by_name(member)?;
    if file.is_dir() {
        anyhow::bail!("Cannot preview an archive folder");
    }
    if file.size() > limit {
        anyhow::bail!("Archive member is larger than the {limit}-byte preview limit");
    }
    let mut contents = String::new();
    file.take(limit.saturating_add(1)).read_to_string(&mut contents)?;
    if contents.len() as u64 > limit {
        anyhow::bail!("Archive member exceeded the preview limit while reading");
    }
    Ok(contents)
}

pub fn read_text(path: &Path, limit: u64) -> anyhow::Result<TextFile> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        anyhow::bail!("Preview is available for regular files only");
    }
    if metadata.len() > limit {
        anyhow::bail!("File is larger than the {limit}-byte preview limit");
    }
    let mut contents = String::new();
    fs::File::open(path)?
        .take(limit.saturating_add(1))
        .read_to_string(&mut contents)
        .map_err(|error| anyhow::anyhow!("File is not readable UTF-8 text: {error}"))?;
    if contents.len() as u64 > limit {
        anyhow::bail!("File exceeded the {limit}-byte preview limit while reading");
    }
    Ok(TextFile {
        revision: FileRevision {
            size: contents.len() as u64,
            modified: metadata.modified().ok(),
            fingerprint: content_fingerprint(&contents),
        },
        contents,
    })
}

pub fn write_text(path: &Path, revision: FileRevision, contents: &str) -> anyhow::Result<()> {
    if contents.len() as u64 > MAX_TEXT_BYTES {
        anyhow::bail!("Edited text exceeds the {MAX_TEXT_BYTES}-byte limit");
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        anyhow::bail!("Editing is available for regular files only");
    }
    let current_file = read_text(path, MAX_TEXT_BYTES)?;
    let current = current_file.revision;
    if current != revision {
        anyhow::bail!("File changed since it was opened; reload it before saving");
    }
    drop(current_file);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("File has no parent directory"))?;
    let transaction = create_staging_directory(parent)?;
    let staged = transaction.join("new-file");
    let backup = transaction.join("previous-file");
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    if let Err(error) = output.write_all(contents.as_bytes()) {
        let _ = fs::remove_dir_all(&transaction);
        return Err(error.into());
    }
    #[cfg(not(windows))]
    if let Err(error) = output.set_permissions(metadata.permissions()) {
        drop(output);
        let _ = fs::remove_dir_all(&transaction);
        return Err(error.into());
    }
    drop(output);
    match read_text(path, MAX_TEXT_BYTES) {
        Ok(latest) if latest.revision == revision => {}
        Ok(_) => {
            let _ = fs::remove_dir_all(&transaction);
            anyhow::bail!("File changed while saving; reload it before saving");
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&transaction);
            return Err(error);
        }
    }
    #[cfg(windows)]
    replace_staged_file(path, &staged, &backup, &transaction)?;
    #[cfg(not(windows))]
    {
        if let Err(error) = fs::rename(path, &backup) {
            let _ = fs::remove_dir_all(&transaction);
            return Err(error.into());
        }
        if let Err(error) = fs::rename(&staged, path) {
            if let Err(restore_error) = fs::rename(&backup, path) {
                let _ = fs::remove_dir_all(&transaction);
                anyhow::bail!("Could not save file ({error}) or restore the original ({restore_error})");
            }
            let _ = fs::remove_dir_all(&transaction);
            return Err(error.into());
        }
    }
    let _ = fs::remove_file(&backup).or_else(|_| {
        let mut permissions = fs::metadata(&backup)?.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&backup, permissions)?;
        fs::remove_file(&backup)
    });
    let _ = fs::remove_dir(transaction);
    Ok(())
}

pub fn file_properties(path: &Path) -> anyhow::Result<FileProperties> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        anyhow::bail!("Properties are available for regular files and folders only");
    }
    Ok(FileProperties {
        read_only: metadata.permissions().readonly(),
        modified: metadata.modified().ok(),
    })
}

pub fn set_file_properties(path: &Path, read_only: bool, modified: SystemTime) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        anyhow::bail!("Properties are available for regular files and folders only");
    }
    let was_read_only = metadata.permissions().readonly();
    if was_read_only {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)?;
    }
    let update_time = set_modified_time(path, modified);
    if let Err(error) = update_time {
        if was_read_only {
            let mut permissions = metadata.permissions();
            permissions.set_readonly(true);
            if let Err(restore_error) = fs::set_permissions(path, permissions) {
                anyhow::bail!("Could not update modified time ({error}) or restore read-only state ({restore_error})");
            }
        }
        return Err(error.into());
    }
    let mut permissions = metadata.permissions();
    permissions.set_readonly(read_only);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn content_fingerprint(contents: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    contents.hash(&mut hasher);
    hasher.finish()
}

#[cfg(windows)]
fn replace_staged_file(
    original: &Path,
    staged: &Path,
    backup: &Path,
    transaction: &Path,
) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    let wide_path = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let original_wide = wide_path(original);
    let staged_wide = wide_path(staged);
    let backup_wide = wide_path(backup);
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::ReplaceFileW(
            original_wide.as_ptr(),
            staged_wide.as_ptr(),
            backup_wide.as_ptr(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if result != 0 {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    let original_exists = fs::symlink_metadata(original).is_ok();
    let backup_exists = fs::symlink_metadata(backup).is_ok();
    if !original_exists && backup_exists {
        if let Err(restore_error) = fs::rename(backup, original) {
            anyhow::bail!("Could not replace file ({error}) or restore the original ({restore_error}); recovery copy remains at {}", backup.display());
        }
    }
    if fs::symlink_metadata(backup).is_err() && fs::symlink_metadata(original).is_ok() {
        let _ = fs::remove_dir_all(transaction);
    }
    Err(error.into())
}

fn set_modified_time(path: &Path, modified: SystemTime) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        let file = fs::OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        file.set_times(fs::FileTimes::new().set_modified(modified))
    }
    #[cfg(not(windows))]
    {
        fs::File::open(path)?.set_times(fs::FileTimes::new().set_modified(modified))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    Name,
    Kind,
    Size,
    Modified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    Copy,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictChoice {
    Replace,
    Skip,
    Cancel,
}

pub fn list_dir(
    path: &Path,
    show_hidden: bool,
    sort: SortBy,
    descending: bool,
) -> anyhow::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for result in fs::read_dir(path)? {
        let entry = result?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let is_symlink = metadata.file_type().is_symlink();
        let is_dir = if is_symlink {
            fs::metadata(&path).is_ok_and(|target| target.is_dir())
        } else {
            metadata.is_dir()
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let hidden = is_hidden(&metadata, &name);
        if hidden && !show_hidden {
            continue;
        }
        entries.push(Entry {
            path,
            name,
            is_dir,
            is_symlink,
            size: metadata.len(),
            modified: metadata.modified().ok(),
        });
    }
    sort_entries(&mut entries, sort, descending);
    Ok(entries)
}

pub fn search(root: &Path, query: &str, show_hidden: bool) -> anyhow::Result<Vec<Entry>> {
    let query = query.to_lowercase();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    fs::symlink_metadata(root)?;
    let mut pending = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(dir) = pending.pop() {
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        for result in read_dir {
            let Ok(entry) = result else { continue };
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let hidden = is_hidden(&metadata, &name);
            if hidden && !show_hidden {
                continue;
            }
            let is_symlink = metadata.file_type().is_symlink();
            let is_dir = metadata.is_dir() && !is_symlink;
            if name.to_lowercase().contains(&query) {
                matches.push(Entry {
                    path: path.clone(),
                    name,
                    is_dir,
                    is_symlink,
                    size: metadata.len(),
                    modified: metadata.modified().ok(),
                });
            }
            if is_dir {
                pending.push(path);
            }
        }
    }
    sort_entries(&mut matches, SortBy::Name, false);
    Ok(matches)
}

pub fn create_folder(parent: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let name = validate_name(name)?;
    let path = parent.join(name);
    fs::create_dir(&path)?;
    Ok(path)
}

pub fn rename(path: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let name = validate_name(name)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot rename this path"))?;
    let destination = parent.join(name);
    if destination.exists() {
        anyhow::bail!("An item with that name already exists");
    }
    fs::rename(path, &destination)?;
    Ok(destination)
}

pub fn transfer(
    sources: &[PathBuf],
    destination: &Path,
    action: Transfer,
    choice: ConflictChoice,
) -> anyhow::Result<Vec<String>> {
    if !destination.is_dir() {
        anyhow::bail!("Destination is not a directory: {}", destination.display());
    }

    if choice == ConflictChoice::Cancel {
        for source in sources {
            let Some(name) = source.file_name() else {
                continue;
            };
            if path_exists(&destination.join(name)) {
                anyhow::bail!("Transfer cancelled because the destination already exists");
            }
        }
    }

    let mut skipped = Vec::new();
    for source in sources {
        if !path_exists(source) {
            anyhow::bail!("Source no longer exists: {}", source.display());
        }
        let Some(name) = source.file_name() else {
            continue;
        };
        let target = destination.join(name);
        if equivalent_path(source, &target) {
            anyhow::bail!(
                "Source and destination are the same path: {}",
                source.display()
            );
        }
        let source_metadata = fs::symlink_metadata(source)?;
        if source_metadata.is_dir()
            && destination
                .canonicalize()?
                .starts_with(source.canonicalize()?)
        {
            anyhow::bail!("Cannot copy or move a folder into itself");
        }
        if path_exists(&target) {
            match choice {
                ConflictChoice::Cancel => {
                    anyhow::bail!("Transfer cancelled because the destination already exists")
                }
                ConflictChoice::Skip => {
                    skipped.push(name.to_string_lossy().into_owned());
                    continue;
                }
                ConflictChoice::Replace => {
                    replace_transfer(source, &target, action)?;
                    continue;
                }
            }
        }
        match action {
            Transfer::Copy => copy_path(source, &target)?,
            Transfer::Move => move_path(source, &target)?,
        }
    }
    Ok(skipped)
}

pub fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

pub fn delete_to_trash(paths: &[PathBuf]) -> anyhow::Result<()> {
    for path in paths {
        trash::delete(path)?;
    }
    Ok(())
}

pub fn parent(path: &Path) -> Option<PathBuf> {
    path.parent().map(Path::to_path_buf)
}

pub fn resolve_path_input(input: &str, current: &Path, home: &Path) -> PathBuf {
    let path = if input == "~" {
        home.to_path_buf()
    } else if let Some(suffix) = input
        .strip_prefix("~/")
        .or_else(|| input.strip_prefix("~\\"))
    {
        home.join(suffix)
    } else {
        PathBuf::from(input)
    };
    if path.is_absolute() {
        path
    } else {
        current.join(path)
    }
}

fn validate_name(name: &str) -> anyhow::Result<&str> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || Path::new(name).components().count() != 1 {
        anyhow::bail!("Enter a valid single item name");
    }
    Ok(name)
}

pub(crate) fn is_hidden(metadata: &fs::Metadata, name: &str) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x2 != 0 || name.starts_with('.')
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        name.starts_with('.')
    }
}

pub fn sort_entries(entries: &mut [Entry], sort: SortBy, descending: bool) {
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            let ordering = match sort {
                SortBy::Name => natural_name_order(&a.name, &b.name).then(a.name.cmp(&b.name)),
                SortBy::Kind => extension(&a.name)
                    .cmp(&extension(&b.name))
                    .then_with(|| natural_name_order(&a.name, &b.name))
                    .then(a.name.cmp(&b.name)),
                SortBy::Size => a
                    .size
                    .cmp(&b.size)
                    .then_with(|| natural_name_order(&a.name, &b.name))
                    .then(a.name.cmp(&b.name)),
                SortBy::Modified => a
                    .modified
                    .cmp(&b.modified)
                    .then_with(|| natural_name_order(&a.name, &b.name))
                    .then(a.name.cmp(&b.name)),
            };
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        })
    });
}

fn natural_name_order(a: &str, b: &str) -> std::cmp::Ordering {
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    let a = a.as_bytes();
    let b = b.as_bytes();
    let (mut i, mut j) = (0, 0);

    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let a_start = i;
            let b_start = j;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let a_significant = (a_start..i)
                .find(|index| a[*index] != b'0')
                .unwrap_or(i - 1);
            let b_significant = (b_start..j)
                .find(|index| b[*index] != b'0')
                .unwrap_or(j - 1);
            let a_value = &a[a_significant..i];
            let b_value = &b[b_significant..j];
            let order = a_value
                .len()
                .cmp(&b_value.len())
                .then_with(|| a_value.cmp(b_value))
                .then_with(|| (i - a_start).cmp(&(j - b_start)));
            if order != std::cmp::Ordering::Equal {
                return order;
            }
        } else {
            let order = a[i].cmp(&b[j]);
            if order != std::cmp::Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }

    a.len().cmp(&b.len())
}

fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase()
}

fn equivalent_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn remove_path(path: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn copy_path(source: &Path, target: &Path) -> anyhow::Result<()> {
    copy_path_inner(source, target)
}

fn replace_transfer(source: &Path, target: &Path, action: Transfer) -> anyhow::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Destination has no parent directory"))?;
    let transaction = create_staging_directory(parent)?;
    let staged = transaction.join("new-item");
    let backup = transaction.join("previous-item");

    if let Err(error) = copy_path(source, &staged) {
        let _ = fs::remove_dir_all(&transaction);
        return Err(error);
    }

    let had_target = path_exists(target);
    if had_target && let Err(error) = fs::rename(target, &backup) {
        let _ = fs::remove_dir_all(&transaction);
        return Err(error.into());
    }

    if let Err(error) = fs::rename(&staged, target) {
        if had_target && let Err(restore_error) = fs::rename(&backup, target) {
            let _ = fs::remove_dir_all(&transaction);
            anyhow::bail!(
                "Could not install replacement ({error}) or restore the previous item ({restore_error})"
            );
        }
        let _ = fs::remove_dir_all(&transaction);
        return Err(error.into());
    }

    if action == Transfer::Move
        && let Err(error) = remove_path(source)
    {
        let _ = remove_path(target);
        if had_target && let Err(restore_error) = fs::rename(&backup, target) {
            let _ = fs::remove_dir_all(&transaction);
            anyhow::bail!(
                "Could not finish the move ({error}) or restore the previous item ({restore_error})"
            );
        }
        let _ = fs::remove_dir_all(&transaction);
        return Err(error);
    }

    if had_target {
        remove_path(&backup)?;
    }
    fs::remove_dir(&transaction)?;
    Ok(())
}

fn create_staging_directory(parent: &Path) -> anyhow::Result<PathBuf> {
    for _ in 0..100 {
        let id = NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".filebrowser-{}-{id}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("Could not allocate a temporary replacement directory")
}

fn copy_path_inner(source: &Path, target: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(fs::read_link(source)?, target)?;
        }
        #[cfg(windows)]
        {
            let link = fs::read_link(source)?;
            if source.is_dir() {
                std::os::windows::fs::symlink_dir(link, target)?;
            } else {
                std::os::windows::fs::symlink_file(link, target)?;
            }
        }
    } else if metadata.is_dir() {
        fs::create_dir(target)?;
        let result: anyhow::Result<()> = (|| {
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                copy_path(&entry.path(), &target.join(entry.file_name()))?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(target);
        }
        result?;
    } else {
        let mut input = fs::File::open(source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        let result = std::io::copy(&mut input, &mut output)
            .and_then(|_| output.set_permissions(metadata.permissions()));
        if let Err(error) = result {
            drop(output);
            let _ = fs::remove_file(target);
            return Err(error.into());
        }
    }
    Ok(())
}

fn move_path(source: &Path, target: &Path) -> anyhow::Result<()> {
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            copy_path(source, target).map_err(|copy_error| {
                anyhow::anyhow!("Move failed ({rename_error}); fallback copy failed ({copy_error})")
            })?;
            if let Err(error) = remove_path(source) {
                let _ = remove_path(target);
                return Err(error);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_hides_and_sorts_entries() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("z-folder")).unwrap();
        fs::write(temp.path().join("a.txt"), "a").unwrap();
        fs::write(temp.path().join(".hidden"), "h").unwrap();
        let entries = list_dir(temp.path(), false, SortBy::Name, false).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "a.txt");
    }

    #[test]
    fn searches_descendants_without_following_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("target.txt"), "x").unwrap();
        let matches = search(temp.path(), "target", false).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "target.txt");
    }

    #[test]
    fn creates_renames_and_transfers_items() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dst = temp.path().join("dst");
        fs::create_dir(&src).unwrap();
        fs::create_dir(&dst).unwrap();
        let folder = create_folder(&src, "created").unwrap();
        let named = rename(&folder, "renamed").unwrap();
        assert!(named.exists());
        let file = src.join("item.txt");
        fs::write(&file, "value").unwrap();
        transfer(&[file], &dst, Transfer::Copy, ConflictChoice::Cancel).unwrap();
        assert_eq!(fs::read_to_string(dst.join("item.txt")).unwrap(), "value");
    }

    #[test]
    fn skips_conflicting_transfer_targets() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("dst");
        fs::create_dir(&dst).unwrap();
        fs::write(&src, "source").unwrap();
        fs::write(dst.join("src.txt"), "existing").unwrap();
        let skipped = transfer(&[src], &dst, Transfer::Copy, ConflictChoice::Skip).unwrap();
        assert_eq!(skipped, vec!["src.txt"]);
        assert_eq!(fs::read_to_string(dst.join("src.txt")).unwrap(), "existing");
    }

    #[test]
    fn replaces_conflicting_transfer_targets() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("dst");
        fs::create_dir(&dst).unwrap();
        fs::write(&src, "source").unwrap();
        fs::write(dst.join("src.txt"), "existing").unwrap();
        transfer(&[src], &dst, Transfer::Copy, ConflictChoice::Replace).unwrap();
        assert_eq!(fs::read_to_string(dst.join("src.txt")).unwrap(), "source");
    }

    #[test]
    fn replacement_preserves_old_item_until_new_item_is_staged() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("dst");
        fs::create_dir(&dst).unwrap();
        fs::write(&src, "new contents").unwrap();
        fs::write(dst.join("src.txt"), "old contents").unwrap();

        transfer(&[src], &dst, Transfer::Copy, ConflictChoice::Replace).unwrap();

        assert_eq!(
            fs::read_to_string(dst.join("src.txt")).unwrap(),
            "new contents"
        );
        assert_eq!(fs::read_dir(&dst).unwrap().count(), 1);
    }

    #[test]
    fn copy_refuses_to_overwrite_an_item_that_appears_at_its_target() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("dst.txt");
        fs::write(&src, "source").unwrap();
        fs::write(&dst, "destination").unwrap();

        assert!(copy_path(&src, &dst).is_err());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "destination");
    }

    #[test]
    fn move_replacement_removes_source_after_installing_destination() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src.txt");
        let dst = temp.path().join("dst");
        fs::create_dir(&dst).unwrap();
        fs::write(&src, "new contents").unwrap();
        fs::write(dst.join("src.txt"), "old contents").unwrap();

        transfer(
            std::slice::from_ref(&src),
            &dst,
            Transfer::Move,
            ConflictChoice::Replace,
        )
        .unwrap();

        assert!(!path_exists(&src));
        assert_eq!(
            fs::read_to_string(dst.join("src.txt")).unwrap(),
            "new contents"
        );
        assert_eq!(fs::read_dir(&dst).unwrap().count(), 1);
    }

    #[test]
    fn cancel_preflights_all_conflicts_before_transferring_any_item() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.txt");
        let second = temp.path().join("second.txt");
        let dst = temp.path().join("dst");
        fs::create_dir(&dst).unwrap();
        fs::write(&first, "first").unwrap();
        fs::write(&second, "second").unwrap();
        fs::write(dst.join("second.txt"), "existing").unwrap();

        let result = transfer(
            &[first.clone(), second],
            &dst,
            Transfer::Copy,
            ConflictChoice::Cancel,
        );

        assert!(result.is_err());
        assert!(!path_exists(&dst.join("first.txt")));
        assert_eq!(
            fs::read_to_string(dst.join("second.txt")).unwrap(),
            "existing"
        );
    }

    #[test]
    fn refuses_copying_a_folder_into_itself() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("folder");
        fs::create_dir(&folder).unwrap();
        let result = transfer(
            std::slice::from_ref(&folder),
            &folder,
            Transfer::Copy,
            ConflictChoice::Replace,
        );
        assert!(result.is_err());
        assert!(folder.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn search_does_not_follow_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        symlink(&outside, root.join("linked")).unwrap();

        assert!(search(&root, "secret", false).unwrap().is_empty());
    }
}
