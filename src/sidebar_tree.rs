use crate::path_utils::{file_display_name, normalize_slashes};
use std::collections::HashSet;

#[derive(Clone, Debug, Default)]
pub struct FolderTreeNode {
    pub name: String,
    pub path: String,
    pub folders: Vec<FolderTreeNode>,
    pub files: Vec<String>,
    pub note_count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct FileTree {
    pub root_files: Vec<String>,
    pub folders: Vec<FolderTreeNode>,
}

#[derive(Clone, Debug)]
pub enum SidebarEntry {
    Folder {
        path: String,
        name: String,
        depth: usize,
        note_count: usize,
        expanded: bool,
    },
    File {
        path: String,
        name: String,
        depth: usize,
    },
}

#[derive(Clone)]
pub enum SidebarContextMenu {
    Folder { path: String, x: f64, y: f64 },
    File { path: String, x: f64, y: f64 },
}

fn insert_file_into_folders(
    folders: &mut Vec<FolderTreeNode>,
    folder_parts: &[&str],
    file_path: &str,
    prefix: &str,
) {
    if folder_parts.is_empty() {
        return;
    }

    let name = folder_parts[0];
    let path = if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    };

    let idx = if let Some(pos) = folders.iter().position(|f| f.name == name) {
        pos
    } else {
        folders.push(FolderTreeNode {
            name: name.to_string(),
            path: path.clone(),
            folders: Vec::new(),
            files: Vec::new(),
            note_count: 0,
        });
        folders.len() - 1
    };

    if folder_parts.len() == 1 {
        folders[idx].files.push(file_path.to_string());
    } else {
        insert_file_into_folders(
            &mut folders[idx].folders,
            &folder_parts[1..],
            file_path,
            &path,
        );
    }
}

fn finalize_folder_tree(nodes: &mut Vec<FolderTreeNode>) {
    nodes.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    for node in nodes.iter_mut() {
        finalize_folder_tree(&mut node.folders);
        node.files
            .sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
        node.note_count = node.files.len()
            + node
                .folders
                .iter()
                .map(|folder| folder.note_count)
                .sum::<usize>();
    }
}

fn ensure_empty_folder_path(folders: &mut Vec<FolderTreeNode>, path_parts: &[&str], prefix: &str) {
    if path_parts.is_empty() {
        return;
    }
    let name = path_parts[0];
    let path = if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    };
    let idx = if let Some(pos) = folders.iter().position(|f| f.name == name) {
        pos
    } else {
        folders.push(FolderTreeNode {
            name: name.to_string(),
            path: path.clone(),
            folders: Vec::new(),
            files: Vec::new(),
            note_count: 0,
        });
        folders.len() - 1
    };
    if path_parts.len() > 1 {
        ensure_empty_folder_path(&mut folders[idx].folders, &path_parts[1..], &path);
    }
}

pub fn add_empty_dirs_to_tree(tree: &mut FileTree, empty_dirs: &[String]) {
    for d in empty_dirs {
        let parts: Vec<&str> = d.split('/').filter(|s| !s.is_empty()).collect();
        if !parts.is_empty() {
            ensure_empty_folder_path(&mut tree.folders, &parts, "");
        }
    }
    finalize_folder_tree(&mut tree.folders);
}

pub fn build_file_tree(files: &[String]) -> FileTree {
    let mut tree = FileTree::default();
    for raw in files {
        let path = normalize_slashes(raw);
        let parts = path
            .split('/')
            .filter(|segment: &&str| !segment.is_empty())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            continue;
        }
        if parts.len() == 1 {
            tree.root_files.push(path);
            continue;
        }
        insert_file_into_folders(&mut tree.folders, &parts[..parts.len() - 1], &path, "");
    }

    tree.root_files
        .sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
    finalize_folder_tree(&mut tree.folders);
    tree
}

fn collect_sidebar_entries_from_folders(
    nodes: &[FolderTreeNode],
    expanded_folders: &HashSet<String>,
    depth: usize,
    out: &mut Vec<SidebarEntry>,
) {
    for folder in nodes {
        let expanded = expanded_folders.contains(&folder.path);
        out.push(SidebarEntry::Folder {
            path: folder.path.clone(),
            name: folder.name.clone(),
            depth,
            note_count: folder.note_count,
            expanded,
        });
        if expanded {
            collect_sidebar_entries_from_folders(&folder.folders, expanded_folders, depth + 1, out);
            for file_path in &folder.files {
                out.push(SidebarEntry::File {
                    path: file_path.clone(),
                    name: file_display_name(file_path),
                    depth: depth + 1,
                });
            }
        }
    }
}

pub fn build_sidebar_entries(tree: &FileTree, expanded_folders: &HashSet<String>) -> Vec<SidebarEntry> {
    let mut out = Vec::new();
    collect_sidebar_entries_from_folders(&tree.folders, expanded_folders, 0, &mut out);
    for file_path in &tree.root_files {
        out.push(SidebarEntry::File {
            path: file_path.clone(),
            name: file_display_name(file_path),
            depth: 0,
        });
    }
    out
}

fn parent_folder_chain(file_path: &str) -> Vec<String> {
    let normalized = normalize_slashes(file_path);
    let mut parts = normalized
        .split('/')
        .filter(|segment: &&str| !segment.is_empty())
        .collect::<Vec<_>>();
    if parts.len() <= 1 {
        return Vec::new();
    }
    parts.pop();

    let mut out = Vec::new();
    let mut current = String::new();
    for part in parts {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(part);
        out.push(current.clone());
    }
    out
}

pub fn expand_parent_folders(expanded_folders: &mut HashSet<String>, file_path: &str) {
    for folder in parent_folder_chain(file_path) {
        expanded_folders.insert(folder);
    }
}

