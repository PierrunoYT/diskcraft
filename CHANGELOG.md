# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-11-16

### Added

- Parallel directory scanner (rayon) with live progress (files, folders, bytes,
  current path) and cancellation; a cancelled rescan keeps the previous results.
- Folder tree with size, % of parent bar, recursive file and folder counts and
  newest change below each folder; sortable by every column, virtualized rows,
  optional hiding of files, keyboard navigation (arrows, Home/End, PageUp/PageDown).
- Side panel scoped to the selected folder:
  - Squarified, nested treemap: click selects, double-click zooms in,
    right-click or ⬆ zooms out, hover shows path and size.
  - File types: bytes and file count per extension, case-insensitive.
  - Largest files: top 250; clicking one reveals it in the tree.
- Context menu: open in Explorer/Finder/file manager, copy path.
- Toolbar with path entry, folder picker, drive list and Rescan.
- Status bar with totals, scan time and count of unreadable entries.
- Report mode: `diskcraft --report PATH [--depth N] [--top N]` prints a
  size-sorted tree; `--help` prints usage.
- Release builds on Windows run without a console window; report mode
  attaches to the calling console.

### Notes

- Sizes are logical file lengths, not allocated size on disk; 1 KB = 1024 bytes.
- Symlinks and junctions are not followed; hard links count once per link.
- On Unix, scans stay on the root's filesystem.
- Unreadable folders are marked ⚠ and counted in the status bar.
