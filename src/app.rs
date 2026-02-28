use crate::editor_core::{
    apply_markdown_command, ChangeOrigin, EditorSnapshot, MarkdownCommand, Selection, TextChange,
    Transaction,
};
use js_sys::{Object, Reflect};
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos::web_sys::{HtmlElement, Node};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

#[derive(Serialize)]
struct ReadDirArgs<'a> {
    path: &'a str,
}
#[derive(Deserialize)]
struct ReadDirResult {
    notes: Vec<String>,
    empty_dirs: Vec<String>,
}
#[derive(Serialize)]
struct ReadFileArgs<'a> {
    path: &'a str,
}
#[derive(Serialize)]
struct WriteFileArgs<'a> {
    path: &'a str,
    content: &'a str,
}
#[derive(Serialize)]
struct SaveSettingsArgs<'a> {
    vault_path: &'a str,
    settings: &'a str,
}
#[derive(Serialize)]
struct VaultPathArgs<'a> {
    vault_path: &'a str,
}
#[derive(Serialize)]
struct RenameNoteArgs<'a> {
    vault_path: &'a str,
    old_path: &'a str,
    new_path: &'a str,
}

#[derive(Deserialize, Clone, Debug)]
struct VaultNote {
    path: String,
    content: String,
}

#[derive(Deserialize, Clone, Debug)]
struct VaultImportReport {
    success: bool,
    cancelled: bool,
    message: String,
    source_vault: Option<String>,
    destination_vault: Option<String>,
    scanned_notes: usize,
    imported_notes: usize,
    scanned_images: usize,
    imported_images: usize,
    renamed_notes: usize,
}

#[derive(Deserialize, Clone, Debug, Default)]
struct VaultSessionState {
    open_vaults: Vec<String>,
    active_vault: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct AppSettings {
    font_size: u32,
    accent_color: String,
    bg_primary: String,
    bg_secondary: String,
    text_primary: String,
    md_h1_color: String,
    md_h2_color: String,
    md_h3_color: String,
    md_h4_color: String,
    md_bold_color: String,
    md_italic_color: String,
    md_code_bg: String,
    md_code_text: String,
    md_quote_color: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            font_size: 16,
            accent_color: "#6366f1".to_string(),
            bg_primary: "#ffffff".to_string(),
            bg_secondary: "#f4f5f7".to_string(),
            text_primary: "#1a1a1a".to_string(),
            md_h1_color: "#1a1a1a".to_string(),
            md_h2_color: "#1a1a1a".to_string(),
            md_h3_color: "#1a1a1a".to_string(),
            md_h4_color: "#1a1a1a".to_string(),
            md_bold_color: "#4f46e5".to_string(),
            md_italic_color: "#1a1a1a".to_string(),
            md_code_bg: "#e9ecef".to_string(),
            md_code_text: "#1a1a1a".to_string(),
            md_quote_color: "#9ca3af".to_string(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct HeadingCache {
    level: u8,
    text: String,
    line: usize,
}

#[derive(Clone, Debug, Default)]
struct FileCache {
    headings: Vec<HeadingCache>,
    tags: Vec<String>,
    links: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct MetadataCacheState {
    file_cache: HashMap<String, FileCache>,
    resolved_links: HashMap<String, HashMap<String, usize>>,
    unresolved_links: HashMap<String, HashMap<String, usize>>,
    backlinks: HashMap<String, Vec<String>>,
    tags_index: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Default)]
struct FolderTreeNode {
    name: String,
    path: String,
    folders: Vec<FolderTreeNode>,
    files: Vec<String>,
    note_count: usize,
}

#[derive(Clone, Debug, Default)]
struct FileTree {
    root_files: Vec<String>,
    folders: Vec<FolderTreeNode>,
}

#[derive(Clone, Debug)]
enum SidebarEntry {
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
enum SidebarContextMenu {
    Folder { path: String, x: f64, y: f64 },
    File { path: String, x: f64, y: f64 },
}

struct InlineMatch {
    start: usize,
    end: usize,
    inner_start: usize,
    inner_end: usize,
    open_len: usize,
    close_len: usize,
    class: &'static str,
    hide_tokens: bool,
    preview_html: Option<String>,
}

struct ImageRenderContext<'a> {
    vault_path: &'a str,
    current_file: &'a str,
    cache: &'a HashMap<String, String>,
}

fn is_escaped_at(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return false;
    }
    let mut cursor = idx;
    let mut slash_count = 0usize;
    while cursor > 0 {
        cursor -= 1;
        if bytes[cursor] == b'\\' {
            slash_count += 1;
        } else {
            break;
        }
    }
    slash_count % 2 == 1
}

fn find_delimiter_positions(text: &str, delimiter: &str) -> Vec<usize> {
    let marker = delimiter.as_bytes();
    if marker.is_empty() {
        return Vec::new();
    }

    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx + marker.len() <= bytes.len() {
        if &bytes[idx..idx + marker.len()] == marker && !is_escaped_at(bytes, idx) {
            out.push(idx);
            idx += marker.len();
        } else {
            idx += 1;
        }
    }
    out
}

fn collect_delimited_matches(
    text: &str,
    delimiter: &str,
    class: &'static str,
    hide_tokens: bool,
) -> Vec<InlineMatch> {
    let token_len = delimiter.len();
    if token_len == 0 {
        return Vec::new();
    }

    let token_positions = find_delimiter_positions(text, delimiter);
    let mut out = Vec::new();
    let mut pending_open: Option<usize> = None;
    for token in token_positions {
        if let Some(open) = pending_open.take() {
            let close = token;
            out.push(InlineMatch {
                start: open,
                end: close + token_len,
                inner_start: open + token_len,
                inner_end: close,
                open_len: token_len,
                close_len: token_len,
                class,
                hide_tokens,
                preview_html: None,
            });
        } else {
            pending_open = Some(token);
        }
    }

    if let Some(open) = pending_open {
        out.push(InlineMatch {
            start: open,
            end: text.len(),
            // Keep unmatched opening markers visible to avoid caret drift while
            // users are still typing the closing token.
            inner_start: open,
            inner_end: text.len(),
            open_len: 0,
            close_len: 0,
            class,
            hide_tokens,
            preview_html: None,
        });
    }

    out
}

fn overlaps_existing(matches: &[InlineMatch], start: usize, end: usize) -> bool {
    matches.iter().any(|x| start < x.end && end > x.start)
}

fn push_non_overlapping(matches: &mut Vec<InlineMatch>, candidate: InlineMatch) {
    if !overlaps_existing(matches, candidate.start, candidate.end) {
        matches.push(candidate);
    }
}

fn wrap_line(class: &str, html: String) -> String {
    format!("<span class=\"{class}\">{html}</span>")
}

fn code_fence_open(line: &str) -> Option<(u8, usize)> {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let marker = bytes[0];
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let mut len = 0usize;
    while len < bytes.len() && bytes[len] == marker {
        len += 1;
    }
    if len >= 3 {
        Some((marker, len))
    } else {
        None
    }
}

fn code_fence_close(line: &str, marker: u8, min_len: usize) -> bool {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() < min_len || bytes.first().copied() != Some(marker) {
        return false;
    }
    let mut len = 0usize;
    while len < bytes.len() && bytes[len] == marker {
        len += 1;
    }
    if len < min_len {
        return false;
    }
    trimmed[len..].trim().is_empty()
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn normalize_rel_path(path: &str) -> String {
    path.trim().replace('\\', "/").trim_matches('/').to_string()
}

fn vault_display_name(path: &str) -> String {
    let normalized = normalize_slashes(path.trim().trim_end_matches('/'));
    normalized
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("Vault")
        .to_string()
}

fn file_display_name(path: &str) -> String {
    let normalized = normalize_slashes(path);
    normalized
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
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

fn add_empty_dirs_to_tree(tree: &mut FileTree, empty_dirs: &[String]) {
    for d in empty_dirs {
        let parts: Vec<&str> = d.split('/').filter(|s| !s.is_empty()).collect();
        if !parts.is_empty() {
            ensure_empty_folder_path(&mut tree.folders, &parts, "");
        }
    }
    finalize_folder_tree(&mut tree.folders);
}

fn build_file_tree(files: &[String]) -> FileTree {
    let mut tree = FileTree::default();
    for raw in files {
        let path = normalize_slashes(raw);
        let parts = path
            .split('/')
            .filter(|segment| !segment.is_empty())
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

fn build_sidebar_entries(tree: &FileTree, expanded_folders: &HashSet<String>) -> Vec<SidebarEntry> {
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
        .filter(|segment| !segment.is_empty())
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

fn expand_parent_folders(expanded_folders: &mut HashSet<String>, file_path: &str) {
    for folder in parent_folder_chain(file_path) {
        expanded_folders.insert(folder);
    }
}

fn normalize_pasted_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\u{00A0}', " ")
}

fn escape_html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn normalize_slashes(path: &str) -> String {
    path.replace('\\', "/")
}

fn collapse_path(path: &str) -> String {
    let normalized = normalize_slashes(path);
    let bytes = normalized.as_bytes();
    let (prefix, rest) = if normalized.starts_with('/') {
        (
            "/".to_string(),
            normalized.trim_start_matches('/').to_string(),
        )
    } else if bytes.len() >= 2 && bytes[1] == b':' {
        let drive = normalized[..2].to_string();
        let tail = normalized[2..].trim_start_matches('/').to_string();
        (format!("{drive}/"), tail)
    } else {
        (String::new(), normalized)
    };

    let mut parts: Vec<&str> = Vec::new();
    for segment in rest.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            if !parts.is_empty() {
                parts.pop();
            }
            continue;
        }
        parts.push(segment);
    }

    let joined = parts.join("/");
    if prefix.is_empty() {
        joined
    } else {
        format!("{prefix}{joined}")
    }
}

fn strip_wiki_target(raw: &str) -> String {
    raw.trim()
        .split('|')
        .next()
        .unwrap_or_default()
        .split('#')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn strip_markdown_image_target(raw: &str) -> String {
    let trimmed = raw.trim();
    let base = if trimmed.starts_with('<') {
        trimmed
            .trim_start_matches('<')
            .split('>')
            .next()
            .unwrap_or_default()
            .trim()
    } else {
        trimmed.split_whitespace().next().unwrap_or_default().trim()
    };
    base.split('#')
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn image_extension(path: &str) -> Option<String> {
    let normalized = normalize_slashes(path);
    let ext = Path::new(&normalized)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    Some(ext)
}

fn is_supported_inline_image_path(path: &str) -> bool {
    matches!(
        image_extension(path).as_deref(),
        Some(
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "bmp"
                | "svg"
                | "tif"
                | "tiff"
                | "ico"
                | "avif"
                | "heic"
                | "heif"
        )
    )
}

fn image_mime_for_path(path: &str) -> &'static str {
    match image_extension(path).as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        Some("tif") | Some("tiff") => "image/tiff",
        Some("ico") => "image/x-icon",
        Some("avif") => "image/avif",
        Some("heic") => "image/heic",
        Some("heif") => "image/heif",
        _ => "application/octet-stream",
    }
}

fn looks_like_external_url(target: &str) -> bool {
    let t = target.trim().to_ascii_lowercase();
    t.contains("://") || t.starts_with("data:")
}

fn current_note_dir(note_path: &str) -> String {
    let normalized = normalize_slashes(note_path);
    normalized
        .rsplit_once('/')
        .map(|(dir, _)| dir.to_string())
        .unwrap_or_default()
}

fn is_path_within(base: &str, candidate: &str) -> bool {
    let base_norm = collapse_path(base).trim_end_matches('/').to_string();
    let candidate_norm = collapse_path(candidate);
    if base_norm.is_empty() {
        return false;
    }
    candidate_norm == base_norm || candidate_norm.starts_with(&format!("{base_norm}/"))
}

fn image_local_candidates(vault_path: &str, note_path: &str, target: &str) -> Vec<String> {
    if target.is_empty() || !is_supported_inline_image_path(target) {
        return Vec::new();
    }
    if looks_like_external_url(target) {
        return Vec::new();
    }

    let mut out = Vec::new();
    let target_norm = normalize_slashes(target.trim());
    let vault_norm = collapse_path(vault_path);

    let is_windows_abs = {
        let b = target_norm.as_bytes();
        b.len() >= 2 && b[1] == b':'
    };
    if is_windows_abs {
        // Keep auto-render strictly within the active vault.
        return Vec::new();
    }
    if target_norm.starts_with('/') {
        out.push(collapse_path(&format!(
            "{vault_norm}/{}",
            target_norm.trim_start_matches('/')
        )));
        return out;
    }

    let note_dir = current_note_dir(note_path);

    if !note_dir.is_empty() {
        out.push(collapse_path(&format!(
            "{vault_norm}/{note_dir}/{target_norm}"
        )));
    }
    out.push(collapse_path(&format!("{vault_norm}/{target_norm}")));
    out.retain(|candidate| is_path_within(&vault_norm, candidate));
    out.sort();
    out.dedup();
    out
}

fn collect_image_targets_for_note(text: &str) -> Vec<(String, bool)> {
    static RE_EMBED: OnceLock<Regex> = OnceLock::new();
    static RE_MD_IMAGE: OnceLock<Regex> = OnceLock::new();

    let re_embed = RE_EMBED.get_or_init(|| Regex::new(r"!\[\[([^\]\n]+)\]\]").unwrap());
    let re_md_image =
        RE_MD_IMAGE.get_or_init(|| Regex::new(r"!\[([^\]\n]*)\]\(([^)\n]+)\)").unwrap());

    let mut out = Vec::new();
    for cap in re_embed.captures_iter(text) {
        let raw = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        out.push((strip_wiki_target(raw), true));
    }
    for cap in re_md_image.captures_iter(text) {
        let raw = cap.get(2).map(|m| m.as_str()).unwrap_or_default();
        out.push((strip_markdown_image_target(raw), false));
    }
    out
}

fn resolve_image_preview_html(
    ctx: Option<&ImageRenderContext>,
    target: &str,
    alt: Option<&str>,
) -> Option<String> {
    if target.is_empty() || !is_supported_inline_image_path(target) {
        return None;
    }

    let src = if looks_like_external_url(target) {
        target.to_string()
    } else {
        let ctx = ctx?;
        let candidates = image_local_candidates(ctx.vault_path, ctx.current_file, target);
        let path = candidates
            .into_iter()
            .find(|candidate| ctx.cache.contains_key(candidate))?;
        ctx.cache.get(&path)?.to_string()
    };

    let alt = alt.unwrap_or_default();
    Some(format!(
        "<span class=\"md-inline-image-wrap\" contenteditable=\"false\"><img class=\"md-inline-image\" src=\"{}\" alt=\"{}\"/></span>",
        escape_html_attr(&src),
        escape_html_attr(alt)
    ))
}

fn is_selection_navigation_key(key: &str) -> bool {
    matches!(
        key,
        "ArrowLeft"
            | "ArrowRight"
            | "ArrowUp"
            | "ArrowDown"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
            | "Tab"
    )
}

fn collect_text_nodes(node: &Node, out: &mut Vec<Node>) {
    if node.node_type() == Node::TEXT_NODE {
        out.push(node.clone());
        return;
    }
    let children = node.child_nodes();
    for idx in 0..children.length() {
        if let Some(child) = children.item(idx) {
            collect_text_nodes(&child, out);
        }
    }
}

fn node_text_len(node: &Node) -> usize {
    if node.node_type() == Node::TEXT_NODE {
        return node.node_value().unwrap_or_default().len();
    }
    let mut total = 0usize;
    let children = node.child_nodes();
    for idx in 0..children.length() {
        if let Some(child) = children.item(idx) {
            total += node_text_len(&child);
        }
    }
    total
}

fn find_offset_in_tree(
    current: &Node,
    target_container: &Node,
    target_offset: u32,
    total: &mut usize,
) -> bool {
    if current.is_same_node(Some(target_container)) {
        if current.node_type() == Node::TEXT_NODE {
            *total += target_offset as usize;
            return true;
        }
        let children = current.child_nodes();
        let upto = target_offset.min(children.length());
        for idx in 0..upto {
            if let Some(child) = children.item(idx) {
                *total += node_text_len(&child);
            }
        }
        return true;
    }

    if current.node_type() == Node::TEXT_NODE {
        *total += current.node_value().unwrap_or_default().len();
        return false;
    }

    let children = current.child_nodes();
    for idx in 0..children.length() {
        if let Some(child) = children.item(idx) {
            if find_offset_in_tree(&child, target_container, target_offset, total) {
                return true;
            }
        }
    }
    false
}

fn compute_dom_offset(root: &Node, container: &Node, offset: u32) -> Option<usize> {
    let mut total = 0usize;
    if find_offset_in_tree(root, container, offset, &mut total) {
        Some(total)
    } else {
        None
    }
}

fn find_text_position(nodes: &[Node], target: usize) -> Option<(Node, u32)> {
    let mut consumed = 0usize;
    for node in nodes {
        let text = node.node_value().unwrap_or_default();
        let len = text.len();
        // Use right-biased boundary mapping so exact boundaries prefer the next node.
        // This avoids caret anchoring to hidden marker nodes at span edges.
        if target < consumed + len {
            return Some((node.clone(), (target - consumed) as u32));
        }
        consumed += len;
    }
    nodes.last().map(|node| {
        (
            node.clone(),
            node.node_value().unwrap_or_default().len() as u32,
        )
    })
}

fn get_selection_byte_offsets(root: &HtmlElement) -> Option<Selection> {
    let win = leptos::web_sys::window()?;
    let selection = win.get_selection().ok().flatten()?;
    if selection.range_count() == 0 {
        return None;
    }
    let range = selection.get_range_at(0).ok()?;
    let root_node: Node = root.clone().unchecked_into();

    let start_container = range.start_container().ok()?;
    let end_container = range.end_container().ok()?;
    if !root_node.contains(Some(&start_container)) || !root_node.contains(Some(&end_container)) {
        return None;
    }

    let start = compute_dom_offset(&root_node, &start_container, range.start_offset().ok()?)?;
    let end = compute_dom_offset(&root_node, &end_container, range.end_offset().ok()?)?;

    Some(Selection::new(start, end))
}

fn set_selection_byte_offsets(root: &HtmlElement, selection: Selection) {
    let Some(win) = leptos::web_sys::window() else {
        return;
    };
    let Some(document) = win.document() else {
        return;
    };
    let Some(dom_selection) = win.get_selection().ok().flatten() else {
        return;
    };

    let root_node: Node = root.clone().unchecked_into();
    let root_text_len = root.text_content().unwrap_or_default().len();
    let selection = selection.clamp(root_text_len);

    let mut text_nodes = Vec::new();
    collect_text_nodes(&root_node, &mut text_nodes);
    if text_nodes.is_empty() {
        return;
    }

    let Some((start_node, start_offset)) = find_text_position(&text_nodes, selection.start) else {
        return;
    };
    let Some((end_node, end_offset)) = find_text_position(&text_nodes, selection.end) else {
        return;
    };

    let Ok(range) = document.create_range() else {
        return;
    };
    if range.set_start(&start_node, start_offset).is_err() {
        return;
    }
    if range.set_end(&end_node, end_offset).is_err() {
        return;
    }
    let _ = dom_selection.remove_all_ranges();
    let _ = dom_selection.add_range(&range);
}

fn highlight_inline(
    text: &str,
    caret: Option<usize>,
    image_ctx: Option<&ImageRenderContext>,
) -> String {
    static RE_EMBED: OnceLock<Regex> = OnceLock::new();
    static RE_WIKI: OnceLock<Regex> = OnceLock::new();
    static RE_MD_LINK: OnceLock<Regex> = OnceLock::new();
    static RE_MD_IMAGE: OnceLock<Regex> = OnceLock::new();
    static RE_CODE: OnceLock<Regex> = OnceLock::new();
    static RE_INLINE_MATH: OnceLock<Regex> = OnceLock::new();
    static RE_FOOTNOTE_REF: OnceLock<Regex> = OnceLock::new();
    static RE_INLINE_FOOTNOTE: OnceLock<Regex> = OnceLock::new();
    static RE_BLOCK_ID: OnceLock<Regex> = OnceLock::new();
    static RE_TAG: OnceLock<Regex> = OnceLock::new();

    let re_embed = RE_EMBED.get_or_init(|| Regex::new(r"!\[\[([^\]\n]+)\]\]").unwrap());
    let re_wiki = RE_WIKI.get_or_init(|| Regex::new(r"\[\[([^\]]+)\]\]").unwrap());
    let re_md_link = RE_MD_LINK.get_or_init(|| Regex::new(r"\[([^\]\n]+)\]\(([^)\n]+)\)").unwrap());
    let re_md_image =
        RE_MD_IMAGE.get_or_init(|| Regex::new(r"!\[([^\]\n]*)\]\(([^)\n]+)\)").unwrap());
    let re_code = RE_CODE.get_or_init(|| Regex::new(r"`([^`\n]+)`").unwrap());
    let re_inline_math = RE_INLINE_MATH.get_or_init(|| Regex::new(r"\$([^$\n]+)\$").unwrap());
    let re_footnote_ref = RE_FOOTNOTE_REF.get_or_init(|| Regex::new(r"\[\^[^\]\n]+\]").unwrap());
    let re_inline_footnote =
        RE_INLINE_FOOTNOTE.get_or_init(|| Regex::new(r"\^\[[^\]\n]+\]").unwrap());
    let re_block_id =
        RE_BLOCK_ID.get_or_init(|| Regex::new(r"\^[A-Za-z0-9][A-Za-z0-9-]*").unwrap());
    let re_tag = RE_TAG.get_or_init(|| Regex::new(r"#[A-Za-z][A-Za-z0-9_/-]*").unwrap());

    let mut matches: Vec<InlineMatch> = Vec::new();

    // Inline code has the highest precedence.
    for cap in re_code.captures_iter(text) {
        let m = cap.get(0).unwrap();
        let inner = cap.get(1).unwrap();
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: inner.start(),
                inner_end: inner.end(),
                open_len: 1,
                close_len: 1,
                class: "hl-code",
                hide_tokens: true,
                preview_html: None,
            },
        );
    }

    // Obsidian comments: %% comment %% (including unmatched opener while typing).
    for m in collect_delimited_matches(text, "%%", "hl-comment", false) {
        push_non_overlapping(&mut matches, m);
    }

    for cap in re_embed.captures_iter(text) {
        let m = cap.get(0).unwrap();
        let inner = cap.get(1).unwrap();
        let target = strip_wiki_target(inner.as_str());
        let preview_html = resolve_image_preview_html(image_ctx, &target, None);
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: inner.start(),
                inner_end: inner.end(),
                open_len: 3,
                close_len: 2,
                class: "hl-embed",
                hide_tokens: true,
                preview_html,
            },
        );
    }

