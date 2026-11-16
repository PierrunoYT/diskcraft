//! Flat, index-based representation of a scanned directory tree.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

pub type NodeId = u32;
pub const ROOT: NodeId = 0;

pub struct Node {
    pub name: Box<str>,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    /// Total bytes: the file length, or the recursive sum for directories.
    pub size: u64,
    /// Recursive file count (0 for files).
    pub files: u64,
    /// Recursive subdirectory count (0 for files).
    pub dirs: u64,
    /// File mtime, or the newest mtime anywhere below a directory.
    pub modified: Option<SystemTime>,
    pub is_dir: bool,
    /// The directory (or part of it) could not be read.
    pub error: bool,
}

impl Node {
    /// Extension of a file as written on disk; `None` for directories and extensionless files.
    pub fn extension(&self) -> Option<&str> {
        if self.is_dir {
            return None;
        }
        match self.name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => Some(ext),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Size,
    Files,
    Folders,
    Modified,
}

pub struct ExtStat {
    pub ext: String,
    pub bytes: u64,
    pub files: u64,
}

pub struct SubtreeStats {
    /// Sorted by bytes, descending.
    pub extensions: Vec<ExtStat>,
    /// Largest files, descending.
    pub largest: Vec<NodeId>,
}

pub struct Tree {
    /// `nodes[ROOT]` is the scanned directory; its `name` is the full root path.
    pub nodes: Vec<Node>,
    pub root_path: PathBuf,
    /// Directories or entries that could not be read.
    pub errors: u64,
    pub elapsed: Duration,
}

impl Tree {
    #[inline]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut names: Vec<&str> = self
            .ancestors_inclusive(id)
            .filter(|&a| a != ROOT)
            .map(|a| &*self.node(a).name)
            .collect();
        names.reverse();
        let mut path = self.root_path.clone();
        path.extend(names);
        path
    }

    /// `id`, its parent, …, up to and including the root.
    pub fn ancestors_inclusive(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(Some(id), |&n| self.node(n).parent)
    }

    /// Nearest directory: `id` itself if it is a directory, otherwise its parent.
    pub fn dir_of(&self, id: NodeId) -> NodeId {
        let node = self.node(id);
        if node.is_dir { id } else { node.parent.unwrap_or(ROOT) }
    }

    pub fn sort_children(&mut self, id: NodeId, key: SortKey, descending: bool) {
        let mut children = std::mem::take(&mut self.nodes[id as usize].children);
        children.sort_unstable_by(|&a, &b| {
            let (a, b) = (self.node(a), self.node(b));
            let ord = match key {
                SortKey::Name => b.is_dir.cmp(&a.is_dir).then_with(|| cmp_name(a, b)),
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Files => a.files.cmp(&b.files),
                SortKey::Folders => a.dirs.cmp(&b.dirs),
                SortKey::Modified => a.modified.cmp(&b.modified),
            };
            let ord = if descending { ord.reverse() } else { ord };
            ord.then_with(|| cmp_name(a, b))
        });
        self.nodes[id as usize].children = children;
    }

    /// File-type breakdown and the `top_n` largest files below `id`.
    pub fn subtree_stats(&self, id: NodeId, top_n: usize) -> SubtreeStats {
        let mut by_ext: HashMap<String, (u64, u64)> = HashMap::new();
        let mut heap: BinaryHeap<Reverse<(u64, NodeId)>> = BinaryHeap::with_capacity(top_n + 1);
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            let node = self.node(n);
            if node.is_dir {
                stack.extend_from_slice(&node.children);
                continue;
            }
            let ext = node.extension().unwrap_or("");
            // Avoid allocating for the common already-lower-case case.
            let entry = if ext.bytes().any(|b| b.is_ascii_uppercase()) {
                by_ext.entry(ext.to_ascii_lowercase()).or_default()
            } else if let Some(e) = by_ext.get_mut(ext) {
                e
            } else {
                by_ext.entry(ext.to_owned()).or_default()
            };
            entry.0 += node.size;
            entry.1 += 1;

            if top_n > 0 {
                heap.push(Reverse((node.size, n)));
                if heap.len() > top_n {
                    heap.pop();
                }
            }
        }

        let mut extensions: Vec<ExtStat> = by_ext
            .into_iter()
            .map(|(ext, (bytes, files))| ExtStat { ext, bytes, files })
            .collect();
        extensions.sort_unstable_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.ext.cmp(&b.ext)));

        let mut largest: Vec<(u64, NodeId)> = heap.into_iter().map(|Reverse(e)| e).collect();
        largest.sort_unstable_by(|a, b| b.cmp(a));

        SubtreeStats {
            extensions,
            largest: largest.into_iter().map(|(_, n)| n).collect(),
        }
    }
}

/// Case-insensitive name order, without allocating.
fn cmp_name(a: &Node, b: &Node) -> Ordering {
    if a.name.is_ascii() && b.name.is_ascii() {
        a.name
            .bytes()
            .map(|c| c.to_ascii_lowercase())
            .cmp(b.name.bytes().map(|c| c.to_ascii_lowercase()))
    } else {
        a.name
            .chars()
            .flat_map(char::to_lowercase)
            .cmp(b.name.chars().flat_map(char::to_lowercase))
    }
}
