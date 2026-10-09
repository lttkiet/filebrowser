use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use crate::{core::{self, Entry, SortBy}, display};

const MAX_FILE_BYTES: u64 = 1_048_576;
const MAX_INDEX_BYTES: u64 = 134_217_728;
const MAX_INDEX_FILES: usize = 100_000;

#[derive(Clone)]
struct IndexedFile {
    entry: Entry,
    searchable_text: String,
}

#[derive(Clone)]
pub struct ContentIndex {
    root: PathBuf,
    files: Vec<IndexedFile>,
    indexed_bytes: u64,
    skipped_files: usize,
}

impl ContentIndex {
    pub fn build(root: &Path, show_hidden: bool) -> anyhow::Result<Self> {
        let metadata = fs::symlink_metadata(root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            anyhow::bail!("Choose a regular folder to index");
        }
        let mut index = Self {
            root: root.to_path_buf(),
            files: Vec::new(),
            indexed_bytes: 0,
            skipped_files: 0,
        };
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = fs::read_dir(&directory) else {
                continue;
            };
            for result in entries {
                let Ok(entry) = result else { continue };
                let path = entry.path();
                let Ok(metadata) = fs::symlink_metadata(&path) else {
                    continue;
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                if !show_hidden && core::is_hidden(&metadata, &name) {
                    continue;
                }
                if metadata.is_dir() && !metadata.file_type().is_symlink() {
                    pending.push(path);
                    continue;
                }
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.len() > MAX_FILE_BYTES
                    || index.files.len() >= MAX_INDEX_FILES
                {
                    index.skipped_files += 1;
                    continue;
                }
                let mut text = String::new();
                let read = fs::File::open(&path)
                    .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text));
                if read.is_err() || text.len() as u64 > MAX_FILE_BYTES {
                    index.skipped_files += 1;
                    continue;
                }
                let searchable_text = text.to_lowercase();
                let normalized_bytes = searchable_text.len() as u64;
                if index.indexed_bytes.saturating_add(normalized_bytes) > MAX_INDEX_BYTES {
                    index.skipped_files += 1;
                    continue;
                }
                index.indexed_bytes += normalized_bytes;
                index.files.push(IndexedFile {
                    entry: Entry {
                        path,
                        name,
                        is_dir: false,
                        is_symlink: false,
                        size: metadata.len(),
                        modified: metadata.modified().ok(),
                    },
                    searchable_text,
                });
            }
        }
        Ok(index)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn summary(&self) -> String {
        format!(
            "Indexed {} text files ({}); skipped {}",
            self.files.len(),
            display::format_size(self.indexed_bytes),
            self.skipped_files
        )
    }

    pub fn search(
        &self,
        query: &str,
        sort: SortBy,
        descending: bool,
    ) -> Vec<Entry> {
        let query = query.to_lowercase();
        let mut entries = self
            .files
            .iter()
            .filter(|file| file.searchable_text.contains(&query))
            .map(|file| file.entry.clone())
            .collect::<Vec<_>>();
        core::sort_entries(&mut entries, sort, descending);
        entries
    }
}