    for cap in re_wiki.captures_iter(text) {
        let m = cap.get(0).unwrap();
        let inner = cap.get(1).unwrap();
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: inner.start(),
                inner_end: inner.end(),
                open_len: 2,
                close_len: 2,
                class: "hl-link",
                hide_tokens: true,
                preview_html: None,
            },
        );
    }

    for cap in re_md_image.captures_iter(text) {
        let m = cap.get(0).unwrap();
        let alt = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let raw_target = cap.get(2).map(|m| m.as_str()).unwrap_or_default();
        let target = strip_markdown_image_target(raw_target);
        let preview_html = resolve_image_preview_html(image_ctx, &target, Some(alt));
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-embed",
                hide_tokens: false,
                preview_html,
            },
        );
    }

    for cap in re_md_link.captures_iter(text) {
        let m = cap.get(0).unwrap();
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-link",
                hide_tokens: false,
                preview_html: None,
            },
        );
    }

    for m in collect_delimited_matches(text, "***", "hl-bold hl-italic", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "___", "hl-bold hl-italic", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "**", "hl-bold", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "__", "hl-bold", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "~~", "hl-strike", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "==", "hl-mark", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "*", "hl-italic", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "_", "hl-italic", true) {
        push_non_overlapping(&mut matches, m);
    }

    for cap in re_inline_math.captures_iter(text) {
        let m = cap.get(0).unwrap();
        let inner = cap.get(1).unwrap();
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: inner.start(),
                inner_end: inner.end(),
                open_len: 1,
                close_len: 1,
                class: "hl-math-inline",
                hide_tokens: true,
                preview_html: None,
            },
        );
    }

    for m in re_footnote_ref.find_iter(text) {
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-footnote",
                hide_tokens: false,
                preview_html: None,
            },
        );
    }

    for m in re_inline_footnote.find_iter(text) {
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-footnote",
                hide_tokens: false,
                preview_html: None,
            },
        );
    }

    for m in re_tag.find_iter(text) {
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-tag",
                hide_tokens: false,
                preview_html: None,
            },
        );
    }

    for m in re_block_id.find_iter(text) {
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-block-id",
                hide_tokens: false,
                preview_html: None,
            },
        );
    }

    matches.sort_by_key(|m| m.start);
    let mut disjoint: Vec<&InlineMatch> = Vec::new();
    let mut last_end = 0usize;
    for m in &matches {
        if m.start >= last_end {
            disjoint.push(m);
            last_end = m.end;
        }
    }

    let mut out = String::new();
    let mut pos = 0usize;
    for m in disjoint {
        out.push_str(&escape_html(&text[pos..m.start]));
        let caret_inside = caret.map(|c| c >= m.start && c <= m.end).unwrap_or(false);

        if caret_inside && m.hide_tokens {
            // Keep live formatting active while caret is inside the markdown span,
            // but reveal the wrapper tokens for accurate editing context.
            out.push_str("<span class=\"md-token md-token-visible\">");
            out.push_str(&escape_html(&text[m.start..m.start + m.open_len]));
            out.push_str("</span><span class=\"");
            out.push_str(m.class);
            out.push_str("\">");
            out.push_str(&escape_html(&text[m.inner_start..m.inner_end]));
            out.push_str("</span><span class=\"md-token md-token-visible\">");
            out.push_str(&escape_html(&text[m.end - m.close_len..m.end]));
            out.push_str("</span>");
        } else if caret_inside {
            out.push_str(&escape_html(&text[m.start..m.end]));
        } else if m.hide_tokens {
            out.push_str("<span class=\"md-token md-token-hidden\">");
            out.push_str(&escape_html(&text[m.start..m.start + m.open_len]));
            out.push_str("</span><span class=\"");
            out.push_str(m.class);
            out.push_str("\">");
            out.push_str(&escape_html(&text[m.inner_start..m.inner_end]));
            out.push_str("</span><span class=\"md-token md-token-hidden\">");
            out.push_str(&escape_html(&text[m.end - m.close_len..m.end]));
            out.push_str("</span>");
        } else {
            out.push_str("<span class=\"");
            out.push_str(m.class);
            out.push_str("\">");
            out.push_str(&escape_html(&text[m.start..m.end]));
            out.push_str("</span>");
        }
        if let Some(preview) = &m.preview_html {
            out.push_str(preview);
        }
        pos = m.end;
    }
    out.push_str(&escape_html(&text[pos..]));
    out
}

