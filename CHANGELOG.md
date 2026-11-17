# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- App logo (a disk platter split into a colorful treemap), shown in the window and taskbar and
  embedded in the Windows `.exe` by `build.rs`.
- README in the style of the other Pierruno apps: logo, tagline, badges, quick links, a platform
  table, report mode example and how sizes are counted. The GitHub repository has a description
  and topics.
- **Allocated** (size on disk) column, sortable, also in the status bar, the treemap tooltip and
  `--report` output. It is read from the directory listing itself (`FILE_FULL_DIR_INFO` on
  Windows, `st_blocks` on Unix), so scans are no slower. Online-only cloud placeholders show
  their real footprint (0).
- **🛡 Restart as administrator** (Windows) next to the count of unreadable folders: relaunches
  through UAC and rescans the same folder. Elevated scans enable the backup privilege
  (`SeBackupPrivilege`), so folders whose ACLs exclude administrators are listed as well.

### Changed

- Disk usage is ranked by allocated size instead of logical size: % of parent, the default sort,
  treemap areas, file types, largest files, the side panel total and `--report` percentages.
  Online-only placeholders (OneDrive, kDrive) no longer appear to fill the disk; an 821 GB kDrive
  folder using 1 GB now shows about 0 %.
- Much lower memory use. The GUI renders with OpenGL (glow) instead of wgpu, which drops the idle
  footprint from about 550 MB to 140 MB. Scanned trees use fixed-size nodes, one shared name
  buffer and one shared child index, and the scanner no longer allocates per file: peak memory
  for a 460k-entry scan fell from 101 MB to 60 MB.

## [0.1.0] - 2026-11-16

### Added

- DiskCraft: a TreeSize-style disk usage analyzer (egui) for Windows, Linux and macOS.
- Parallel directory scanner (rayon) with live progress (files, folders, bytes, current path)
  and Cancel; a cancelled rescan keeps the previous results.
- Folder tree with size, a % of parent bar, recursive file and folder counts and the newest
  change below each folder. Sortable by every column, virtualized rows, optional hiding of files,
  keyboard navigation (arrows, Home/End, PageUp/PageDown).
- Side panel scoped to the selected folder:
  - Squarified, nested treemap: click selects, double-click zooms in, right-click or **⬆** zooms
    out, hovering shows path and size.
  - File types: bytes and file count per extension, case-insensitive.
  - Largest files: the top 250; clicking one reveals it in the tree.
- Context menu: open in Explorer, Finder or the file manager (`xdg-open`), copy path, show in
  side panel.
- Toolbar with path entry, folder picker, Drives menu (drive letters on Windows, `/` and the home
  folder elsewhere) and Scan/Rescan.
- Status bar with totals, scan time and the count of unreadable entries; unreadable folders are
  marked ⚠.
- Report mode: `diskcraft --report PATH [--depth N] [--top N]` prints a size-sorted tree;
  `--help` prints usage. A bare drive such as `C:` means its root.
- Release builds on Windows run without a console window; report mode attaches to the calling
  console.
- Symlinks and junctions are not followed and hard links count once per link. On Unix, scans
  stay on the starting folder's file system.
