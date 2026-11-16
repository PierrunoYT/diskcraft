# DiskCraft

TreeSize-style disk usage analyzer written in Rust (egui).

- **Parallel scan** of a folder or drive (rayon), with live progress and cancel.
- **Folder tree** sorted by size: size, allocated size on disk, % of parent bar, recursive file/folder counts,
  last change anywhere below. Sort by any column; files can be hidden.
- **Treemap** (squarified) of the selected folder, nested several levels deep.
  Click selects in the tree, double-click zooms in, right-click zooms out.
- **File types**: bytes and file count per extension below the selected folder.
- **Largest files** below the selected folder.
- Context menu: open in Explorer/Finder/file manager, copy path.
- **Report mode** for terminals and scripts.

## Build

```sh
cargo build --release
```

## Usage

```sh
diskcraft                 # open the GUI
diskcraft D:\             # open the GUI and scan D:\
diskcraft --report D:\Projects --depth 3 --top 15
```

Keyboard in the tree: arrows (←/→ collapse/expand), Home/End, PageUp/PageDown.

## Semantics

- **Size** is the logical file length; **Allocated** is the space actually used on disk
  (cluster-rounded; smaller for compressed/sparse files; 0 for OneDrive/kDrive-style
  online-only placeholders). Percentages and the treemap use Size. 1 KB = 1024 bytes.
- Symlinks and junctions are not followed, so nothing is counted twice through them.
  Hard links are counted once per link.
- On Unix the scan stays on the root's filesystem (no `/proc`, network mounts, …).
- Folders that cannot be read (permissions) are marked ⚠ and counted in the status bar;
  run as administrator to include protected system folders.