fn highlight_markdown(
    text: &str,
    caret: Option<usize>,
    image_ctx: Option<&ImageRenderContext>,
) -> String {
    static RE_HEADING: OnceLock<Regex> = OnceLock::new();
    static RE_CALLOUT: OnceLock<Regex> = OnceLock::new();
    static RE_QUOTE: OnceLock<Regex> = OnceLock::new();
    static RE_TASK: OnceLock<Regex> = OnceLock::new();
    static RE_LIST: OnceLock<Regex> = OnceLock::new();
    static RE_ORDERED: OnceLock<Regex> = OnceLock::new();
    static RE_HR: OnceLock<Regex> = OnceLock::new();
    static RE_TABLE_ROW: OnceLock<Regex> = OnceLock::new();
    static RE_TABLE_SEPARATOR: OnceLock<Regex> = OnceLock::new();
    static RE_FOOTNOTE_DEF: OnceLock<Regex> = OnceLock::new();

    let re_heading = RE_HEADING.get_or_init(|| Regex::new(r"^(#{1,6})[^\S\n]+.*$").unwrap());
    let re_callout =
        RE_CALLOUT.get_or_init(|| Regex::new(r"^\s*>\s*\[![A-Za-z0-9-]+\][+-]?\s*.*$").unwrap());
    let re_quote = RE_QUOTE.get_or_init(|| Regex::new(r"^\s*>\s+.*$").unwrap());
    let re_task = RE_TASK.get_or_init(|| Regex::new(r"^\s*[-*+]\s+\[(?: |x|X)\]\s+.*$").unwrap());
    let re_list = RE_LIST.get_or_init(|| Regex::new(r"^\s*[-*+]\s+.*$").unwrap());
    let re_ordered = RE_ORDERED.get_or_init(|| Regex::new(r"^\s*\d+[.)]\s+.*$").unwrap());
    let re_hr = RE_HR.get_or_init(|| {
        Regex::new(r"^\s{0,3}(?:(?:\*[\t ]*){3,}|(?:-[\t ]*){3,}|(?:_[\t ]*){3,})\s*$").unwrap()
    });
    let re_table_row = RE_TABLE_ROW.get_or_init(|| Regex::new(r"^\s*\|.*\|\s*$").unwrap());
    let re_table_separator = RE_TABLE_SEPARATOR.get_or_init(|| {
        Regex::new(r"^\s*\|?(?:\s*:?-{3,}:?\s*\|)+\s*:?-{3,}:?\s*\|?\s*$").unwrap()
    });
    let re_footnote_def =
        RE_FOOTNOTE_DEF.get_or_init(|| Regex::new(r"^\s*\[\^[^\]]+\]:\s+.*$").unwrap());

    let mut out = String::new();
    let mut offset = 0usize;
    let mut in_frontmatter = false;
    let mut frontmatter_possible = true;
    let mut in_math_block = false;
    let mut in_comment_block = false;
    let mut code_fence: Option<(u8, usize)> = None;

    for line in text.split_inclusive('\n') {
        let line_len = line.len();
        let line_without_nl = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = line_without_nl.trim();

        if let Some((marker, min_len)) = code_fence {
            out.push_str(&wrap_line("hl-codeblock", escape_html(line)));
            if code_fence_close(line_without_nl, marker, min_len) {
                code_fence = None;
            }
            offset += line_len;
            continue;
        }

        if in_math_block {
            out.push_str(&wrap_line("hl-math-block", escape_html(line)));
            if trimmed == "$$" {
                in_math_block = false;
            }
            offset += line_len;
            continue;
        }

        if in_frontmatter {
            out.push_str(&wrap_line("hl-frontmatter", escape_html(line)));
            if trimmed == "---" || trimmed == "..." {
                in_frontmatter = false;
                frontmatter_possible = false;
            }
            offset += line_len;
            continue;
        }

        if in_comment_block {
            out.push_str(&wrap_line("hl-comment", escape_html(line)));
            if line_without_nl.matches("%%").count() % 2 == 1 {
                in_comment_block = false;
            }
            offset += line_len;
            continue;
        }

        if frontmatter_possible {
            if trimmed == "---" {
                out.push_str(&wrap_line("hl-frontmatter", escape_html(line)));
                in_frontmatter = true;
                offset += line_len;
                continue;
            }
            if !trimmed.is_empty() {
                frontmatter_possible = false;
            }
        }

        if let Some((marker, len)) = code_fence_open(line_without_nl) {
            out.push_str(&wrap_line("hl-codeblock hl-code-fence", escape_html(line)));
            code_fence = Some((marker, len));
            offset += line_len;
            continue;
        }

        if trimmed == "$$" {
            out.push_str(&wrap_line("hl-math-block", escape_html(line)));
            in_math_block = true;
            offset += line_len;
            continue;
        }

        let caret_rel = caret
            .filter(|c| *c >= offset && *c < offset + line_len)
            .map(|c| c - offset);
        let line_html = highlight_inline(line, caret_rel, image_ctx);

        let wrapped = if re_callout.is_match(line_without_nl) {
            wrap_line("hl-callout", line_html)
        } else if let Some(cap) = re_heading.captures(line_without_nl) {
            let level = cap.get(1).map(|m| m.as_str().len()).unwrap_or(1);
            match level {
                1 => wrap_line("hl-h1", line_html),
                2 => wrap_line("hl-h2", line_html),
                3 => wrap_line("hl-h3", line_html),
                4 => wrap_line("hl-h4", line_html),
                5 => wrap_line("hl-h5", line_html),
                _ => wrap_line("hl-h6", line_html),
            }
        } else if re_hr.is_match(line_without_nl) {
            wrap_line("hl-hr", line_html)
        } else if re_footnote_def.is_match(line_without_nl) {
            wrap_line("hl-footnote-def", line_html)
        } else if re_quote.is_match(line_without_nl) {
            wrap_line("hl-quote", line_html)
        } else if re_task.is_match(line_without_nl) {
            wrap_line("hl-task", line_html)
        } else if re_ordered.is_match(line_without_nl) || re_list.is_match(line_without_nl) {
            wrap_line("hl-list", line_html)
        } else if re_table_separator.is_match(line_without_nl)
            || re_table_row.is_match(line_without_nl)
        {
            wrap_line("hl-table", line_html)
        } else {
            line_html
        };
        out.push_str(&wrapped);

        if line_without_nl.matches("%%").count() % 2 == 1 {
            in_comment_block = true;
        }

        offset += line_len;
    }

    out
}

fn highlight_markdown_for_editor(
    text: &str,
    caret: Option<usize>,
    vault_path: &str,
    current_file: &str,
    image_cache: &HashMap<String, String>,
) -> String {
    if vault_path.is_empty() || current_file.is_empty() {
        return highlight_markdown(text, caret, None);
    }
    let ctx = ImageRenderContext {
        vault_path,
        current_file,
        cache: image_cache,
    };
    highlight_markdown(text, caret, Some(&ctx))
}

fn extract_file_cache(text: &str) -> FileCache {
    static RE_HEADING: OnceLock<Regex> = OnceLock::new();
    static RE_WIKI: OnceLock<Regex> = OnceLock::new();
    static RE_MD_LINK: OnceLock<Regex> = OnceLock::new();
    static RE_TAG: OnceLock<Regex> = OnceLock::new();

    let re_heading = RE_HEADING.get_or_init(|| Regex::new(r"^(#{1,6})[ \t]+(.+?)\s*$").unwrap());
    let re_wiki = RE_WIKI.get_or_init(|| Regex::new(r"\[\[([^\]]+)\]\]").unwrap());
    let re_md_link = RE_MD_LINK.get_or_init(|| Regex::new(r"!?\[[^\]\n]*\]\(([^)\n]+)\)").unwrap());
    let re_tag = RE_TAG.get_or_init(|| Regex::new(r"#[A-Za-z][A-Za-z0-9_/-]*").unwrap());

    let mut headings = Vec::new();
    let mut tags = Vec::new();
    let mut links = Vec::new();

    for (idx, line) in text.lines().enumerate() {
        if let Some(cap) = re_heading.captures(line) {
            let level = cap.get(1).map(|m| m.as_str().len()).unwrap_or(1) as u8;
            let text = cap
                .get(2)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default();
            headings.push(HeadingCache {
                level,
                text,
                line: idx + 1,
            });
        }
        for tag in re_tag.find_iter(line) {
            tags.push(tag.as_str().trim_start_matches('#').to_ascii_lowercase());
        }
    }

    for cap in re_wiki.captures_iter(text) {
        let raw_inner = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let left = raw_inner.split('|').next().unwrap_or_default();
        let link = left.split('#').next().unwrap_or_default().trim();
        if !link.is_empty() {
            links.push(normalize_rel_path(link));
        }
    }

    for cap in re_md_link.captures_iter(text) {
        let raw_target = cap.get(1).map(|m| m.as_str()).unwrap_or_default().trim();
        let target = raw_target.trim_matches('<').trim_matches('>');
        if target.is_empty() || target.starts_with('#') {
            continue;
        }
        let lowered = target.to_ascii_lowercase();
        if lowered.contains("://") || lowered.starts_with("mailto:") {
            continue;
        }
        let cleaned = target
            .split('#')
            .next()
            .unwrap_or_default()
            .split('?')
            .next()
            .unwrap_or_default()
            .trim();
        if !cleaned.is_empty() {
            links.push(normalize_rel_path(cleaned));
        }
    }

    tags.sort();
    tags.dedup();
    links.sort();
    links.dedup();

    FileCache {
        headings,
        tags,
        links,
    }
}

fn resolve_linkpath(
    linkpath: &str,
    source_path: &str,
    file_lookup: &HashMap<String, String>,
    stem_lookup: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let raw = normalize_rel_path(linkpath);
    if raw.is_empty() {
        return None;
    }

    let raw_has_ext = raw.to_ascii_lowercase().ends_with(".md");
    let mut candidates = Vec::new();
    candidates.push(if raw_has_ext {
        raw.clone()
    } else {
        format!("{raw}.md")
    });

    if raw.contains('/') {
        let source_dir = source_path
            .rsplit_once('/')
            .map(|(dir, _)| dir)
            .unwrap_or("");
        if !source_dir.is_empty() {
            let joined = normalize_rel_path(&format!("{source_dir}/{raw}"));
            candidates.push(if raw_has_ext {
                joined.clone()
            } else {
                format!("{joined}.md")
            });
        }
    }

    for candidate in candidates {
        let key = candidate.to_ascii_lowercase();
        if let Some(found) = file_lookup.get(&key) {
            return Some(found.clone());
        }
    }

    let stem = Path::new(&raw)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&raw)
        .to_ascii_lowercase();

    if let Some(candidates) = stem_lookup.get(&stem) {
        if candidates.len() == 1 {
            return candidates.first().cloned();
        }
    }

    None
}

fn build_metadata_cache(notes: &HashMap<String, String>, files: &[String]) -> MetadataCacheState {
    let mut state = MetadataCacheState::default();
    let mut file_lookup = HashMap::new();
    let mut stem_lookup: HashMap<String, Vec<String>> = HashMap::new();

    for path in files {
        file_lookup.insert(path.to_ascii_lowercase(), path.clone());
        let stem = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
            .to_ascii_lowercase();
        stem_lookup.entry(stem).or_default().push(path.clone());

        let text = notes.get(path).cloned().unwrap_or_default();
        let cache = extract_file_cache(&text);
        for tag in &cache.tags {
            state
                .tags_index
                .entry(tag.clone())
                .or_default()
                .push(path.clone());
        }
        state.file_cache.insert(path.clone(), cache);
    }

    for path in files {
        let cache = state.file_cache.get(path).cloned().unwrap_or_default();
        for link in cache.links {
            if let Some(target) = resolve_linkpath(&link, path, &file_lookup, &stem_lookup) {
                let by_source = state.resolved_links.entry(path.clone()).or_default();
                *by_source.entry(target.clone()).or_insert(0) += 1;
                state
                    .backlinks
                    .entry(target)
                    .or_default()
                    .push(path.clone());
            } else {
                let by_source = state.unresolved_links.entry(path.clone()).or_default();
                *by_source.entry(link).or_insert(0) += 1;
            }
        }
    }

    for files_for_tag in state.tags_index.values_mut() {
        files_for_tag.sort();
        files_for_tag.dedup();
    }
    for backlink_sources in state.backlinks.values_mut() {
        backlink_sources.sort();
        backlink_sources.dedup();
    }

    state
}

#[cfg(test)]
mod markdown_syntax_tests {
    use super::*;

