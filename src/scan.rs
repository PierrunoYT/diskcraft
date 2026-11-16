//! Parallel filesystem scanner.
//!
//! Directories are walked concurrently with rayon into an owned intermediate tree,
//! which is then flattened into a [`Tree`] with aggregated sizes, sorted by size.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::{Instant, SystemTime};

use rayon::prelude::*;

use crate::tree::{Node, NodeId, Tree};

/// Live counters shared between the scanner and the UI. Also carries the cancel flag.
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    cancel: AtomicBool,
    current: Mutex<String>,
}

impl Progress {
    pub fn cancel(&self) {
        self.cancel.store(true, Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }

    /// Directory most recently entered by any worker.
    pub fn current(&self) -> String {
        self.current.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set_current(&self, path: &Path) {
        // Best effort: never make workers wait on the UI's read.
        if let Ok(mut current) = self.current.try_lock() {
            current.clear();
            current.push_str(&path.to_string_lossy());
        }
    }

    fn error(&self) {
        self.errors.fetch_add(1, Relaxed);
    }
}

struct RawFile {
    name: Box<str>,
    size: u64,
    modified: Option<SystemTime>,
}

struct RawDir {
    name: Box<str>,
    modified: Option<SystemTime>,
    error: bool,
    files: Vec<RawFile>,
    dirs: Vec<RawDir>,
}

struct Walker<'a> {
    progress: &'a Progress,
    /// Device of the scan root; subdirectories on other filesystems are skipped (Unix only).
    #[cfg(unix)]
    root_dev: u64,
}

/// Scans `root` recursively. Returns `Ok(None)` if cancelled through `progress`.
///
/// Symlinks and junctions are not followed and contribute nothing, so no data is
/// counted twice through them. Unreadable directories are kept with `error` set.
pub fn scan(root: &Path, progress: &Progress) -> io::Result<Option<Tree>> {
    let started = Instant::now();
    let meta = fs::metadata(root)?;
    if !meta.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotADirectory, "not a directory"));
    }

    let walker = Walker {
        progress,
        #[cfg(unix)]
        root_dev: std::os::unix::fs::MetadataExt::dev(&meta),
    };

    // Deep hierarchies plus work-stealing nest frames, so give workers generous stacks.
    let pool = rayon::ThreadPoolBuilder::new()
        .stack_size(16 << 20)
        .thread_name(|i| format!("scan-{i}"))
        .build()
        .map_err(io::Error::other)?;

    let tree = pool.install(|| {
        let raw = walker.dir(root, root.display().to_string().into(), meta.modified().ok());
        if progress.is_cancelled() {
            return None;
        }
        let capacity = progress.files.load(Relaxed) + progress.dirs.load(Relaxed);
        let mut nodes = Vec::with_capacity(capacity as usize);
        flatten(raw, None, &mut nodes);
        Some(nodes)
    });

    Ok(tree.map(|nodes| Tree {
        nodes,
        root_path: root.to_path_buf(),
        errors: progress.errors.load(Relaxed),
        elapsed: started.elapsed(),
    }))
}

impl Walker<'_> {
    fn dir(&self, path: &Path, name: Box<str>, modified: Option<SystemTime>) -> RawDir {
        let p = self.progress;
        let mut dir = RawDir { name, modified, error: false, files: Vec::new(), dirs: Vec::new() };
        if p.is_cancelled() {
            return dir;
        }
        p.dirs.fetch_add(1, Relaxed);
        p.set_current(path);

        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(_) => {
                dir.error = true;
                p.error();
                return dir;
            }
        };

        let mut subdirs: Vec<(PathBuf, Box<str>, Option<SystemTime>)> = Vec::new();
        let mut bytes = 0;
        for entry in entries {
            // On Windows, file type and metadata come from the directory listing itself;
            // no extra system call per file.
            let (entry, file_type, meta) = match entry.and_then(|e| {
                let ft = e.file_type()?;
                let meta = e.metadata()?;
                Ok((e, ft, meta))
            }) {
                Ok(x) => x,
                Err(_) => {
                    dir.error = true;
                    p.error();
                    continue;
                }
            };
            if file_type.is_symlink() {
                continue; // includes Windows junctions / mount points
            }
            let name: Box<str> = entry.file_name().to_string_lossy().into();
            if file_type.is_dir() {
                #[cfg(unix)]
                if std::os::unix::fs::MetadataExt::dev(&meta) != self.root_dev {
                    continue;
                }
                subdirs.push((entry.path(), name, meta.modified().ok()));
            } else {
                bytes += meta.len();
                dir.files.push(RawFile { name, size: meta.len(), modified: meta.modified().ok() });
            }
        }
        p.files.fetch_add(dir.files.len() as u64, Relaxed);
        p.bytes.fetch_add(bytes, Relaxed);

        dir.dirs = subdirs
            .into_par_iter()
            .map(|(path, name, modified)| self.dir(&path, name, modified))
            .collect();
        dir
    }
}

