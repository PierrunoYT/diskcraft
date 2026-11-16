//! Flat, index-based representation of a scanned directory tree.
//!
//! Memory layout is tuned for drives with millions of entries: nodes are fixed-size
//! (56 bytes), names live in one shared buffer, and a directory's children occupy a
//! contiguous slice of one shared index array instead of a `Vec` per node.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub type NodeId = u32;
pub const ROOT: NodeId = 0;
const NO_PARENT: NodeId = NodeId::MAX;
const NO_TIME: i64 = i64::MIN;

pub struct Node {
    name_offset: u64,
    /// Total bytes: the file length, or the recursive sum for directories.
    pub size: u64,
    /// Bytes allocated on disk (see [`crate::listing::Item::allocated`]), summed for directories.
    pub allocated: u64,
    /// Unix seconds of the file mtime, or the newest mtime below a directory.
    mtime: i64,
    parent: NodeId,
    first_child: u32,
    child_count: u32,
    /// Recursive file count (0 for files).
    pub files: u32,
    /// Recursive subdirectory count (0 for files).
    pub dirs: u32,
    name_len: u16,
    pub is_dir: bool,
    /// The directory (or part of it) could not be read.
    pub error: bool,
}

const _: () = assert!(std::mem::size_of::<Node>() == 56);

impl Node {
    pub fn parent(&self) -> Option<NodeId> {
        (self.parent != NO_PARENT).then_some(self.parent)
    }

    pub fn modified(&self) -> Option<SystemTime> {
        match self.mtime {
            NO_TIME => None,
            s if s >= 0 => Some(UNIX_EPOCH + Duration::from_secs(s as u64)),
            s => UNIX_EPOCH.checked_sub(Duration::from_secs(s.unsigned_abs())),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Size,
    Allocated,
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
    /// Sorted by allocated bytes, descending.
    pub extensions: Vec<ExtStat>,
    /// Files using the most disk space, descending.
    pub largest: Vec<NodeId>,
}

pub struct Tree {
    /// `nodes[ROOT]` is the scanned directory; its name is the full root path.
    nodes: Vec<Node>,
    names: String,
    /// Children of node `n` are `child_order[first_child .. first_child + child_count]`.
    child_order: Vec<NodeId>,
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

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn name(&self, id: NodeId) -> &str {
        let n = self.node(id);
        &self.names[n.name_offset as usize..n.name_offset as usize + n.name_len as usize]
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        let n = self.node(id);
        &self.child_order[n.first_child as usize..(n.first_child + n.child_count) as usize]
    }

    /// Extension of a file as written on disk; `None` for directories and extensionless files.
    pub fn extension(&self, id: NodeId) -> Option<&str> {
        if self.node(id).is_dir {
            return None;
        }
        match self.name(id).rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => Some(ext),
            _ => None,
        }
    }

    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut names: Vec<&str> = self
            .ancestors_inclusive(id)
            .filter(|&a| a != ROOT)
            .map(|a| self.name(a))
            .collect();
        names.reverse();
        let mut path = self.root_path.clone();
        path.extend(names);
        path
    }

    /// `id`, its parent, …, up to and including the root.
    pub fn ancestors_inclusive(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(Some(id), |&n| self.node(n).parent())
    }

    /// Nearest directory: `id` itself if it is a directory, otherwise its parent.
    pub fn dir_of(&self, id: NodeId) -> NodeId {
        let node = self.node(id);
        if node.is_dir { id } else { node.parent().unwrap_or(ROOT) }
    }

    pub fn sort_children(&mut self, id: NodeId, key: SortKey, descending: bool) {
        let n = &self.nodes[id as usize];
        let range = n.first_child as usize..(n.first_child + n.child_count) as usize;
        let (nodes, names) = (&self.nodes, &self.names);
        let name = |i: NodeId| {
            let n = &nodes[i as usize];
            &names[n.name_offset as usize..n.name_offset as usize + n.name_len as usize]
        };
        self.child_order[range].sort_unstable_by(|&a, &b| {
            let (x, y) = (&nodes[a as usize], &nodes[b as usize]);
            let ord = match key {
                SortKey::Name => y.is_dir.cmp(&x.is_dir).then_with(|| cmp_name(name(a), name(b))),
                SortKey::Size => x.size.cmp(&y.size),
                SortKey::Allocated => x.allocated.cmp(&y.allocated),
                SortKey::Files => x.files.cmp(&y.files),
                SortKey::Folders => x.dirs.cmp(&y.dirs),
                SortKey::Modified => x.mtime.cmp(&y.mtime),
            };
            let ord = if descending { ord.reverse() } else { ord };
            ord.then_with(|| cmp_name(name(a), name(b)))
        });
    }