    fn assert_has(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected substring `{needle}` in:\n{haystack}"
        );
    }

    #[test]
    fn highlights_obsidian_inline_emphasis_variants() {
        let html = highlight_inline(
            "**bold** __bold2__ *it* _it2_ ~~gone~~ ==mark== ***both*** ___both2___",
            None,
            None,
        );
        assert_has(&html, "hl-bold");
        assert_has(&html, "hl-italic");
        assert_has(&html, "hl-strike");
        assert_has(&html, "hl-mark");
        assert_has(&html, "hl-bold hl-italic");
    }

    #[test]
    fn highlights_obsidian_links_embeds_tags_and_blocks() {
        let html = highlight_inline(
            "[[Note]] ![[Asset.png]] [Label](Note.md) ![img](Img.png) #tag ^block-id",
            None,
            None,
        );
        assert_has(&html, "hl-link");
        assert_has(&html, "hl-embed");
        assert_has(&html, "hl-tag");
        assert_has(&html, "hl-block-id");
    }

    #[test]
    fn highlights_obsidian_comments_footnotes_and_math() {
        let html = highlight_inline(
            "%%comment%% Ref[^1] inline ^[note] and $e^{i\\pi}+1=0$",
            None,
            None,
        );
        assert_has(&html, "hl-comment");
        assert_has(&html, "hl-footnote");
        assert_has(&html, "hl-math-inline");
    }

    #[test]
    fn highlights_headings_h1_to_h6() {
        let html = highlight_markdown(
            "# h1\n## h2\n### h3\n#### h4\n##### h5\n###### h6\n".trim_end_matches('\n'),
            None,
            None,
        );
        assert_has(&html, "hl-h1");
        assert_has(&html, "hl-h2");
        assert_has(&html, "hl-h3");
        assert_has(&html, "hl-h4");
        assert_has(&html, "hl-h5");
        assert_has(&html, "hl-h6");
    }

    #[test]
    fn highlights_callouts_lists_quotes_and_hr() {
        let html = highlight_markdown(
            "> [!note] Title\n> quote\n- [ ] task\n1) one\n- item\n---\n",
            None,
            None,
        );
        assert_has(&html, "hl-callout");
        assert_has(&html, "hl-quote");
        assert_has(&html, "hl-task");
        assert_has(&html, "hl-list");
        assert_has(&html, "hl-hr");
    }

    #[test]
    fn highlights_tables_code_fences_math_blocks_frontmatter_and_comments() {
        let html = highlight_markdown(
            "---\ntitle: Bedrock\n---\n| a | b |\n| --- | --- |\n```rust\nlet x = 1;\n```\n$$\na+b\n$$\n%%\ncomment block\n%%\n",
            None,
            None,
        );
        assert_has(&html, "hl-frontmatter");
        assert_has(&html, "hl-table");
        assert_has(&html, "hl-codeblock");
        assert_has(&html, "hl-code-fence");
        assert_has(&html, "hl-math-block");
        assert_has(&html, "hl-comment");
    }

    #[test]
    fn highlights_footnote_definitions() {
        let html = highlight_markdown("[^note]: footnote text\n", None, None);
        assert_has(&html, "hl-footnote-def");
    }

    #[test]
    fn metadata_extracts_wikilinks_and_markdown_links() {
        let cache = extract_file_cache(
            "[[Wiki Note]]\n[md](Folder/Note.md)\n![img](Image.png)\n[ext](https://example.com)\n",
        );
        assert!(cache.links.iter().any(|link| link == "Wiki Note"));
        assert!(cache.links.iter().any(|link| link == "Folder/Note.md"));
        assert!(cache.links.iter().any(|link| link == "Image.png"));
        assert!(!cache.links.iter().any(|link| link.contains("https://")));
    }

    #[test]
    fn inline_delimiters_respect_escaping() {
        let html = highlight_inline(r"\*\*literal\*\* and **bold**", None, None);
        assert_has(&html, "hl-bold");
        assert_has(&html, r"\*\*literal\*\*");
    }
}

