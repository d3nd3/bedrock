use crate::editor_core::{
    apply_markdown_command, ChangeOrigin, EditorSnapshot, MarkdownCommand, Selection, TextChange,
    Transaction,
};
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos::web_sys::{HtmlElement, Node};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

struct InlineMatch {
    start: usize,
    end: usize,
    inner_start: usize,
    inner_end: usize,
    open_len: usize,
    close_len: usize,
    class: &'static str,
    hide_tokens: bool,
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

fn normalize_pasted_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\u{00A0}', " ")
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

fn highlight_inline(text: &str, caret: Option<usize>) -> String {
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
            },
        );
    }

    for cap in re_md_image.captures_iter(text) {
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
                class: "hl-embed",
                hide_tokens: false,
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
        pos = m.end;
    }
    out.push_str(&escape_html(&text[pos..]));
    out
}

fn highlight_markdown(text: &str, caret: Option<usize>) -> String {
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
        let line_html = highlight_inline(line, caret_rel);

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
        let html = highlight_markdown("[^note]: footnote text\n", None);
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
        let html = highlight_inline(r"\*\*literal\*\* and **bold**", None);
        assert_has(&html, "hl-bold");
        assert_has(&html, r"\*\*literal\*\*");
    }
}

#[component]
pub fn App() -> impl IntoView {
    let (vault_path, set_vault_path) = signal(String::new());
    let (files, set_files) = signal(Vec::<String>::new());
    let (note_texts, set_note_texts) = signal(HashMap::<String, String>::new());
    let (metadata_cache, set_metadata_cache) = signal(MetadataCacheState::default());

    let (current_file, set_current_file) = signal(String::new());
    let (_content, set_content) = signal(String::new());
    let (editor_snapshot, set_editor_snapshot) = signal(EditorSnapshot::new(String::new()));
    let (parsed_html, set_parsed_html) = signal(String::new());
    let (_caret_pos, set_caret_pos) = signal(Option::<usize>::None);
    let editor_ref = NodeRef::<html::Div>::new();
    let (is_composing, set_is_composing) = signal(false);
    let (composition_dirty, set_composition_dirty) = signal(false);

    let (plugin_css, set_plugin_css) = signal(String::new());
    let (settings, set_settings) = signal(AppSettings::default());

    let (save_timeout_id, set_save_timeout_id) = signal(Option::<i32>::None);
    let (save_status, set_save_status) = signal("Saved".to_string());
    let (show_markdown_syntax, set_show_markdown_syntax) = signal(false);
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

    let refresh_vault_snapshot = move |path: String, preferred_file: Option<String>| {
        spawn_local(async move {
            let dir_args = serde_wasm_bindgen::to_value(&ReadDirArgs { path: &path }).unwrap();
            let dir_val = invoke("read_dir", dir_args).await;
            let dir_list =
                serde_wasm_bindgen::from_value::<Vec<String>>(dir_val).unwrap_or_default();
            set_files.set(dir_list.clone());

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
            set_metadata_cache.set(build_metadata_cache(&note_map, &dir_list));

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

            if let Some(path) = next_file {
                let text = note_map.get(&path).cloned().unwrap_or_default();
                set_current_file.set(path);
                set_content.set(text.clone());
                set_parsed_html.set(highlight_markdown(&text, None));
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

    Effect::new(move |_| {
        spawn_local(async move {
            let path_val = invoke("init_vault", JsValue::NULL).await;
            if let Some(path_str) = path_val.as_string() {
                set_vault_path.set(path_str.clone());
                refresh_vault_snapshot(path_str.clone(), None);

                let vault_args = serde_wasm_bindgen::to_value(&VaultPathArgs {
                    vault_path: &path_str,
                })
                .unwrap();

                let css_val = invoke("load_plugins_css", vault_args.clone()).await;
                if let Some(css_str) = css_val.as_string() {
                    set_plugin_css.set(css_str);
                }

                let s_val = invoke("load_settings", vault_args).await;
                if let Some(s_str) = s_val.as_string() {
                    if let Ok(s) = serde_json::from_str::<AppSettings>(&s_str) {
                        set_settings.set(s);
                    }
                }
            }
        });
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
        set_parsed_html.set(highlight_markdown(&final_text, Some(final_selection.start)));

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
            set_current_file.set(filename);
            set_content.set(text.clone());
            set_parsed_html.set(highlight_markdown(&text, None));
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
                set_content.set(text.clone());
                set_parsed_html.set(highlight_markdown(&text, None));
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
                    set_parsed_html.set(highlight_markdown(&text, Some(selection.start)));
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
                    <div class="sidebar-header" style="height: var(--topbar-height); display: flex; align-items: center; justify-content: space-between; padding: 0 1rem; border-bottom: 1px solid var(--border-color); font-weight: 600; color: var(--accent-color);">
                        <span>"Bedrock"</span>
                        <div style="display: flex; gap: 0.5rem; align-items: center;">
                            <button
                                on:click=move |_| create_new_note()
                                style="background: transparent; border: none; font-size: 1.2rem; cursor: pointer; color: var(--text-muted);"
                                title="New note"
                            >
                                "+"
                            </button>
                            <button
                                on:click=move |_| {
                                    spawn_local(async move {
                                        invoke("open_settings_window", JsValue::NULL).await;
                                    });
                                }
                                style="background: transparent; border: none; font-size: 1.2rem; cursor: pointer; color: var(--text-muted);"
                                title="Settings"
                            >
                                "⚙"
                            </button>
                        </div>
                    </div>
                    <div class="file-list" style="flex: 1; overflow-y: auto; padding: 0.75rem 0.5rem;">
                        {move || files.get().into_iter().map(|f| {
                            let filename = f.clone();
                            let f_clone = f.clone();
                            let is_active = move || current_file.get() == f_clone;

                            view! {
                                <div
                                    class="file-item"
                                    style=move || format!("padding: 0.5rem 0.75rem; cursor: pointer; border-radius: var(--radius-md); margin-bottom: 4px; font-size: 0.9rem; transition: background 0.2s, color 0.2s; {}", if is_active() { "background: var(--accent-color); color: white;" } else { "color: var(--text-secondary);" })
                                    on:click=move |_| select_file(filename.clone())
                                >
                                    {f}
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                </nav>
                <section class="editor-pane" style="flex: 1; display: flex; flex-direction: column; background: var(--bg-primary); min-width: 0;">
                    {move || if current_file.get().is_empty() {
                        view! {
                            <div style="flex: 1; display: flex; align-items: center; justify-content: center; color: var(--text-muted);">
                                "Select a note from the sidebar to start editing."
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <header class="topbar" style="height: var(--topbar-height); border-bottom: 1px solid var(--border-color); display: flex; align-items: center; justify-content: space-between; padding: 0 1.5rem; color: var(--text-muted); font-size: 0.9rem; gap: 1rem;">
                                <div style="display: flex; align-items: center; gap: 0.75rem; min-width: 0;">
                                    <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{move || current_file.get()}</span>
                                    <span style="font-size: 0.8rem; color: var(--text-muted);">{move || save_status.get()}</span>
                                </div>
                                <div style="display: flex; gap: 0.5rem;">
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