/// Moves `raw` into `nodes` depth-first, aggregating totals. Children end up sorted by size, descending.
fn flatten(raw: RawDir, parent: Option<NodeId>, nodes: &mut Vec<Node>) -> NodeId {
    let id = nodes.len() as NodeId;
    nodes.push(Node {
        name: raw.name,
        parent,
        children: Vec::new(),
        size: 0,
        files: 0,
        dirs: 0,
        modified: raw.modified,
        is_dir: true,
        error: raw.error,
    });

    let mut children = Vec::with_capacity(raw.dirs.len() + raw.files.len());
    let (mut size, mut files, mut dirs, mut modified) = (0u64, 0u64, 0u64, raw.modified);

    for sub in raw.dirs {
        let child = flatten(sub, Some(id), nodes);
        let c = &nodes[child as usize];
        size += c.size;
        files += c.files;
        dirs += c.dirs + 1;
        modified = modified.max(c.modified);
        children.push(child);
    }
    for file in raw.files {
        children.push(nodes.len() as NodeId);
        size += file.size;
        files += 1;
        modified = modified.max(file.modified);
        nodes.push(Node {
            name: file.name,
            parent: Some(id),
            children: Vec::new(),
            size: file.size,
            files: 0,
            dirs: 0,
            modified: file.modified,
            is_dir: false,
            error: false,
        });
    }

    children.sort_unstable_by(|&a, &b| nodes[b as usize].size.cmp(&nodes[a as usize].size));
    let node = &mut nodes[id as usize];
    node.children = children;
    node.size = size;
    node.files = files;
    node.dirs = dirs;
    node.modified = modified;
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::ROOT;

    #[test]
    fn aggregates_sizes_and_counts() {
        let root = std::env::temp_dir().join(format!("diskcraft-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
        fs::write(root.join("top.bin"), vec![0u8; 10]).unwrap();
        fs::write(root.join("a/one.TXT"), vec![0u8; 100]).unwrap();
        fs::write(root.join("a/b/two.txt"), vec![0u8; 1000]).unwrap();

        let tree = scan(&root, &Progress::default()).unwrap().unwrap();
        fs::remove_dir_all(&root).unwrap();

        let r = tree.node(ROOT);
        assert_eq!((r.size, r.files, r.dirs), (1110, 3, 3));
        // Largest first: a (1100), top.bin (10), empty (0).
        let names: Vec<&str> = r.children.iter().map(|&c| &*tree.node(c).name).collect();
        assert_eq!(names, ["a", "top.bin", "empty"]);

        let a = r.children[0];
        assert_eq!((tree.node(a).size, tree.node(a).files, tree.node(a).dirs), (1100, 2, 1));
        assert_eq!(tree.path(a), root.join("a"));

        let stats = tree.subtree_stats(ROOT, 2);
        let txt = stats.extensions.iter().find(|e| e.ext == "txt").unwrap();
        assert_eq!((txt.bytes, txt.files), (1100, 2), "extensions are case-folded");
        let largest: Vec<&str> = stats.largest.iter().map(|&n| &*tree.node(n).name).collect();
        assert_eq!(largest, ["two.txt", "one.TXT"]);
    }
}