#[component]
pub fn App() -> impl IntoView {
    let (vault_path, set_vault_path) = signal(String::new());
    let (open_vaults, set_open_vaults) = signal(Vec::<String>::new());
    let (files, set_files) = signal(Vec::<String>::new());
    let (empty_dirs, set_empty_dirs) = signal(Vec::<String>::new());
    let (note_texts, set_note_texts) = signal(HashMap::<String, String>::new());
    let (metadata_cache, set_metadata_cache) = signal(MetadataCacheState::default());

    let (current_file, set_current_file) = signal(String::new());
    let (content, set_content) = signal(String::new());
    let (editor_snapshot, set_editor_snapshot) = signal(EditorSnapshot::new(String::new()));
    let (parsed_html, set_parsed_html) = signal(String::new());
    let (_caret_pos, set_caret_pos) = signal(Option::<usize>::None);
    let editor_ref = NodeRef::<html::Div>::new();
    let (is_composing, set_is_composing) = signal(false);
    let (composition_dirty, set_composition_dirty) = signal(false);
    let (image_preview_cache, set_image_preview_cache) = signal(HashMap::<String, String>::new());
    let (image_preview_loading, set_image_preview_loading) = signal(HashSet::<String>::new());
    let (image_preview_failed, set_image_preview_failed) = signal(HashSet::<String>::new());

    let (plugin_css, set_plugin_css) = signal(String::new());
    let (settings, set_settings) = signal(AppSettings::default());

    let (save_timeout_id, set_save_timeout_id) = signal(Option::<i32>::None);
    let (save_status, set_save_status) = signal("Saved".to_string());
    let (show_markdown_syntax, set_show_markdown_syntax) = signal(false);
    let (expanded_folders, set_expanded_folders) = signal(HashSet::<String>::new());
    let (sidebar_context_menu, set_sidebar_context_menu) = signal(Option::<SidebarContextMenu>::None);
    let (selection_restore_ticket, set_selection_restore_ticket) = signal(0u64);
    let (selection_sync_ticket, set_selection_sync_ticket) = signal(0u64);

    let closure = Closure::<dyn FnMut(leptos::web_sys::CustomEvent)>::new(
        move |e: leptos::web_sys::CustomEvent| {
            if let Some(detail) = e.detail().as_string() {
                if let Ok(s) = serde_json::from_str::<AppSettings>(&detail) {
                    set_settings.set(s);
                }
            }
        },
    );
    let _ = window()
        .add_event_listener_with_callback("bedrock-settings", closure.as_ref().unchecked_ref());
    closure.forget();

    let is_settings_window = window()
        .location()
        .search()
        .unwrap_or_default()
        .contains("settings=true");

    let clear_active_vault_state = move || {
        set_files.set(Vec::new());
        set_empty_dirs.set(Vec::new());
        set_note_texts.set(HashMap::new());
        set_metadata_cache.set(MetadataCacheState::default());
        set_current_file.set(String::new());
        set_content.set(String::new());
        set_parsed_html.set(String::new());
        set_caret_pos.set(None);
        set_editor_snapshot.set(EditorSnapshot::new(String::new()));
        set_image_preview_cache.set(HashMap::new());
        set_image_preview_loading.set(HashSet::new());
        set_image_preview_failed.set(HashSet::new());
        set_expanded_folders.set(HashSet::new());
        set_plugin_css.set(String::new());
    };

    let refresh_vault_snapshot = move |path: String, preferred_file: Option<String>| {
        spawn_local(async move {
            set_image_preview_cache.set(HashMap::new());
            set_image_preview_loading.set(HashSet::new());
            set_image_preview_failed.set(HashSet::new());

            let dir_args = serde_wasm_bindgen::to_value(&ReadDirArgs { path: &path }).unwrap();
            let dir_val = invoke("read_dir", dir_args).await;
            let dir_result =
                serde_wasm_bindgen::from_value::<ReadDirResult>(dir_val).unwrap_or_else(|_| ReadDirResult { notes: Vec::new(), empty_dirs: Vec::new() });
            set_files.set(dir_result.notes.clone());
            set_empty_dirs.set(dir_result.empty_dirs.clone());

            let vault_args =
                serde_wasm_bindgen::to_value(&VaultPathArgs { vault_path: &path }).unwrap();
            let notes_val = invoke("read_vault_notes", vault_args).await;
            let notes_list =
                serde_wasm_bindgen::from_value::<Vec<VaultNote>>(notes_val).unwrap_or_default();

            let mut note_map = HashMap::new();
            for note in notes_list {
                note_map.insert(note.path, note.content);
            }

            set_note_texts.set(note_map.clone());
            let dir_list = &dir_result.notes;
            set_metadata_cache.set(build_metadata_cache(&note_map, dir_list));

            let next_file = preferred_file
                .filter(|f| dir_list.contains(f))
                .or_else(|| {
                    let active = current_file.get_untracked();
                    if !active.is_empty() && dir_list.contains(&active) {
                        Some(active)
                    } else {
                        None
                    }
                })
                .or_else(|| dir_list.first().cloned());

            if let Some(selected_file) = next_file {
                let text = note_map.get(&selected_file).cloned().unwrap_or_default();
                set_current_file.set(selected_file.clone());
                set_expanded_folders.update(|expanded| {
                    expand_parent_folders(expanded, &selected_file);
                });
                set_content.set(text.clone());
                set_parsed_html.set(highlight_markdown_for_editor(
                    &text,
                    None,
                    &path,
                    &selected_file,
                    &image_preview_cache.get_untracked(),
                ));
                set_caret_pos.set(None);
                set_editor_snapshot.set(EditorSnapshot::new(text));
            } else {
                set_current_file.set(String::new());
                set_content.set(String::new());
                set_parsed_html.set(String::new());
                set_caret_pos.set(None);
                set_editor_snapshot.set(EditorSnapshot::new(String::new()));
            }
        });
    };

    let load_vault_visual_state = move |path: String| {
        if path.is_empty() {
            set_plugin_css.set(String::new());
            return;
        }
        spawn_local(async move {
            let vault_args =
                serde_wasm_bindgen::to_value(&VaultPathArgs { vault_path: &path }).unwrap();

            let css_val = invoke("load_plugins_css", vault_args.clone()).await;
            if let Some(css_str) = css_val.as_string() {
                set_plugin_css.set(css_str);
            } else {
                set_plugin_css.set(String::new());
            }

            let s_val = invoke("load_settings", vault_args).await;
            if let Some(s_str) = s_val.as_string() {
                if let Ok(s) = serde_json::from_str::<AppSettings>(&s_str) {
                    set_settings.set(s);
                }
            }
        });
    };

    let persist_vault_session = move |open_list: Vec<String>, active: Option<String>| {
        let set_open = set_open_vaults;
        let set_active = set_vault_path;
        spawn_local(async move {
            let payload = Object::new();
            let open_value = serde_wasm_bindgen::to_value(&open_list).unwrap_or(JsValue::NULL);
            let active_value = active
                .as_ref()
                .map(|value| JsValue::from_str(value))
                .unwrap_or(JsValue::NULL);

            let _ = Reflect::set(&payload, &JsValue::from_str("open_vaults"), &open_value);
            let _ = Reflect::set(&payload, &JsValue::from_str("openVaults"), &open_value);
            let _ = Reflect::set(&payload, &JsValue::from_str("active_vault"), &active_value);
            let _ = Reflect::set(&payload, &JsValue::from_str("activeVault"), &active_value);

            let result = invoke("save_vault_session", payload.into()).await;
            if let Ok(normalized) = serde_wasm_bindgen::from_value::<VaultSessionState>(result) {
                set_open.set(normalized.open_vaults.clone());
                if let Some(active_path) = normalized.active_vault {
                    set_active.set(active_path);
                }
            }
        });
    };

    let activate_vault = move |path: String, preferred_file: Option<String>| {
        if path.is_empty() {
            return;
        }
        let normalized = collapse_path(&path);
        let mut open_now = open_vaults.get_untracked();
        if !open_now.iter().any(|p| collapse_path(p) == normalized) {
            open_now.push(path.clone());
        }
        set_open_vaults.set(open_now.clone());
        set_vault_path.set(path.clone());
        persist_vault_session(open_now, Some(path.clone()));
        refresh_vault_snapshot(path.clone(), preferred_file);
        load_vault_visual_state(path);
    };

    Effect::new(move |_| {
        spawn_local(async move {
            let default_path = invoke("init_vault", JsValue::NULL).await.as_string();

            let session_val = invoke("load_vault_session", JsValue::NULL).await;
            let mut session = serde_wasm_bindgen::from_value::<VaultSessionState>(session_val)
                .unwrap_or_default();

            let fallback = default_path.unwrap_or_default();
            if session.open_vaults.is_empty() && !fallback.is_empty() {
                session.open_vaults.push(fallback.clone());
            }

            let mut active = session
                .active_vault
                .clone()
                .filter(|p| !p.trim().is_empty())
                .or_else(|| session.open_vaults.first().cloned());

            if active.is_none() && !fallback.is_empty() {
                active = Some(fallback.clone());
            }

            if let Some(active_path) = active.clone() {
                if let Some(existing) = session
                    .open_vaults
                    .iter()
                    .find(|p| collapse_path(p) == collapse_path(&active_path))
                    .cloned()
                {
                    active = Some(existing);
                } else if let Some(first) = session.open_vaults.first().cloned() {
                    active = Some(first);
                } else {
                    session.open_vaults.push(active_path.clone());
                    active = Some(active_path);
                }
            }

            set_open_vaults.set(session.open_vaults.clone());

            if let Some(path) = active.clone() {
                activate_vault(path, None);
            } else {
                set_vault_path.set(String::new());
                clear_active_vault_state();
                set_plugin_css.set(String::new());
            }
            persist_vault_session(session.open_vaults.clone(), active.clone());
        });
    });

    Effect::new(move |_| {
        let v_path = vault_path.get();
        let file = current_file.get();
        let text = content.get();
        if v_path.is_empty() || file.is_empty() || text.is_empty() {
            return;
        }

        let mut pending_paths = Vec::new();
        let cache = image_preview_cache.get_untracked();
        let loading = image_preview_loading.get_untracked();
        let failed = image_preview_failed.get_untracked();

        for (target, _is_wiki_embed) in collect_image_targets_for_note(&text) {
            if target.is_empty()
                || looks_like_external_url(&target)
                || !is_supported_inline_image_path(&target)
            {
                continue;
            }
            let candidates = image_local_candidates(&v_path, &file, &target);
            if candidates
                .iter()
                .any(|candidate| cache.contains_key(candidate))
            {
                continue;
            }
            for candidate in candidates {
                if !loading.contains(&candidate) && !failed.contains(&candidate) {
                    pending_paths.push(candidate);
                }
            }
        }

        pending_paths.sort();
        pending_paths.dedup();
        if pending_paths.is_empty() {
            return;
        }

        set_image_preview_loading.update(|loading_set| {
            for path in &pending_paths {
                loading_set.insert(path.clone());
            }
        });

        for image_path in pending_paths {
            let set_cache = set_image_preview_cache;
            let set_loading = set_image_preview_loading;
            let set_failed = set_image_preview_failed;
            spawn_local(async move {
                let args =
                    serde_wasm_bindgen::to_value(&ReadFileArgs { path: &image_path }).unwrap();
                let value = invoke("read_file_base64", args).await;
                if let Some(encoded) = value.as_string() {
                    let src = format!(
                        "data:{};base64,{}",
                        image_mime_for_path(&image_path),
                        encoded
                    );
                    set_cache.update(|cache| {
                        cache.insert(image_path.clone(), src);
                    });
                    set_failed.update(|failed| {
                        failed.remove(&image_path);
                    });
                } else {
                    set_failed.update(|failed| {
                        failed.insert(image_path.clone());
                    });
                }
                set_loading.update(|loading| {
                    loading.remove(&image_path);
                });
            });
        }
    });

    let schedule_disk_write = move |filename: String, new_text: String| {
        if filename.is_empty() {
            return;
        }
        let v_path = vault_path.get_untracked();
        if v_path.is_empty() {
            return;
        }

        if let Some(win) = leptos::web_sys::window() {
            if let Some(timeout) = save_timeout_id.get_untracked() {
                win.clear_timeout_with_handle(timeout);
            }

            set_save_status.set("Saving...".to_string());
            let file_path = format!("{}/{}", v_path, filename);
            let set_timeout = set_save_timeout_id;
            let set_status = set_save_status;

            let cb = Closure::once(move || {
                set_timeout.set(None);
                spawn_local(async move {
                    let args = serde_wasm_bindgen::to_value(&WriteFileArgs {
                        path: &file_path,
                        content: &new_text,
                    })
                    .unwrap();
                    invoke("write_file", args).await;
                    set_status.set("Saved".to_string());
                });
            });

            if let Ok(id) = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                220,
            ) {
                set_save_timeout_id.set(Some(id));
                cb.forget();
            } else {
                set_save_status.set("Save Failed".to_string());
            }
        }
    };

    let schedule_selection_restore = move |selection_to_restore: Selection| {
        let next_ticket = selection_restore_ticket.get_untracked().wrapping_add(1);
        set_selection_restore_ticket.set(next_ticket);
        let expected_ticket = next_ticket;
        let eref = editor_ref.clone();
        let cb = Closure::once(move || {
            if selection_restore_ticket.get_untracked() != expected_ticket {
                return;
            }
            if let Some(el) = eref.get() {
                if let Ok(root) = el.dyn_into::<HtmlElement>() {
                    set_selection_byte_offsets(&root, selection_to_restore);
                }
            }
        });
        if let Some(win) = leptos::web_sys::window() {
            let _ = win.request_animation_frame(cb.as_ref().unchecked_ref());
            cb.forget();
        }
    };

    Effect::new(move |_| {
        let cache = image_preview_cache.get();
        if cache.is_empty() || is_composing.get_untracked() {
            return;
        }

        let text = content.get_untracked();
        let file = current_file.get_untracked();
        let v_path = vault_path.get_untracked();
        if text.is_empty() || file.is_empty() || v_path.is_empty() {
            return;
        }

        let selection = editor_snapshot.get_untracked().selection.clamp(text.len());
        set_parsed_html.set(highlight_markdown_for_editor(
            &text,
            Some(selection.start),
            &v_path,
            &file,
            &cache,
        ));
        schedule_selection_restore(selection);
    });

    let apply_editor_update = move |new_text: String, sel_start: usize, sel_end: usize| {
        set_composition_dirty.set(false);
        let mut snapshot = editor_snapshot.get_untracked();
        let selection = Selection::new(sel_start, sel_end);
        snapshot.replace_from_input(new_text, selection);
        let final_text = snapshot.text.clone();
        let final_selection = snapshot.selection;
        set_editor_snapshot.set(snapshot);

        set_content.set(final_text.clone());
        set_caret_pos.set(Some(final_selection.start));
        set_parsed_html.set(highlight_markdown_for_editor(
            &final_text,
            Some(final_selection.start),
            &vault_path.get_untracked(),
            &current_file.get_untracked(),
            &image_preview_cache.get_untracked(),
        ));

        let file = current_file.get_untracked();
        if !file.is_empty() {
            let mut notes = note_texts.get_untracked();
            notes.insert(file.clone(), final_text.clone());
            let cache = build_metadata_cache(&notes, &files.get_untracked());
            set_note_texts.set(notes);
            set_metadata_cache.set(cache);
            schedule_disk_write(file, final_text.clone());
        }

        schedule_selection_restore(final_selection);
    };

    let apply_composition_shadow_update =
        move |new_text: String, sel_start: usize, sel_end: usize| {
            let mut snapshot = editor_snapshot.get_untracked();
            snapshot.replace_from_input(new_text.clone(), Selection::new(sel_start, sel_end));
            let selection = snapshot.selection;
            set_editor_snapshot.set(snapshot);
            set_content.set(new_text);
            set_caret_pos.set(Some(selection.start));
            set_composition_dirty.set(true);
        };

    let select_file = move |filename: String| {
        if let Some(text) = note_texts.get_untracked().get(&filename).cloned() {
            set_current_file.set(filename.clone());
            set_expanded_folders.update(|expanded| {
                expand_parent_folders(expanded, &filename);
            });
            set_content.set(text.clone());
            set_parsed_html.set(highlight_markdown_for_editor(
                &text,
                None,
                &vault_path.get_untracked(),
                &filename,
                &image_preview_cache.get_untracked(),
            ));
            set_caret_pos.set(None);
            set_editor_snapshot.set(EditorSnapshot::new(text));
            return;
        }

        let v_path = vault_path.get_untracked();
        if v_path.is_empty() {
            return;
        }
        spawn_local(async move {
            let file_path = format!("{}/{}", v_path, filename);
            let args = serde_wasm_bindgen::to_value(&ReadFileArgs { path: &file_path }).unwrap();
            let text_val = invoke("read_file", args).await;
            if let Some(text) = text_val.as_string() {
                set_current_file.set(filename.clone());
                set_expanded_folders.update(|expanded| {
                    expand_parent_folders(expanded, &filename);
                });
                set_content.set(text.clone());
                set_parsed_html.set(highlight_markdown_for_editor(
                    &text,
                    None,
                    &v_path,
                    &filename,
                    &image_preview_cache.get_untracked(),
                ));
                set_caret_pos.set(None);
                set_editor_snapshot.set(EditorSnapshot::new(text.clone()));

                let mut notes = note_texts.get_untracked();
                notes.insert(filename.clone(), text);
                set_metadata_cache.set(build_metadata_cache(&notes, &files.get_untracked()));
                set_note_texts.set(notes);
            }
        });
    };

    let schedule_selection_sync = move || {
        let next_ticket = selection_sync_ticket.get_untracked().wrapping_add(1);
        set_selection_sync_ticket.set(next_ticket);
        let expected_ticket = next_ticket;
        let cb = Closure::once(move || {
            if selection_sync_ticket.get_untracked() != expected_ticket {
                return;
            }
            let Some(el) = editor_ref.get() else {
                return;
            };
            let Ok(root) = el.dyn_into::<HtmlElement>() else {
                return;
            };
            let text = root.inner_text();
            if let Some(selection) = get_selection_byte_offsets(&root).map(|s| s.clamp(text.len()))
            {
                let mut snapshot = editor_snapshot.get_untracked();
                snapshot.set_selection(selection);
                set_editor_snapshot.set(snapshot);
                set_caret_pos.set(Some(selection.start));
                if !is_composing.get_untracked() {
                    set_parsed_html.set(highlight_markdown_for_editor(
                        &text,
                        Some(selection.start),
                        &vault_path.get_untracked(),
                        &current_file.get_untracked(),
                        &image_preview_cache.get_untracked(),
                    ));
                    schedule_selection_restore(selection);
                }
            }
        });
        if let Some(win) = leptos::web_sys::window() {
            let _ = win.request_animation_frame(cb.as_ref().unchecked_ref());
            cb.forget();
        }
    };

    let update_content = move |ev| {
        let target = event_target::<leptos::web_sys::Element>(&ev);
        let new_text = target
            .dyn_into::<HtmlElement>()
            .ok()
            .map(|el| el.inner_text())
            .unwrap_or_default();
        let selection = editor_ref
            .get()
            .and_then(|el| el.dyn_into::<HtmlElement>().ok())
            .and_then(|root| get_selection_byte_offsets(&root))
            .unwrap_or_else(|| Selection::cursor(new_text.len()))
            .clamp(new_text.len());

        if is_composing.get_untracked() {
            apply_composition_shadow_update(new_text, selection.start, selection.end);
        } else {
            apply_editor_update(new_text, selection.start, selection.end);
        }
    };

    let handle_editor_keydown = move |e: leptos::ev::KeyboardEvent| {
        if is_composing.get_untracked() {
            return;
        }
        let Some(el) = editor_ref.get() else {
            return;
        };
        let Ok(root) = el.dyn_into::<HtmlElement>() else {
            return;
        };

        let text = root.inner_text();
        let selection = get_selection_byte_offsets(&root)
            .unwrap_or_else(|| Selection::cursor(text.len()))
            .clamp(text.len());

        let mut snapshot = editor_snapshot.get_untracked();
        snapshot.replace_from_input(text.clone(), selection);
        set_editor_snapshot.set(snapshot.clone());

        let key = e.key();
        let ctrl_or_cmd = e.ctrl_key() || e.meta_key();

        if ctrl_or_cmd && !e.alt_key() {
            match key.as_str() {
                "b" | "B" => {
                    if apply_markdown_command(
                        &mut snapshot,
                        MarkdownCommand::Wrap {
                            open: "**",
                            close: "**",
                            label: "bold",
                        },
                    )
                    .unwrap_or(false)
                    {
                        e.prevent_default();
                        apply_editor_update(
                            snapshot.text.clone(),
                            snapshot.selection.start,
                            snapshot.selection.end,
                        );
                        return;
                    }
                }
                "i" | "I" => {
                    if apply_markdown_command(
                        &mut snapshot,
                        MarkdownCommand::Wrap {
                            open: "*",
                            close: "*",
                            label: "italic",
                        },
                    )
                    .unwrap_or(false)
                    {
                        e.prevent_default();
                        apply_editor_update(
                            snapshot.text.clone(),
                            snapshot.selection.start,
                            snapshot.selection.end,
                        );
                        return;
                    }
                }
                "k" | "K" => {
                    if apply_markdown_command(
                        &mut snapshot,
                        MarkdownCommand::Wrap {
                            open: "[[",
                            close: "]]",
                            label: "wikilink",
                        },
                    )
                    .unwrap_or(false)
                    {
                        e.prevent_default();
                        apply_editor_update(
                            snapshot.text.clone(),
                            snapshot.selection.start,
                            snapshot.selection.end,
                        );
                        return;
                    }
                }
                _ => {}
            }
        }

        if key == "Tab" {
            e.prevent_default();
            let command = if e.shift_key() {
                MarkdownCommand::Outdent
            } else {
                MarkdownCommand::Indent
            };
            if apply_markdown_command(&mut snapshot, command).unwrap_or(false) {
                apply_editor_update(
                    snapshot.text.clone(),
                    snapshot.selection.start,
                    snapshot.selection.end,
                );
            }
            return;
        }

        if key == "Enter" {
            e.prevent_default();
            if apply_markdown_command(&mut snapshot, MarkdownCommand::ContinueBlock)
                .unwrap_or(false)
            {
                apply_editor_update(
                    snapshot.text.clone(),
                    snapshot.selection.start,
                    snapshot.selection.end,
                );
                return;
            }
            let fallback_start = snapshot.selection.start;
            let fallback = Transaction::single(
                TextChange::new(snapshot.selection.start, snapshot.selection.end, "\n"),
                Some(Selection::cursor(fallback_start + 1)),
                ChangeOrigin::Command,
                "insert-newline",
            );
            if snapshot.apply_transaction(fallback).is_ok() {
                apply_editor_update(
                    snapshot.text.clone(),
                    snapshot.selection.start,
                    snapshot.selection.end,
                );
            }
            return;
        }

        if !ctrl_or_cmd && !e.alt_key() {
            let pair = match key.as_str() {
                "(" => Some(("(", ")")),
                "[" => Some(("[", "]")),
                "{" => Some(("{", "}")),
                "\"" => Some(("\"", "\"")),
                "'" => Some(("'", "'")),
                "`" => Some(("`", "`")),
                _ => None,
            };

            if let Some((open, close)) = pair {
                if apply_markdown_command(&mut snapshot, MarkdownCommand::AutoPair { open, close })
                    .unwrap_or(false)
                {
                    e.prevent_default();
                    apply_editor_update(
                        snapshot.text.clone(),
                        snapshot.selection.start,
                        snapshot.selection.end,
                    );
                }
            }
        }
    };

    let handle_editor_paste = move |e: leptos::ev::Event| {
        let Some(raw) = e.dyn_ref::<leptos::web_sys::ClipboardEvent>() else {
            return;
        };
        e.prevent_default();

        let pasted = raw
            .clipboard_data()
            .and_then(|dt| dt.get_data("text/plain").ok())
            .unwrap_or_default();
        if pasted.is_empty() {
            return;
        }
        let pasted = normalize_pasted_text(&pasted);

        let Some(el) = editor_ref.get() else {
            return;
        };
        let Ok(root) = el.dyn_into::<HtmlElement>() else {
            return;
        };

        let text = root.inner_text();
        let selection = get_selection_byte_offsets(&root)
            .unwrap_or_else(|| Selection::cursor(text.len()))
            .clamp(text.len());

        let mut snapshot = editor_snapshot.get_untracked();
        snapshot.replace_from_input(text, selection);
        let transaction = Transaction::single(
            TextChange::new(
                snapshot.selection.start,
                snapshot.selection.end,
                pasted.clone(),
            ),
            Some(Selection::cursor(snapshot.selection.start + pasted.len())),
            ChangeOrigin::Command,
            "paste-plain-text",
        );

        if snapshot.apply_transaction(transaction).is_ok() {
            apply_editor_update(
                snapshot.text.clone(),
                snapshot.selection.start,
                snapshot.selection.end,
            );
        }
    };

    let handle_composition_start = move |_| {
        set_is_composing.set(true);
        set_composition_dirty.set(false);
    };

    let handle_composition_end = move |_| {
        set_is_composing.set(false);
        if !composition_dirty.get_untracked() {
            return;
        }
        let Some(el) = editor_ref.get() else {
            return;
        };
        let Ok(root) = el.dyn_into::<HtmlElement>() else {
            return;
        };
        let text = root.inner_text();
        let selection = get_selection_byte_offsets(&root)
            .unwrap_or_else(|| Selection::cursor(text.len()))
            .clamp(text.len());
        set_composition_dirty.set(false);
        apply_editor_update(text, selection.start, selection.end);
    };

    let run_editor_action = move |action: &'static str| {
        let Some(el) = editor_ref.get() else {
            return;
        };
        let Ok(root) = el.dyn_into::<HtmlElement>() else {
            return;
        };

        let text = root.inner_text();
        let selection = get_selection_byte_offsets(&root)
            .unwrap_or_else(|| Selection::cursor(text.len()))
            .clamp(text.len());

        let mut snapshot = editor_snapshot.get_untracked();
        snapshot.replace_from_input(text, selection);

        let command = match action {
            "bold" => MarkdownCommand::Wrap {
                open: "**",
                close: "**",
                label: "bold",
            },
            "italic" => MarkdownCommand::Wrap {
                open: "*",
                close: "*",
                label: "italic",
            },
            "code" => MarkdownCommand::Wrap {
                open: "`",
                close: "`",
                label: "code",
            },
            "link" => MarkdownCommand::Wrap {
                open: "[[",
                close: "]]",
                label: "wikilink",
            },
            "quote" => MarkdownCommand::PrefixLine {
                prefix: "> ",
                label: "quote",
            },
            "task" => MarkdownCommand::PrefixLine {
                prefix: "- [ ] ",
                label: "task",
            },
            _ => return,
        };

        if apply_markdown_command(&mut snapshot, command).unwrap_or(false) {
            apply_editor_update(
                snapshot.text.clone(),
                snapshot.selection.start,
                snapshot.selection.end,
            );
        }
    };

    let save_settings_to_disk = move |s: AppSettings| {
        let v_path = vault_path.get_untracked();
        if v_path.is_empty() {
            return;
        }
        if let Ok(s_json) = serde_json::to_string(&s) {
            spawn_local(async move {
                let args = serde_wasm_bindgen::to_value(&SaveSettingsArgs {
                    vault_path: &v_path,
                    settings: &s_json,
                })
                .unwrap();
                invoke("save_settings", args).await;
            });
        }
    };

    let create_new_note = move || {
        let v_path = vault_path.get_untracked();
        if v_path.is_empty() {
            return;
        }

        if let Ok(Some(raw)) = window().prompt_with_message("New note path") {
            let name = normalize_rel_path(raw.trim());
            if name.is_empty() {
                return;
            }
            let filename = if name.to_ascii_lowercase().ends_with(".md") {
                name
            } else {
                format!("{name}.md")
            };
            let file_path = format!("{}/{}", v_path, filename);
            let initial = "# New Note\n\n".to_string();
            let path_for_refresh = v_path.clone();
            let file_for_refresh = filename.clone();

            spawn_local(async move {
                let args = serde_wasm_bindgen::to_value(&WriteFileArgs {
                    path: &file_path,
                    content: &initial,
                })
                .unwrap();
                invoke("write_file", args).await;
                refresh_vault_snapshot(path_for_refresh, Some(file_for_refresh));
            });
        }
    };

    let create_note_in_folder = move |folder_path: String| {
        set_sidebar_context_menu.set(None);
        let v_path = vault_path.get_untracked();
        if v_path.is_empty() {
            return;
        }
        if let Ok(Some(raw)) = window().prompt_with_message_and_default("New note name", "New Note") {
            let name = raw.trim().replace('\\', "/").trim_matches('/').to_string();
            if name.is_empty() || name.contains('/') {
                return;
            }
            let base = if name.to_ascii_lowercase().ends_with(".md") { name } else { format!("{name}.md") };
            let filename = if folder_path.is_empty() { base } else { format!("{}/{}", folder_path, base) };
            let file_path = format!("{}/{}", v_path, filename);
            let initial = "# New Note\n\n".to_string();
            let path_for_refresh = v_path.clone();
            let file_for_refresh = filename.clone();
            set_expanded_folders.update(|expanded| {
                expand_parent_folders(expanded, &filename);
            });
            spawn_local(async move {
                let args = serde_wasm_bindgen::to_value(&WriteFileArgs {
                    path: &file_path,
                    content: &initial,
                })
                .unwrap();
                invoke("write_file", args).await;
                refresh_vault_snapshot(path_for_refresh, Some(file_for_refresh));
            });
        }
    };

    let create_folder_in_folder = move |folder_path: String| {
        set_sidebar_context_menu.set(None);
        let v_path = vault_path.get_untracked();
        if v_path.is_empty() {
            return;
        }
        let default_name = "New folder";
        if let Ok(Some(raw)) = window().prompt_with_message_and_default("New folder name", default_name) {
            let name = normalize_rel_path(raw.trim());
            if name.is_empty() || name.contains('/') {
                return;
            }
            let full_path = if folder_path.is_empty() {
                format!("{}/{}", v_path, name)
            } else {
                format!("{}/{}/{}", v_path, folder_path, name)
            };
            let path_for_refresh = v_path.clone();
            spawn_local(async move {
                let args = serde_wasm_bindgen::to_value(&ReadFileArgs { path: &full_path }).unwrap();
                let _ = invoke("create_dir", args).await;
                refresh_vault_snapshot(path_for_refresh, None);
            });
        }
    };

    let delete_note = move |file_path: String| {
        set_sidebar_context_menu.set(None);
        let v_path = vault_path.get_untracked();
        if v_path.is_empty() || !window().confirm_with_message(&format!("Delete note \"{}\"?", file_path)).unwrap_or(false) {
            return;
        }
        let full_path = format!("{}/{}", v_path, file_path);
        let path_for_refresh = v_path.clone();
        let next_file = if current_file.get_untracked() == file_path { None } else { Some(current_file.get_untracked().clone()) };
        spawn_local(async move {
            let args = serde_wasm_bindgen::to_value(&ReadFileArgs { path: &full_path }).unwrap();
            let _ = invoke("delete_file", args).await;
            refresh_vault_snapshot(path_for_refresh, next_file);
        });
    };

    let delete_folder = move |folder_path: String| {
        set_sidebar_context_menu.set(None);
        let v_path = vault_path.get_untracked();
        if v_path.is_empty() || !window().confirm_with_message(&format!("Delete folder \"{}\" and its contents?", folder_path)).unwrap_or(false) {
            return;
        }
        let full_path = format!("{}/{}", v_path, folder_path);
        let path_for_refresh = v_path.clone();
        let current = current_file.get_untracked();
        let next_file = if !current.is_empty() && current != folder_path && !current.starts_with(&format!("{}/", folder_path)) {
            Some(current.clone())
        } else {
            None
        };
        spawn_local(async move {
            let args = serde_wasm_bindgen::to_value(&ReadFileArgs { path: &full_path }).unwrap();
            let _ = invoke("delete_dir", args).await;
            refresh_vault_snapshot(path_for_refresh, next_file);
        });
    };

    let rename_current_note = move || {
        let v_path = vault_path.get_untracked();
        let old_name = current_file.get_untracked();
        if v_path.is_empty() || old_name.is_empty() {
            return;
        }

        if let Ok(Some(raw)) = window().prompt_with_message_and_default("Rename note", &old_name) {
            let next_name = normalize_rel_path(raw.trim());
            if next_name.is_empty() {
                return;
            }
            let final_name = if next_name.to_ascii_lowercase().ends_with(".md") {
                next_name
            } else {
                format!("{next_name}.md")
            };
            if final_name == old_name {
                return;
            }

            let path_for_refresh = v_path.clone();
            let old_for_api = old_name.clone();
            let next_for_api = final_name.clone();
            spawn_local(async move {
                let args = serde_wasm_bindgen::to_value(&RenameNoteArgs {
                    vault_path: &v_path,
                    old_path: &old_for_api,
                    new_path: &next_for_api,
                })
                .unwrap();
                let result = invoke("rename_note", args).await;
                let selected = result.as_string().unwrap_or(next_for_api);
                refresh_vault_snapshot(path_for_refresh, Some(selected));
            });
        }
    };

    let import_from_obsidian_vault = move || {
        spawn_local(async move {
            let result = invoke("import_obsidian_vault_with_picker", JsValue::NULL).await;
            let Ok(report) = serde_wasm_bindgen::from_value::<VaultImportReport>(result) else {
                let _ = window()
                    .alert_with_message("Import failed: backend returned an invalid response.");
                return;
            };

            let mut summary = report.message.clone();
            if report.success {
                if let Some(source) = report.source_vault.as_ref() {
                    summary.push_str(&format!("\nSource: {source}"));
                }
                if let Some(destination) = report.destination_vault.as_ref() {
                    summary.push_str(&format!("\nDestination: {destination}"));
                }
                summary.push_str(&format!(
                    "\nNotes: scanned {} imported {} | Images: scanned {} imported {} | Renamed: {}",
                    report.scanned_notes,
                    report.imported_notes,
                    report.scanned_images,
                    report.imported_images,
                    report.renamed_notes
                ));
            } else if report.cancelled {
                summary = "Import cancelled by user.".to_string();
            }
            let _ = window().alert_with_message(&summary);

            if report.success {
                if let Some(destination) = report.destination_vault {
                    activate_vault(destination, None);
                }
            }
        });
    };

    let open_bedrock_vault = move || {
        spawn_local(async move {
            let result = invoke("pick_bedrock_vault", JsValue::NULL).await;
            let picked = serde_wasm_bindgen::from_value::<Option<String>>(result)
                .ok()
                .flatten();
            let Some(path) = picked else {
                return;
            };
            activate_vault(path, None);
        });
    };

    let switch_to_vault = move |path: String| {
        if path.is_empty() {
            return;
        }
        let current = collapse_path(&vault_path.get_untracked());
        let next = collapse_path(&path);
        if current == next {
            return;
        }
        activate_vault(path, None);
    };

    let close_current_vault = move || {
        let current = vault_path.get_untracked();
        if current.is_empty() {
            return;
        }

        let current_norm = collapse_path(&current);
        let mut next_vault = None::<String>;
        set_open_vaults.update(|vaults| {
            vaults.retain(|p| collapse_path(p) != current_norm);
            next_vault = vaults.last().cloned();
        });

        if let Some(path) = next_vault {
            activate_vault(path, None);
        } else {
            set_vault_path.set(String::new());
            clear_active_vault_state();
            persist_vault_session(Vec::new(), None);
        }
    };

    let dynamic_style = move || {
        let s = settings.get();
        format!(
            "--editor-font-size: {}px; --accent-color: {}; --bg-primary: {}; --bg-secondary: {}; --text-primary: {}; --md-h1-color: {}; --md-h2-color: {}; --md-h3-color: {}; --md-h4-color: {}; --md-bold-color: {}; --md-italic-color: {}; --md-code-bg: {}; --md-code-text: {}; --md-quote-color: {};",
            s.font_size,
            s.accent_color,
            s.bg_primary,
            s.bg_secondary,
            s.text_primary,
            s.md_h1_color,
            s.md_h2_color,
            s.md_h3_color,
            s.md_h4_color,
            s.md_bold_color,
            s.md_italic_color,
            s.md_code_bg,
            s.md_code_text,
            s.md_quote_color
        )
    };

    let app_view = move || {
        if is_settings_window {
            view! {
                <div style="flex: 1; padding: 3rem; overflow-y: auto;">
                    <h2 style="margin-top: 0; margin-bottom: 2rem;">"Theme Settings"</h2>

                    <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 1.5rem;">
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label style="font-weight: 600; font-size: 0.9em;">"Editor Font Size (px)"</label>
                            <input
                                style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--border-color); background: var(--bg-secondary); color: var(--text-primary); width: 100%; box-sizing: border-box;"
                                type="number"
                                prop:value=move || settings.get().font_size.to_string()
                                on:input=move |e| {
                                    let mut s = settings.get_untracked();
                                    s.font_size = event_target_value(&e).parse().unwrap_or(16);
                                    set_settings.set(s.clone());
                                    save_settings_to_disk(s);
                                }
                            />
                        </div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label style="font-weight: 600; font-size: 0.9em;">"Accent Color"</label>
                            <input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().accent_color.clone() on:input=move |e| {
                                let mut s = settings.get_untracked();
                                s.accent_color = event_target_value(&e);
                                set_settings.set(s.clone());
                                save_settings_to_disk(s);
                            } />
                        </div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label style="font-weight: 600; font-size: 0.9em;">"Background Primary"</label>
                            <input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().bg_primary.clone() on:input=move |e| {
                                let mut s = settings.get_untracked();
                                s.bg_primary = event_target_value(&e);
                                set_settings.set(s.clone());
                                save_settings_to_disk(s);
                            } />
                        </div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label style="font-weight: 600; font-size: 0.9em;">"Background Secondary"</label>
                            <input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().bg_secondary.clone() on:input=move |e| {
                                let mut s = settings.get_untracked();
                                s.bg_secondary = event_target_value(&e);
                                set_settings.set(s.clone());
                                save_settings_to_disk(s);
                            } />
                        </div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label style="font-weight: 600; font-size: 0.9em;">"Text Primary"</label>
                            <input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().text_primary.clone() on:input=move |e| {
                                let mut s = settings.get_untracked();
                                s.text_primary = event_target_value(&e);
                                set_settings.set(s.clone());
                                save_settings_to_disk(s);
                            } />
                        </div>
                    </div>

                    <h3 style="margin-top: 2.5rem; border-bottom: 1px solid var(--border-color); padding-bottom: 0.5rem;">"Markdown Colors"</h3>
                    <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 1.5rem; margin-top: 1.5rem;">
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;"><label style="font-weight: 600; font-size: 0.9em;">"H1 Color"</label><input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().md_h1_color.clone() on:input=move |e| { let mut s = settings.get_untracked(); s.md_h1_color = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s); } /></div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;"><label style="font-weight: 600; font-size: 0.9em;">"H2 Color"</label><input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().md_h2_color.clone() on:input=move |e| { let mut s = settings.get_untracked(); s.md_h2_color = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s); } /></div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;"><label style="font-weight: 600; font-size: 0.9em;">"H3 Color"</label><input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().md_h3_color.clone() on:input=move |e| { let mut s = settings.get_untracked(); s.md_h3_color = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s); } /></div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;"><label style="font-weight: 600; font-size: 0.9em;">"H4 Color"</label><input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().md_h4_color.clone() on:input=move |e| { let mut s = settings.get_untracked(); s.md_h4_color = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s); } /></div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;"><label style="font-weight: 600; font-size: 0.9em;">"Bold Text Color"</label><input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().md_bold_color.clone() on:input=move |e| { let mut s = settings.get_untracked(); s.md_bold_color = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s); } /></div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;"><label style="font-weight: 600; font-size: 0.9em;">"Italic Text Color"</label><input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().md_italic_color.clone() on:input=move |e| { let mut s = settings.get_untracked(); s.md_italic_color = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s); } /></div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;"><label style="font-weight: 600; font-size: 0.9em;">"Code Background"</label><input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().md_code_bg.clone() on:input=move |e| { let mut s = settings.get_untracked(); s.md_code_bg = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s); } /></div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;"><label style="font-weight: 600; font-size: 0.9em;">"Code Text Color"</label><input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().md_code_text.clone() on:input=move |e| { let mut s = settings.get_untracked(); s.md_code_text = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s); } /></div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;"><label style="font-weight: 600; font-size: 0.9em;">"Blockquote Color"</label><input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().md_quote_color.clone() on:input=move |e| { let mut s = settings.get_untracked(); s.md_quote_color = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s); } /></div>
                    </div>
                </div>
            }
            .into_any()
        } else {
            view! {
                <nav class="sidebar" style="width: var(--sidebar-width); border-right: 1px solid var(--border-color); display: flex; flex-direction: column; background: var(--bg-secondary); transition: all 0.3s ease;">
                    <div class="sidebar-header" style="display: flex; flex-direction: column; gap: 0.5rem; padding: 0.65rem 0.75rem; border-bottom: 1px solid var(--border-color);">
                        <div style="display: flex; align-items: center; justify-content: space-between; gap: 0.5rem;">
                            <span style="font-weight: 700; color: var(--accent-color);">"Bedrock"</span>
                            <div style="display: flex; gap: 0.3rem; align-items: center;">
                                <button
                                    on:click=move |_| open_bedrock_vault()
                                    style="background: transparent; border: none; font-size: 0.95rem; cursor: pointer; color: var(--text-muted);"
                                    title="Open or add a vault"
                                >
                                    "🗂"
                                </button>
                                <button
                                    on:click=move |_| close_current_vault()
                                    style="background: transparent; border: none; font-size: 0.95rem; cursor: pointer; color: var(--text-muted);"
                                    title="Close current vault"
                                >
                                    "✕"
                                </button>
                                <button
                                    on:click=move |_| create_new_note()
                                    style="background: transparent; border: none; font-size: 1.1rem; cursor: pointer; color: var(--text-muted);"
                                    title="New note"
                                >
                                    "+"
                                </button>
                                <button
                                    on:click=move |_| import_from_obsidian_vault()
                                    style="background: transparent; border: none; font-size: 1rem; cursor: pointer; color: var(--text-muted);"
                                    title="Import from Obsidian vault"
                                >
                                    "⇪"
                                </button>
                                <button
                                    on:click=move |_| {
                                        spawn_local(async move {
                                            invoke("open_settings_window", JsValue::NULL).await;
                                        });
                                    }
                                    style="background: transparent; border: none; font-size: 1.1rem; cursor: pointer; color: var(--text-muted);"
                                    title="Settings"
                                >
                                    "⚙"
                                </button>
                            </div>
                        </div>
                        <div style="display: flex; flex-direction: column; gap: 0.25rem;">
                            <select
                                style="width: 100%; font-size: 0.78rem; padding: 0.2rem 0.35rem; border-radius: var(--radius-sm); border: 1px solid var(--border-color); background: var(--bg-primary); color: var(--text-primary);"
                                prop:value=move || vault_path.get()
                                on:change=move |e| switch_to_vault(event_target_value(&e))
                            >
                                {move || {
                                    let vaults = open_vaults.get();
                                    if vaults.is_empty() {
                                        vec![view! { <option value=String::new()>{"No open vaults".to_string()}</option> }]
                                    } else {
                                        vaults
                                            .into_iter()
                                            .map(|path| {
                                                let label = vault_display_name(&path);
                                                view! { <option value=path.clone()>{label}</option> }
                                            })
                                            .collect::<Vec<_>>()
                                    }
                                }}
                            </select>
                            <span
                                style="font-size: 0.72rem; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;"
                                title=move || vault_path.get()
                            >
                                {move || {
                                    let path = vault_path.get();
                                    if path.is_empty() {
                                        "No active vault".to_string()
                                    } else {
                                        format!("Active: {}", vault_display_name(&path))
                                    }
                                }}
                            </span>
                        </div>
                    </div>
                    <div class="file-list" style="flex: 1; overflow-y: auto; padding: 0.75rem 0.5rem;">
                        {move || {
                            let files_in_vault = files.get();
                            if files_in_vault.is_empty() {
                                let msg = if vault_path.get().is_empty() {
                                    "Open a Bedrock vault to see notes."
                                } else {
                                    "No markdown notes in this vault."
                                };
                                return view! {
                                    <div style="padding: 0.5rem 0.75rem; font-size: 0.82rem; color: var(--text-muted);">
                                        {msg}
                                    </div>
                                }.into_any();
                            }

                            let mut tree = build_file_tree(&files_in_vault);
                            add_empty_dirs_to_tree(&mut tree, &empty_dirs.get());
                            let rows = build_sidebar_entries(&tree, &expanded_folders.get());

                            view! {
                                <>
                                    {rows.into_iter().map(|row| {
                                        match row {
                                            SidebarEntry::Folder { path, name, depth, note_count, expanded } => {
                                                let toggle_path = path.clone();
                                                let context_path = path.clone();
                                                let indent = 0.45 + (depth as f32 * 0.95);
                                                let chevron = if expanded { "▾" } else { "▸" };
                                                let row_bg = if expanded {
                                                    "background: color-mix(in srgb, var(--accent-color) 11%, transparent);"
                                                } else {
                                                    ""
                                                };

                                                view! {
                                                    <div
                                                        class="folder-item"
                                                        style=format!(
                                                            "display: flex; align-items: center; gap: 0.4rem; padding: 0.34rem 0.5rem 0.34rem {indent}rem; cursor: pointer; border-radius: var(--radius-md); margin-bottom: 2px; font-size: 0.82rem; color: var(--text-secondary); transition: background 0.15s ease; {row_bg}"
                                                        )
                                                        on:click=move |_| {
                                                            set_expanded_folders.update(|expanded_set| {
                                                                if expanded_set.contains(&toggle_path) {
                                                                    expanded_set.remove(&toggle_path);
                                                                } else {
                                                                    expanded_set.insert(toggle_path.clone());
                                                                }
                                                            });
                                                        }
                                                        on:contextmenu=move |ev: leptos::ev::MouseEvent| {
                                                            ev.prevent_default();
                                                            set_sidebar_context_menu.set(Some(SidebarContextMenu::Folder { path: context_path.clone(), x: ev.client_x() as f64, y: ev.client_y() as f64 }));
                                                        }
                                                        title=path
                                                    >
                                                        <span style="width: 0.8rem; text-align: center; color: var(--text-muted);">{chevron}</span>
                                                        <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{name}</span>
                                                        <span style="margin-left: auto; font-size: 0.72rem; color: var(--text-muted);">{note_count.to_string()}</span>
                                                    </div>
                                                }.into_any()
                                            }
                                            SidebarEntry::File { path, name, depth } => {
                                                let filename = path.clone();
                                                let active_path = path.clone();
                                                let context_file_path = path.clone();
                                                let is_active = move || current_file.get() == active_path;
                                                let indent = 1.5 + (depth as f32 * 0.95);

                                                view! {
                                                    <div
                                                        class="file-item"
                                                        style=move || format!(
                                                            "padding: 0.38rem 0.65rem 0.38rem {indent}rem; cursor: pointer; border-radius: var(--radius-md); margin-bottom: 2px; font-size: 0.84rem; transition: background 0.2s, color 0.2s; {}",
                                                            if is_active() { "background: var(--accent-color); color: white;" } else { "color: var(--text-secondary);" }
                                                        )
                                                        on:click=move |_| select_file(filename.clone())
                                                        on:contextmenu=move |ev: leptos::ev::MouseEvent| {
                                                            ev.prevent_default();
                                                            set_sidebar_context_menu.set(Some(SidebarContextMenu::File { path: context_file_path.clone(), x: ev.client_x() as f64, y: ev.client_y() as f64 }));
                                                        }
                                                        title=path
                                                    >
                                                        {name}
                                                    </div>
                                                }.into_any()
                                            }
                                        }
                                    }).collect::<Vec<_>>()}
                                </>
                            }.into_any()
                        }}
                    </div>
                </nav>
                {move || match sidebar_context_menu.get() {
                    Some(SidebarContextMenu::Folder { path, x, y }) => {
                        let path_for_note = path.clone();
                        let path_for_folder = path.clone();
                        let path_for_delete = path.clone();
                        view! {
                            <div
                                style="position: fixed; inset: 0; z-index: 1000;"
                                on:click=move |_| set_sidebar_context_menu.set(None)
                            >
                                <div
                                    style=format!("position: absolute; left: {}px; top: {}px; background: var(--bg-secondary); border: 1px solid var(--border-color); border-radius: var(--radius-md); padding: 0.25rem 0; box-shadow: 0 4px 12px rgba(0,0,0,0.15); min-width: 8rem;", x, y)
                                    on:click=move |ev| ev.stop_propagation()
                                >
                                    <button
                                        style="display: block; width: 100%; padding: 0.4rem 0.75rem; text-align: left; font-size: 0.85rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary);"
                                        on:click=move |_| create_note_in_folder(path_for_note.clone())
                                    >
                                        "New note"
                                    </button>
                                    <button
                                        style="display: block; width: 100%; padding: 0.4rem 0.75rem; text-align: left; font-size: 0.85rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary);"
                                        on:click=move |_| create_folder_in_folder(path_for_folder.clone())
                                    >
                                        "New folder"
                                    </button>
                                    <button
                                        style="display: block; width: 100%; padding: 0.4rem 0.75rem; text-align: left; font-size: 0.85rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary);"
                                        on:click=move |_| delete_folder(path_for_delete.clone())
                                    >
                                        "Delete folder"
                                    </button>
                                </div>
                            </div>
                        }.into_any()
                    }
                    Some(SidebarContextMenu::File { path, x, y }) => {
                        let path_for_delete = path.clone();
                        view! {
                            <div
                                style="position: fixed; inset: 0; z-index: 1000;"
                                on:click=move |_| set_sidebar_context_menu.set(None)
                            >
                                <div
                                    style=format!("position: absolute; left: {}px; top: {}px; background: var(--bg-secondary); border: 1px solid var(--border-color); border-radius: var(--radius-md); padding: 0.25rem 0; box-shadow: 0 4px 12px rgba(0,0,0,0.15); min-width: 8rem;", x, y)
                                    on:click=move |ev| ev.stop_propagation()
                                >
                                    <button
                                        style="display: block; width: 100%; padding: 0.4rem 0.75rem; text-align: left; font-size: 0.85rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary);"
                                        on:click=move |_| delete_note(path_for_delete.clone())
                                    >
                                        "Delete note"
                                    </button>
                                </div>
                            </div>
                        }.into_any()
                    }
                    None => view! { <></> }.into_any(),
                }}
                <section class="editor-pane" style="flex: 1; display: flex; flex-direction: column; background: var(--bg-primary); min-width: 0;">
                    {move || if current_file.get().is_empty() {
                        let has_vault = !vault_path.get().is_empty();
                        let message = if has_vault {
                            "Select a note from the sidebar to start editing."
                        } else {
                            "Open a Bedrock vault to begin."
                        };
                        view! {
                            <div style="flex: 1; display: flex; align-items: center; justify-content: center; color: var(--text-muted);">
                                {message}
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <header class="topbar" style="height: var(--topbar-height); border-bottom: 1px solid var(--border-color); display: flex; align-items: center; justify-content: space-between; padding: 0 1.5rem; color: var(--text-muted); font-size: 0.9rem; gap: 1rem;">
                                <div style="display: flex; align-items: center; gap: 0.75rem; min-width: 0;">
                                    <div style="display: flex; flex-direction: column; min-width: 0; gap: 0.1rem;">
                                        <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{move || current_file.get()}</span>
                                        <span style="font-size: 0.72rem; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title=move || vault_path.get()>{move || format!("Vault: {}", vault_display_name(&vault_path.get()))}</span>
                                    </div>
                                    <span style="font-size: 0.8rem; color: var(--text-muted);">{move || save_status.get()}</span>
                                </div>
                                <div style="display: flex; gap: 0.5rem;">
                                    <button style="padding: 0.25rem 0.6rem; font-size: 0.75rem;" on:click=move |_| open_bedrock_vault()>"Open Vault"</button>
                                    <button style="padding: 0.25rem 0.6rem; font-size: 0.75rem;" on:click=move |_| import_from_obsidian_vault()>"Import Obsidian"</button>
                                    <button style="padding: 0.25rem 0.6rem; font-size: 0.75rem;" on:click=move |_| rename_current_note()>"Rename"</button>
                                </div>
                            </header>
                            <div class="editor-toolbar" style="display: flex; align-items: center; gap: 0.5rem; padding: 0.5rem 1.25rem; border-bottom: 1px solid var(--border-color); background: var(--bg-secondary);">
                                <button style="padding: 0.2rem 0.5rem; font-size: 0.75rem;" on:click=move |_| run_editor_action("bold")>"Bold"</button>
                                <button style="padding: 0.2rem 0.5rem; font-size: 0.75rem;" on:click=move |_| run_editor_action("italic")>"Italic"</button>
                                <button style="padding: 0.2rem 0.5rem; font-size: 0.75rem;" on:click=move |_| run_editor_action("code")>"Code"</button>
                                <button style="padding: 0.2rem 0.5rem; font-size: 0.75rem;" on:click=move |_| run_editor_action("link")>"WikiLink"</button>
                                <button style="padding: 0.2rem 0.5rem; font-size: 0.75rem;" on:click=move |_| run_editor_action("quote")>"Quote"</button>
                                <button style="padding: 0.2rem 0.5rem; font-size: 0.75rem;" on:click=move |_| run_editor_action("task")>"Task"</button>
                                <div style="margin-left: auto; display: flex; align-items: center; gap: 0.6rem;">
                                    <button
                                        style="padding: 0.2rem 0.5rem; font-size: 0.75rem;"
                                        on:click=move |_| {
                                            set_show_markdown_syntax.update(|v| *v = !*v);
                                        }
                                    >
                                        {move || if show_markdown_syntax.get() { "Hide Markdown" } else { "Show Markdown" }}
                                    </button>
                                    <span style="font-size: 0.75rem; color: var(--text-muted);">"Cmd/Ctrl+B, I, K • Tab/Shift+Tab • Enter continues lists"</span>
                                </div>
                            </div>
                            <div class="editor-container" style="flex: 1; position: relative; overflow: hidden; background: var(--bg-primary);">
                                <div
                                    node_ref=editor_ref
                                    class="editor-surface"
                                    class:show-syntax=move || show_markdown_syntax.get()
                                    style="width: 100%; height: 100%; padding: 2rem 3rem; font-family: var(--font-editor); font-size: var(--editor-font-size); line-height: 1.6; color: var(--text-primary); white-space: pre-wrap; word-wrap: break-word; box-sizing: border-box; overflow-y: auto; outline: none; caret-color: var(--text-primary);"
                                    contenteditable="true"
                                    spellcheck="false"
                                    inner_html=move || parsed_html.get()
                                    on:keydown=handle_editor_keydown
                                    on:input=update_content
                                    on:paste=handle_editor_paste
                                    on:compositionstart=handle_composition_start
                                    on:compositionend=handle_composition_end
                                    on:click=move |_| schedule_selection_sync()
                                    on:keyup=move |e: leptos::ev::KeyboardEvent| {
                                        if is_selection_navigation_key(&e.key()) {
                                            schedule_selection_sync();
                                        }
                                    }
                                    on:mouseup=move |_| schedule_selection_sync()
                                    on:focus=move |_| schedule_selection_sync()
                                ></div>
                            </div>
                        }.into_any()
                    }}
                </section>
                <aside style="width: 300px; border-left: 1px solid var(--border-color); background: var(--bg-secondary); display: flex; flex-direction: column; min-width: 0;">
                    <header style="height: var(--topbar-height); display: flex; align-items: center; padding: 0 1rem; border-bottom: 1px solid var(--border-color); color: var(--text-muted); font-size: 0.85rem;">
                        "Metadata Cache"
                    </header>
                    <div style="padding: 1rem; overflow-y: auto; display: flex; flex-direction: column; gap: 1rem;">
                        <div style="font-size: 0.82rem; color: var(--text-muted);">
                            {move || {
                                let note_count = files.get().len();
                                let tag_count = metadata_cache.get().tags_index.len();
                                format!("{} indexed notes • {} unique tags", note_count, tag_count)
                            }}
                        </div>

                        {move || {
                            let current = current_file.get();
                            if current.is_empty() {
                                return view! { <div style="color: var(--text-muted); font-size: 0.85rem;">"Open a note to inspect headings, links, and backlinks."</div> }.into_any();
                            }

                            let cache = metadata_cache.get();
                            let file_cache = cache.file_cache.get(&current).cloned().unwrap_or_default();

                            let backlinks = cache.backlinks.get(&current).cloned().unwrap_or_default();

                            let mut resolved = cache
                                .resolved_links
                                .get(&current)
                                .cloned()
                                .unwrap_or_default()
                                .into_iter()
                                .collect::<Vec<_>>();
                            resolved.sort_by(|a, b| a.0.cmp(&b.0));

                            let mut unresolved = cache
                                .unresolved_links
                                .get(&current)
                                .cloned()
                                .unwrap_or_default()
                                .into_iter()
                                .collect::<Vec<_>>();
                            unresolved.sort_by(|a, b| a.0.cmp(&b.0));

                            let mut linked_tags = file_cache.tags.clone();
                            linked_tags.sort();
                            linked_tags.dedup();

                            view! {
                                <>
                                    <section class="meta-block">
                                        <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">"Tags"</h4>
                                        {if linked_tags.is_empty() {
                                            view! { <div style="font-size: 0.85rem; color: var(--text-muted);">"No tags"</div> }.into_any()
                                        } else {
                                            view! {
                                                <div style="display: flex; flex-wrap: wrap; gap: 0.4rem;">
                                                    {linked_tags.into_iter().map(|tag| {
                                                        view! {
                                                            <span style="font-size: 0.75rem; padding: 0.2rem 0.4rem; border-radius: 999px; background: color-mix(in srgb, var(--accent-color) 14%, transparent); color: var(--accent-color);">{format!("#{}", tag)}</span>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            }.into_any()
                                        }}
                                    </section>

                                    <section class="meta-block">
                                        <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">"Headings"</h4>
                                        {if file_cache.headings.is_empty() {
                                            view! { <div style="font-size: 0.85rem; color: var(--text-muted);">"No headings"</div> }.into_any()
                                        } else {
                                            view! {
                                                <div style="display: flex; flex-direction: column; gap: 0.35rem;">
                                                    {file_cache.headings.into_iter().map(|h| {
                                                        view! {
                                                            <div style="font-size: 0.82rem; color: var(--text-secondary); display: flex; gap: 0.4rem; align-items: baseline;">
                                                                <span style="font-family: var(--font-mono); color: var(--text-muted);">{format!("H{}", h.level)}</span>
                                                                <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{h.text}</span>
                                                                <span style="margin-left: auto; color: var(--text-muted);">{format!("L{}", h.line)}</span>
                                                            </div>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            }.into_any()
                                        }}
                                    </section>

                                    <section class="meta-block">
                                        <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">"Outgoing Links"</h4>
                                        {if resolved.is_empty() {
                                            view! { <div style="font-size: 0.85rem; color: var(--text-muted);">"No resolved links"</div> }.into_any()
                                        } else {
                                            view! {
                                                <div style="display: flex; flex-direction: column; gap: 0.3rem;">
                                                    {resolved.into_iter().map(|(path, count)| {
                                                        view! {
                                                            <div style="font-size: 0.82rem; color: var(--text-secondary); display: flex; gap: 0.5rem;">
                                                                <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{path}</span>
                                                                <span style="margin-left: auto; color: var(--text-muted);">{count.to_string()}</span>
                                                            </div>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            }.into_any()
                                        }}
                                    </section>

                                    <section class="meta-block">
                                        <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">"Backlinks"</h4>
                                        {if backlinks.is_empty() {
                                            view! { <div style="font-size: 0.85rem; color: var(--text-muted);">"No backlinks"</div> }.into_any()
                                        } else {
                                            view! {
                                                <div style="display: flex; flex-direction: column; gap: 0.3rem;">
                                                    {backlinks.into_iter().map(|source| {
                                                        view! {
                                                            <div style="font-size: 0.82rem; color: var(--text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{source}</div>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            }.into_any()
                                        }}
                                    </section>

                                    <section class="meta-block">
                                        <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">"Unresolved"</h4>
                                        {if unresolved.is_empty() {
                                            view! { <div style="font-size: 0.85rem; color: var(--text-muted);">"No unresolved links"</div> }.into_any()
                                        } else {
                                            view! {
                                                <div style="display: flex; flex-direction: column; gap: 0.3rem;">
                                                    {unresolved.into_iter().map(|(target, count)| {
                                                        view! {
                                                            <div style="font-size: 0.82rem; color: #f97316; display: flex; gap: 0.5rem;">
                                                                <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{target}</span>
                                                                <span style="margin-left: auto; color: var(--text-muted);">{count.to_string()}</span>
                                                            </div>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            }.into_any()
                                        }}
                                    </section>
                                </>
                            }.into_any()
                        }}
                    </div>
                </aside>
            }
            .into_any()
        }
    };

    view! {
        <style>{move || plugin_css.get()}</style>
        <main class="app-layout" style=move || format!("display: flex; height: 100vh; width: 100vw; background: var(--bg-primary); color: var(--text-primary); {}", dynamic_style())>
            {app_view}
        </main>
    }
}
