//! Headless mode: scan and print a size-sorted tree to stdout.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::format;
use crate::scan::{Progress, scan};
use crate::tree::{NodeId, ROOT, Tree};

pub fn run(args: &[String]) -> ExitCode {
    let mut path = None;
    let mut depth = 2usize;
    let mut top = 10usize;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let number = |v: Option<&String>| v.and_then(|v| v.parse().ok());
        match arg.as_str() {
            "--depth" => match number(it.next()) {
                Some(n) => depth = n,
                None => return usage_error("--depth needs a number"),
            },
            "--top" => match number(it.next()) {
                Some(n) => top = n,
                None => return usage_error("--top needs a number"),
            },
            p if path.is_none() && !p.starts_with("--") => path = Some(PathBuf::from(p)),
            other => return usage_error(&format!("unexpected argument: {other}")),
        }
    }
    let Some(path) = path else {
        return usage_error("missing PATH");
    };
    let root = crate::normalize_path(&path);

    let tree = match scan(&root, &Progress::default()) {
        Ok(Some(tree)) => tree,
        Ok(None) => unreachable!("report scans are never cancelled"),
        Err(e) => {
            eprintln!("error: {}: {e}", root.display());
            return ExitCode::FAILURE;
        }
    };

    let r = tree.node(ROOT);
    println!(
        "{}  {}  ({} files, {} folders)  scanned in {:.2?}{}",
        r.name,
        format::size(r.size),
        format::count(r.files),
        format::count(r.dirs),
        tree.elapsed,
        match tree.errors {
            0 => String::new(),
            n => format!(", {} unreadable", format::count(n)),
        }
    );
    print_children(&tree, ROOT, 1, depth, top);
    ExitCode::SUCCESS
}

fn print_children(tree: &Tree, id: NodeId, level: usize, max_depth: usize, top: usize) {
    if level > max_depth {
        return;
    }
    let parent = tree.node(id);
    for &c in parent.children.iter().take(top) {
        let node = tree.node(c);
        println!(
            "{:>10} {:>6.1}%  {}{}{}{}",
            format::size(node.size),
            format::fraction(node.size, parent.size) * 100.0,
            "  ".repeat(level - 1),
            node.name,
            if node.is_dir { std::path::MAIN_SEPARATOR_STR } else { "" },
            if node.error { "  [unreadable]" } else { "" },
        );
        if node.is_dir {
            print_children(tree, c, level + 1, max_depth, top);
        }
    }
    let rest = &parent.children[parent.children.len().min(top)..];
    if !rest.is_empty() {
        let bytes: u64 = rest.iter().map(|&c| tree.node(c).size).sum();
        println!(
            "{:>10} {:>6.1}%  {}… {} more",
            format::size(bytes),
            format::fraction(bytes, parent.size) * 100.0,
            "  ".repeat(level - 1),
            format::count(rest.len() as u64),
        );
    }
}

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("error: {msg}\n\n{}", crate::USAGE);
    ExitCode::from(2)
}