    /// File-type breakdown and the `top_n` files using the most disk space below `id`.
    /// Weighted by allocated size, so cloud-only placeholders do not dominate.
    pub fn subtree_stats(&self, id: NodeId, top_n: usize) -> SubtreeStats {
        let mut by_ext: HashMap<String, (u64, u64)> = HashMap::new();
        let mut heap: BinaryHeap<Reverse<(u64, NodeId)>> = BinaryHeap::with_capacity(top_n + 1);
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            let node = self.node(n);
            if node.is_dir {
                stack.extend_from_slice(self.children(n));
                continue;
            }
            let ext = self.extension(n).unwrap_or("");
            // Avoid allocating for the common already-lower-case case.
            let entry = if ext.bytes().any(|b| b.is_ascii_uppercase()) {
                by_ext.entry(ext.to_ascii_lowercase()).or_default()
            } else if let Some(e) = by_ext.get_mut(ext) {
                e
            } else {
                by_ext.entry(ext.to_owned()).or_default()
            };
            entry.0 += node.allocated;
            entry.1 += 1;

            if top_n > 0 {
                heap.push(Reverse((node.allocated, n)));
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
fn cmp_name(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        a.bytes()
            .map(|c| c.to_ascii_lowercase())
            .cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
    } else {
        a.chars()
            .flat_map(char::to_lowercase)
            .cmp(b.chars().flat_map(char::to_lowercase))
    }
}

pub fn unix_secs(time: Option<SystemTime>) -> i64 {
    match time.map(|t| t.duration_since(UNIX_EPOCH)) {
        None => NO_TIME,
        Some(Ok(d)) => d.as_secs().min(i64::MAX as u64) as i64,
        Some(Err(e)) => -(e.duration().as_secs().min(i64::MAX as u64) as i64),
    }
}

/// Fields of a new entry; aggregates of directories are filled in by [`TreeBuilder::finish_dir`].
pub struct Entry<'a> {
    pub name: &'a str,
    pub size: u64,
    pub allocated: u64,
    pub mtime: i64,
    pub is_dir: bool,
    pub error: bool,
}

/// Builds a [`Tree`] top-down: create a directory's children as one contiguous block,
/// then descend into each child directory, then `finish_dir` to aggregate.
pub struct TreeBuilder {
    nodes: Vec<Node>,
    names: String,
}

impl TreeBuilder {
    pub fn new(capacity: usize, root: Entry) -> Self {
        let mut b = Self { nodes: Vec::with_capacity(capacity), names: String::new() };
        b.push(root, NO_PARENT);
        b
    }

    fn push(&mut self, e: Entry, parent: NodeId) -> NodeId {
        let id = self.nodes.len() as NodeId;
        let name_offset = self.names.len() as u64;
        // A root path may exceed u16; components never do (255 UTF-16 units).
        let name = &e.name[..floor_char_boundary(e.name, u16::MAX as usize)];
        self.names.push_str(name);
        self.nodes.push(Node {
            name_offset,
            size: e.size,
            allocated: e.allocated,
            mtime: e.mtime,
            parent,
            first_child: 0,
            child_count: 0,
            files: 0,
            dirs: 0,
            name_len: name.len() as u16,
            is_dir: e.is_dir,
            error: e.error,
        });
        id
    }

    /// Appends all children of `parent` (which must have none yet); returns the first child's id.
    pub fn add_children<'a>(&mut self, parent: NodeId, children: impl IntoIterator<Item = Entry<'a>>) -> NodeId {
        let first = self.nodes.len() as NodeId;
        for child in children {
            self.push(child, parent);
        }
        let count = self.nodes.len() as u32 - first;
        let node = &mut self.nodes[parent as usize];
        node.first_child = first;
        node.child_count = count;
        first
    }

    /// Sums sizes, counts and newest mtime of `dir`'s children (already finished) into `dir`.
    pub fn finish_dir(&mut self, dir: NodeId) {
        let node = &self.nodes[dir as usize];
        let range = node.first_child as usize..(node.first_child + node.child_count) as usize;
        let (mut size, mut allocated, mut files, mut dirs, mut mtime) = (0u64, 0u64, 0u32, 0u32, node.mtime);
        for c in &self.nodes[range] {
            size += c.size;
            allocated += c.allocated;
            mtime = mtime.max(c.mtime);
            if c.is_dir {
                files += c.files;
                dirs += c.dirs + 1;
            } else {
                files += 1;
            }
        }
        let node = &mut self.nodes[dir as usize];
        (node.size, node.allocated, node.files, node.dirs, node.mtime) = (size, allocated, files, dirs, mtime);
    }

    pub fn finish(mut self, root_path: PathBuf, errors: u64, elapsed: Duration) -> Tree {
        self.nodes.shrink_to_fit();
        self.names.shrink_to_fit();
        let mut tree = Tree {
            child_order: (0..self.nodes.len() as NodeId).collect(),
            nodes: self.nodes,
            names: self.names,
            root_path,
            errors,
            elapsed,
        };
        for id in 0..tree.nodes.len() as NodeId {
            if tree.node(id).child_count > 1 {
                tree.sort_children(id, SortKey::Allocated, true);
            }
        }
        tree
    }
}

fn floor_char_boundary(s: &str, max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    (0..=max).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0)
}
