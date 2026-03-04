use crate::app_state::{AppSettings, RecentNoteEntry};
use crate::editor_core::{
    apply_markdown_command, ChangeOrigin, EditorSnapshot, MarkdownCommand, Selection, TextChange,
    Transaction,
};
use crate::markdown_syntax::{FileCache, MetadataCacheState};
use crate::metadata_sidebar::MetadataSidebar;
use crate::path_utils::{collapse_path, normalize_rel_path};
use crate::recent_notes_pane::RecentNotesPane;
use crate::sidebar_panel::SidebarPanel;
use crate::sidebar_tree::{expand_parent_folders, SidebarContextMenu};
use crate::editor_pane::EditorPane;
use crate::tauri_bridge;
use crate::top_bar::TopBar;
use js_sys::{Object, Reflect, Date};
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos::web_sys::{Element, Event, HtmlElement, KeyboardEvent, Node};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

// File tree and sidebar helpers now live in `sidebar_tree.rs`.

fn normalize_pasted_text(text: &str) -> String {
    crate::markdown_syntax::normalize_pasted_text(text)
}

fn image_mime_for_path(path: &str) -> &'static str {
    crate::markdown_syntax::image_mime_for_path(path)
}

fn is_supported_inline_image_path(path: &str) -> bool {
    crate::markdown_syntax::image_mime_for_path(path) != "application/octet-stream"
}

fn looks_like_external_url(target: &str) -> bool {
    crate::markdown_syntax::looks_like_external_url(target)
}

fn image_local_candidates(vault_path: &str, note_path: &str, target: &str) -> Vec<String> {
    crate::markdown_syntax::image_local_candidates(vault_path, note_path, target)
}

fn collect_image_targets_for_note(text: &str) -> Vec<(String, bool)> {
    crate::markdown_syntax::collect_image_targets_for_note(text)
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

fn find_ancestor_with_class(node: &Node, class: &str) -> Option<Node> {
    if let Ok(el) = node.clone().dyn_into::<Element>() {
        if el.class_list().contains(class) {
            return Some(node.clone());
        }
    }
    let mut current = node.clone();
    loop {
        let parent = current.parent_node()?;
        if let Ok(el) = parent.clone().dyn_into::<Element>() {
            if el.class_list().contains(class) {
                return Some(parent);
            }
        }
        current = parent;
    }
}

fn offset_before_node(root: &Node, node: &Node) -> Option<usize> {
    let mut text_nodes = Vec::new();
    collect_text_nodes(root, &mut text_nodes);
    let mut sum = 0usize;
    for tn in &text_nodes {
        if node.contains(Some(tn)) {
            return Some(sum);
        }
        sum += tn.node_value().unwrap_or_default().len();
    }
    Some(sum)
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

fn offset_after_previous_sibling(root: &Node, container: &Node, class: &str, text_len: usize) -> Option<usize> {
    let wrap = find_ancestor_with_class(container, class)?;
    let prev = wrap.previous_sibling()?;
    let before = offset_before_node(root, &prev)?;
    Some((before + node_text_len(&prev)).min(text_len))
}

fn range_to_byte_offset(
    root: &Node,
    text_len: usize,
    container: &Node,
    offset: u32,
) -> Option<usize> {
    compute_dom_offset(root, container, offset)
        .or_else(|| offset_after_previous_sibling(root, container, "md-inline-image-wrap", text_len))
        .or_else(|| offset_after_previous_sibling(root, container, "md-embed-line-end", text_len))
}

fn get_selection_byte_offsets_with_collapsed(root: &HtmlElement) -> Option<(Selection, bool)> {
    let win = leptos::web_sys::window()?;
    let dom_selection = win.get_selection().ok().flatten()?;
    if dom_selection.range_count() == 0 {
        return None;
    }
    let root_node: Node = root.clone().unchecked_into();
    let text_len = root_node.text_content().unwrap_or_default().len();
    let text = root_node.text_content().unwrap_or_default();

    let (start, end) = {
        let range_idx = (dom_selection.range_count() - 1).max(0);
        let range = dom_selection.get_range_at(range_idx).ok()?;
        let start_container = range.start_container().ok()?;
        let end_container = range.end_container().ok()?;
        if !root_node.contains(Some(&start_container)) || !root_node.contains(Some(&end_container)) {
            return None;
        }
        let start = range_to_byte_offset(&root_node, text_len, &start_container, range.start_offset().ok()?)?;
        let end = range_to_byte_offset(&root_node, text_len, &end_container, range.end_offset().ok()?)?;
        (start, end)
    };

    let sel = if dom_selection.range_count() > 1 {
        let first_range = dom_selection.get_range_at(0).ok()?;
        let first_start_container = first_range.start_container().ok()?;
        let first_start = range_to_byte_offset(
            &root_node,
            text_len,
            &first_start_container,
            first_range.start_offset().ok()?,
        )?;
        let line_first = line_index_at(&text, first_start);
        let line_last = line_index_at(&text, start);
        let collapse_pos = if line_last > line_first {
            line_start(&text, line_first + 1).min(text_len)
        } else {
            start
        };
        Selection::cursor(collapse_pos)
    } else {
        Selection::new(start, end)
    };

    let did_collapse = dom_selection.range_count() > 1;
    if did_collapse {
        set_selection_byte_offsets(root, sel);
    }
    Some((sel, did_collapse))
}

fn get_selection_byte_offsets(root: &HtmlElement) -> Option<Selection> {
    get_selection_byte_offsets_with_collapsed(root).map(|(s, _)| s)
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

fn line_index_at(text: &str, byte_offset: usize) -> usize {
    let end = byte_offset.min(text.len());
    text[..end].lines().count().saturating_sub(1).max(0)
}

#[allow(dead_code)]
fn line_start(text: &str, line_index: usize) -> usize {
    if line_index == 0 {
        return 0;
    }
    text.match_indices('\n')
        .nth(line_index.saturating_sub(1))
        .map(|(i, _)| i + 1)
        .unwrap_or(text.len())
}
fn highlight_markdown_for_editor(
    text: &str,
    caret: Option<usize>,
    vault_path: &str,
    current_file: &str,
    image_cache: &HashMap<String, String>,
) -> String {
    crate::markdown_syntax::highlight_markdown_for_editor(
        text,
        caret,
        vault_path,
        current_file,
        image_cache,
    )
}

fn extract_file_cache(text: &str) -> FileCache {
    crate::markdown_syntax::extract_file_cache(text)
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
    let mut candidates: Vec<String> = Vec::new();
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
    crate::markdown_syntax::build_metadata_cache(notes, files)
}

#[component]
pub fn App() -> impl IntoView {
    let (vault_path, set_vault_path) = signal(String::new());
    let (open_vaults, set_open_vaults) = signal(Vec::<String>::new());
    let (files, set_files) = signal(Vec::<String>::new());
    let (empty_dirs, set_empty_dirs) = signal(Vec::<String>::new());
    let (note_texts, set_note_texts) = signal(HashMap::<String, String>::new());
    let (note_texts_lower, set_note_texts_lower) = signal(HashMap::<String, String>::new());
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
    let (recent_notes_persist_timeout_id, set_recent_notes_persist_timeout_id) =
        signal(Option::<i32>::None);
    let (save_status, set_save_status) = signal("Saved".to_string());
    let (show_markdown_syntax, set_show_markdown_syntax) = signal(false);
    let (search_query, set_search_query) = signal(String::new());
    let (expanded_folders, set_expanded_folders) = signal(HashSet::<String>::new());
    let (sidebar_context_menu, set_sidebar_context_menu) =
        signal(Option::<SidebarContextMenu>::None);
    let (sidebar_tab, set_sidebar_tab) = signal("search".to_string());
    let (recent_notes, set_recent_notes) = signal(Vec::<RecentNoteEntry>::new());
    let (selection_restore_ticket, set_selection_restore_ticket) = signal(0u64);
    let (selection_sync_ticket, set_selection_sync_ticket) = signal(0u64);
    let (undo_stack, set_undo_stack) = signal(Vec::<(String, Selection)>::new());
    let (redo_stack, set_redo_stack) = signal(Vec::<(String, Selection)>::new());
    let (vault_loaded, set_vault_loaded) = signal(false);
    let (auto_recent_applied, set_auto_recent_applied) = signal(false);
    // Track the last note the user explicitly opened so the Recent pane can
    // still show it even if current_file is momentarily cleared by a race.
    let (last_opened_file, set_last_opened_file) = signal(String::new());
    // Ensure that when the Recent tab is opened after a full restart, we
    // always perform at least one disk read to hydrate the list, even if
    // earlier startup effects failed or were raced out.
    let (recent_tab_disk_bootstrap_done, set_recent_tab_disk_bootstrap_done) =
        signal(false);
    // Track when we last persisted a non-empty recent list into the
    // vault-session file so we can avoid redundant writes while still
    // guaranteeing that at least one non-empty snapshot is available for
    // session-based startup fallback across restarts.
    let (last_persisted_recent_len, set_last_persisted_recent_len) = signal(0usize);

    let closure = Closure::<dyn FnMut(leptos::web_sys::CustomEvent)>::new(
        move |e: leptos::web_sys::CustomEvent| {
            if let Some(detail) = e.detail().as_string() {
                if let Ok(s) = serde_json::from_str::<AppSettings>(&detail) {
                    set_settings.set(s);
                }
            }
        },
    );
    let is_settings_window = window()
        .location()
        .search()
        .unwrap_or_default()
        .contains("settings=true");
    let _ = window()
        .add_event_listener_with_callback("bedrock-settings", closure.as_ref().unchecked_ref());
    closure.forget();

    if !is_settings_window {
        let close_closure = Closure::<dyn FnMut(_)>::new(move |_e: Event| {
            // Capture state synchronously first so index.html can persist even when
            // requestAnimationFrame does not run (e.g. window closing in release webview).
            let win = leptos::web_sys::window().expect("window");
            {
                if let Some(win_ref) = leptos::web_sys::window() {
                    let v = vault_path.get_untracked();
                    let r = recent_notes.get_untracked();
                    let entries: Vec<RecentNoteEntry> = if r.is_empty() && !v.is_empty() {
                        let open = current_file.get_untracked();
                        let fallback = if !open.is_empty() {
                            open
                        } else {
                            last_opened_file.get_untracked()
                        };
                        if !fallback.is_empty() {
                            let title = fallback
                                .rsplit('/')
                                .next()
                                .unwrap_or(&fallback)
                                .to_string();
                            vec![RecentNoteEntry {
                                path: fallback,
                                title,
                                last_opened: Date::now() as i64,
                            }]
                        } else {
                            r
                        }
                    } else {
                        r
                    };
                    let state = Object::new();
                    let _ = Reflect::set(
                        &state,
                        &JsValue::from_str("vault_path"),
                        &JsValue::from_str(&v),
                    );
                    let entries_js =
                        serde_wasm_bindgen::to_value(&entries).unwrap_or(JsValue::NULL);
                    let _ = Reflect::set(&state, &JsValue::from_str("entries"), &entries_js);
                    let current = current_file.get_untracked();
                    if !current.is_empty() {
                        let _ = Reflect::set(
                            &state,
                            &JsValue::from_str("current_file"),
                            &JsValue::from_str(&current),
                        );
                    }
                    let win_js: &JsValue = win_ref.as_ref();
                    let _ = Reflect::set(
                        win_js,
                        &JsValue::from_str("__BEDROCK_STATE__"),
                        &state,
                    );
                    if let Ok(ev) = Event::new("bedrock-state-saved") {
                        let _ = win_ref.dispatch_event(&ev);
                    }
                }
            }
            // Optionally refine state after pending reactive updates (may not run on close).
            let win2 = win.clone();
            let frame_closure = Closure::once(move || {
                let frame_closure2 = Closure::once(move || {
                    if let Some(win_ref) = leptos::web_sys::window() {
                        let v = vault_path.get_untracked();
                        let r = recent_notes.get_untracked();
                        let entries: Vec<RecentNoteEntry> = if r.is_empty() && !v.is_empty() {
                            let open = current_file.get_untracked();
                            let fallback = if !open.is_empty() {
                                open
                            } else {
                                last_opened_file.get_untracked()
                            };
                            if !fallback.is_empty() {
                                let title = fallback
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or(&fallback)
                                    .to_string();
                                vec![RecentNoteEntry {
                                    path: fallback,
                                    title,
                                    last_opened: Date::now() as i64,
                                }]
                            } else {
                                r
                            }
                        } else {
                            r
                        };
                        let state = Object::new();
                        let _ = Reflect::set(
                            &state,
                            &JsValue::from_str("vault_path"),
                            &JsValue::from_str(&v),
                        );
                        let entries_js =
                            serde_wasm_bindgen::to_value(&entries).unwrap_or(JsValue::NULL);
                        let _ = Reflect::set(&state, &JsValue::from_str("entries"), &entries_js);
                        let current = current_file.get_untracked();
                        if !current.is_empty() {
                            let _ = Reflect::set(
                                &state,
                                &JsValue::from_str("current_file"),
                                &JsValue::from_str(&current),
                            );
                        }
                        let win_js: &JsValue = win_ref.as_ref();
                        let _ = Reflect::set(
                            win_js,
                            &JsValue::from_str("__BEDROCK_STATE__"),
                            &state,
                        );
                        if let Ok(ev) = Event::new("bedrock-state-saved") {
                            let _ = win_ref.dispatch_event(&ev);
                        }
                    }
                });
                let _ = win2.request_animation_frame(frame_closure2.as_ref().unchecked_ref());
                frame_closure2.forget();
            });
            let _ = win.request_animation_frame(frame_closure.as_ref().unchecked_ref());
            frame_closure.forget();
        });
        let _ = window().add_event_listener_with_callback(
            "bedrock-save-state-and-close",
            close_closure.as_ref().unchecked_ref(),
        );
        close_closure.forget();
    }

    let clear_active_vault_state = move || {
        set_files.set(Vec::new());
        set_empty_dirs.set(Vec::new());
        set_note_texts.set(HashMap::new());
        set_note_texts_lower.set(HashMap::new());
        set_search_query.set(String::new());
        set_metadata_cache.set(MetadataCacheState::default());
        set_current_file.set(String::new());
        set_last_opened_file.set(String::new());
        set_content.set(String::new());
        set_parsed_html.set(String::new());
        set_caret_pos.set(None);
        set_editor_snapshot.set(EditorSnapshot::new(String::new()));
        set_image_preview_cache.set(HashMap::new());
        set_image_preview_loading.set(HashSet::new());
        set_image_preview_failed.set(HashSet::new());
        set_expanded_folders.set(HashSet::new());
        set_recent_notes.set(Vec::new());
        set_undo_stack.set(Vec::new());
        set_redo_stack.set(Vec::new());
        set_plugin_css.set(String::new());
        set_vault_loaded.set(false);
        set_auto_recent_applied.set(false);
    };

    let push_to_recent_notes = move |path: String| {
        if path.is_empty() {
            return;
        }
        let v_path = vault_path.get_untracked();
        if v_path.is_empty() {
            return;
        }
        set_recent_notes.update(|list| {
            let now = Date::now() as i64;
            let title = path
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .to_string();
            list.retain(|e| e.path != path);
            list.insert(
                0,
                RecentNoteEntry {
                    path: path.clone(),
                    title,
                    last_opened: now,
                },
            );
            if list.len() > 50 {
                list.truncate(50);
            }
        });
        let list = recent_notes.get_untracked();
        let v = v_path.clone();
        // Immediate persist so recent list survives quick close or crash.
        spawn_local(async move {
            tauri_bridge::cache_recent_notes(&v, &list).await;
        });
        // Second persist after 150ms so we retry if the first invoke failed or was too late.
        if let Some(win) = leptos::web_sys::window() {
            let v2 = v_path.clone();
            let recent_for_delay = recent_notes.clone();
            let cb = Closure::once(move || {
                let list2 = recent_for_delay.get_untracked();
                if list2.is_empty() {
                    return;
                }
                spawn_local(async move {
                    tauri_bridge::cache_recent_notes(&v2, &list2).await;
                });
            });
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                150,
            );
            cb.forget();
        }
    };

    let refresh_vault_snapshot = move |path: String, preferred_file: Option<String>| {
        spawn_local(async move {
            set_image_preview_cache.set(HashMap::new());
            set_image_preview_loading.set(HashSet::new());
            set_image_preview_failed.set(HashSet::new());

            let dir_result = tauri_bridge::read_dir(&path).await;
            let current_vault = vault_path.get_untracked();
            if collapse_path(&path) != collapse_path(&current_vault) {
                return;
            }
            set_vault_loaded.set(true);
            set_files.set(dir_result.notes.clone());
            set_empty_dirs.set(dir_result.empty_dirs.clone());

            let mut loaded_recent = if collapse_path(&path) == collapse_path(&vault_path.get_untracked()) {
                let mut l = tauri_bridge::read_recent_notes(&path).await;
                l.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
                // Prefer in-memory recent_notes when non-empty so a refresh (e.g. focus, watcher)
                // doesn't overwrite with stale disk data before persist has completed.
                let in_memory = recent_notes.get_untracked();
                if !in_memory.is_empty() {
                    let mut merged = in_memory.clone();
                    merged.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
                    set_recent_notes.set(merged.clone());
                    merged
                } else {
                    // Re-read in-memory right before setting: user may have opened a note
                    // after we read disk, so we must not overwrite with stale empty list.
                    let now_memory = recent_notes.get_untracked();
                    if !now_memory.is_empty() {
                        let mut merged = now_memory.clone();
                        merged.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
                        set_recent_notes.set(merged.clone());
                        merged
                    } else {
                        // Never show "No recent notes" when user has a file open: if disk is empty
                        // but current_file is set and in this vault's files, ensure it's in the list.
                        let mut list_to_set = l.clone();
                        if list_to_set.is_empty() {
                            let open_file = current_file.get_untracked();
                            if !open_file.is_empty() && dir_result.notes.contains(&open_file) {
                                let title = open_file
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or(&open_file)
                                    .to_string();
                                list_to_set.push(RecentNoteEntry {
                                    path: open_file.clone(),
                                    title,
                                    last_opened: Date::now() as i64,
                                });
                            }
                        }
                        // Do not overwrite if user opened a note after we read now_memory (race).
                        let right_before_set = recent_notes.get_untracked();
                        if !right_before_set.is_empty() {
                            let mut merged = right_before_set.clone();
                            merged.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
                            set_recent_notes.set(merged.clone());
                            merged
                        } else {
                            set_recent_notes.set(list_to_set.clone());
                            list_to_set
                        }
                    }
                }
            } else {
                Vec::new()
            };

            // Normalize recent note paths so they always match the current vault's note list.
            // This handles older recent.json files that may store absolute paths instead of
            // the relative paths used by collect_note_paths/read_dir.
            if collapse_path(&path) == collapse_path(&vault_path.get_untracked()) && !loaded_recent.is_empty() {
                let files_in_vault = dir_result.notes.clone();
                let vault_prefix = format!("{}/", path);
                let mut normalized: Vec<RecentNoteEntry> = Vec::new();

                for mut entry in loaded_recent.iter().cloned() {
                    if files_in_vault.contains(&entry.path) {
                        normalized.push(entry);
                        continue;
                    }

                    if entry.path.starts_with(&vault_prefix) {
                        let rel = entry.path[vault_prefix.len()..].to_string();
                        if files_in_vault.contains(&rel) {
                            entry.path = rel;
                            normalized.push(entry);
                            continue;
                        }
                    }
                }

                if !normalized.is_empty() {
                    set_recent_notes.set(normalized.clone());
                    loaded_recent = normalized;
                }
                // If normalized is empty we keep loaded_recent as-is; UI already has it from set_recent_notes.set(l) above.
            }

            let notes_list = tauri_bridge::read_vault_notes(&path).await;

            let mut note_map = HashMap::new();
            let mut lower_map = HashMap::new();
            for note in notes_list {
                let path = note.path;
                let content = note.content;
                note_map.insert(path.clone(), content.clone());
                lower_map.insert(path, content.to_lowercase());
            }

            set_note_texts.set(note_map.clone());
            set_note_texts_lower.set(lower_map);
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
                });

            if let Some(selected_file) = next_file {
                let text = note_map.get(&selected_file).cloned().unwrap_or_default();
                set_current_file.set(selected_file.clone());
                set_recent_notes.update(|list| {
                    let now = Date::now() as i64;
                    let title = selected_file
                        .rsplit('/')
                        .next()
                        .unwrap_or(&selected_file)
                        .to_string();
                    list.retain(|e| e.path != selected_file);
                    list.insert(
                        0,
                        RecentNoteEntry {
                            path: selected_file.clone(),
                            title,
                            last_opened: now,
                        },
                    );
                    if list.len() > 50 {
                        list.truncate(50);
                    }
                });
                let now = Date::now() as i64;
                let title = selected_file
                    .rsplit('/')
                    .next()
                    .unwrap_or(&selected_file)
                    .to_string();
                loaded_recent.retain(|e| e.path != selected_file);
                loaded_recent.insert(
                    0,
                    RecentNoteEntry {
                        path: selected_file.clone(),
                        title,
                        last_opened: now,
                    },
                );
                if loaded_recent.len() > 50 {
                    loaded_recent.truncate(50);
                }
                set_undo_stack.set(Vec::new());
                set_redo_stack.set(Vec::new());
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
                // Only clear current_file if it's no longer in the vault (e.g. deleted).
                // If it's still in dir_list, keep it so the Recent pane and fallbacks still show it
                // (avoids race where refresh completes after user opened a note but computed next_file earlier).
                let open = current_file.get_untracked();
                if open.is_empty() || !dir_list.contains(&open) {
                    set_current_file.set(String::new());
                }
                set_content.set(String::new());
                set_parsed_html.set(String::new());
                set_caret_pos.set(None);
                set_editor_snapshot.set(EditorSnapshot::new(String::new()));
            }
            if path == vault_path.get_untracked() {
                // Final safeguard: ensure current_file is in recent list so we never persist
                // a list that would show "No recent notes." when a note is open (e.g. race
                // where refresh overwrote the optimistic push).
                let open_file = current_file.get_untracked();
                if !open_file.is_empty() && dir_list.contains(&open_file) {
                    let in_memory_now = recent_notes.get_untracked();
                    let has_open = in_memory_now.iter().any(|e| e.path == open_file);
                    if !has_open {
                        let title = open_file
                            .rsplit('/')
                            .next()
                            .unwrap_or(&open_file)
                            .to_string();
                        set_recent_notes.update(|list| {
                            list.retain(|e| e.path != open_file);
                            list.insert(
                                0,
                                RecentNoteEntry {
                                    path: open_file.clone(),
                                    title: title.clone(),
                                    last_opened: Date::now() as i64,
                                },
                            );
                            if list.len() > 50 {
                                list.truncate(50);
                            }
                        });
                        let mut fixed = loaded_recent.clone();
                        fixed.retain(|e| e.path != open_file);
                        fixed.insert(
                            0,
                            RecentNoteEntry {
                                path: open_file.clone(),
                                title,
                                last_opened: Date::now() as i64,
                            },
                        );
                        if fixed.len() > 50 {
                            fixed.truncate(50);
                        }
                        loaded_recent = fixed;
                    }
                }
                tauri_bridge::cache_recent_notes(&path, &loaded_recent).await;
            }
        });
    };

    let load_vault_visual_state = move |path: String| {
        if path.is_empty() {
            set_plugin_css.set(String::new());
            return;
        }
        spawn_local(async move {
            if let Some(css_str) = tauri_bridge::load_plugins_css(&path).await {
                set_plugin_css.set(css_str);
            } else {
                set_plugin_css.set(String::new());
            }

            if let Some(s_str) = tauri_bridge::load_settings(&path).await {
                if let Ok(s) = serde_json::from_str::<AppSettings>(&s_str) {
                    set_settings.set(s);
                }
            }
        });
    };

    let persist_vault_session = move |open_list: Vec<String>, active: Option<String>| {
        let set_open = set_open_vaults;
        let set_active = set_vault_path;
        // Pass current recent_notes for the active vault so session file persists the list.
        let recent_for_active = active.as_ref().and_then(|a| {
            if collapse_path(a) == collapse_path(&vault_path.get_untracked()) {
                let r = recent_notes.get_untracked();
                if r.is_empty() {
                    None
                } else {
                    Some(r)
                }
            } else {
                None
            }
        });
        spawn_local(async move {
            let normalized = tauri_bridge::save_vault_session(
                &open_list,
                active.as_deref(),
                recent_for_active.as_deref(),
            )
            .await;
            set_open.set(normalized.open_vaults.clone());
            if let Some(active_path) = normalized.active_vault {
                set_active.set(active_path);
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
        set_vault_loaded.set(false);
        persist_vault_session(open_now, Some(path.clone()));
        refresh_vault_snapshot(path.clone(), preferred_file);
        load_vault_visual_state(path);
    };

    Effect::new(move |_| {
        spawn_local(async move {
            let default_path = tauri_bridge::init_vault().await;

            let mut session = tauri_bridge::load_vault_session().await;

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
                // If the session carried a recent-notes snapshot for this vault, restore
                // it immediately so the Recent notes tab is populated on startup even
                // before any async refresh completes.
                if !session.recent_notes.is_empty() {
                    let active_key = collapse_path(&path);
                    // Try exact path key first, then collapse_path match (backend uses canonical paths).
                    let list = session
                        .recent_notes
                        .get(&path)
                        .cloned()
                        .or_else(|| {
                            session
                                .recent_notes
                                .iter()
                                .find(|(k, _)| collapse_path(k) == active_key)
                                .map(|(_, v)| v.clone())
                        });
                    if let Some(list) = list {
                        if !list.is_empty() {
                            set_recent_notes.set(list);
                        }
                    }
                }
                activate_vault(path.clone(), None);
                // Always load recent notes from disk for the active vault at startup so
                // persistence across restarts is guaranteed (disk is source of truth after launch).
                let path_for_fallback = path.clone();
                let set_recent_notes_fallback = set_recent_notes.clone();
                spawn_local(async move {
                    let list = tauri_bridge::read_recent_notes(&path_for_fallback).await;
                    if !list.is_empty() {
                        set_recent_notes_fallback.set(list);
                    }
                });
                // Delayed fallback: if a late refresh overwrote the list with empty, re-apply from disk.
                if let Some(win) = leptos::web_sys::window() {
                    let path_delayed = path.clone();
                    let set_delayed = set_recent_notes.clone();
                    let recent_check = recent_notes.clone();
                    let cb = Closure::once(move || {
                        spawn_local(async move {
                            let current = recent_check.get_untracked();
                            if !current.is_empty() {
                                return;
                            }
                            let list = tauri_bridge::read_recent_notes(&path_delayed).await;
                            if !list.is_empty() {
                                set_delayed.set(list);
                            }
                        });
                    });
                    let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                        cb.as_ref().unchecked_ref(),
                        400,
                    );
                    cb.forget();
                    // Longer delayed fallback: if refresh runs after 400ms and overwrote with empty,
                    // restore from disk again so persistence is visible after full restart.
                    let path_delayed2 = path.clone();
                    let set_delayed2 = set_recent_notes.clone();
                    let recent_check2 = recent_notes.clone();
                    let cb2 = Closure::once(move || {
                        spawn_local(async move {
                            let current = recent_check2.get_untracked();
                            if !current.is_empty() {
                                return;
                            }
                            let list = tauri_bridge::read_recent_notes(&path_delayed2).await;
                            if !list.is_empty() {
                                set_delayed2.set(list);
                            }
                        });
                    });
                    let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                        cb2.as_ref().unchecked_ref(),
                        1200,
                    );
                    cb2.forget();
                    // Third fallback at 2500ms so persistence is visible even if refresh/race cleared the list late.
                    let path_delayed3 = path.clone();
                    let set_delayed3 = set_recent_notes.clone();
                    let recent_check3 = recent_notes.clone();
                    let cb3 = Closure::once(move || {
                        spawn_local(async move {
                            let current = recent_check3.get_untracked();
                            if !current.is_empty() {
                                return;
                            }
                            let list = tauri_bridge::read_recent_notes(&path_delayed3).await;
                            if !list.is_empty() {
                                set_delayed3.set(list);
                            }
                        });
                    });
                    let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                        cb3.as_ref().unchecked_ref(),
                        2500,
                    );
                    cb3.forget();
                }
            } else {
                set_vault_path.set(String::new());
                clear_active_vault_state();
                set_plugin_css.set(String::new());
            }
            persist_vault_session(session.open_vaults.clone(), active.clone());
        });
    });

    Effect::new(move |_| {
        if auto_recent_applied.get() {
            return;
        }
        if vault_path.get().is_empty() || !vault_loaded.get() {
            return;
        }
        if recent_notes.get().is_empty() {
            return;
        }
        set_sidebar_tab.set("recent".to_string());
        set_auto_recent_applied.set(true);
    });

    // Whenever the active vault has a non-empty recent list, persist it into
    // the vault-session file so that load_vault_session can restore Recent
    // notes across process restarts even if disk writes for `.bedrock/recent.json`
    // were delayed or failed. We only persist when the list length changes to
    // avoid tight feedback loops when save_vault_session normalizes state.
    Effect::new(move |_| {
        let v = vault_path.get();
        let open_list = open_vaults.get();
        let recents = recent_notes.get();
        if v.is_empty() || open_list.is_empty() || recents.is_empty() {
            return;
        }
        let current_len = recents.len();
        if current_len == last_persisted_recent_len.get() {
            return;
        }
        set_last_persisted_recent_len.set(current_len);
        persist_vault_session(open_list.clone(), Some(v.clone()));
    });

    // When the user explicitly opens the Recent tab after a full restart,
    // make a last-chance disk read to hydrate the list if it is still empty.
    Effect::new(move |_| {
        if recent_tab_disk_bootstrap_done.get() {
            return;
        }
        if sidebar_tab.get() != "recent" {
            return;
        }
        let v = vault_path.get();
        if v.is_empty() {
            return;
        }
        if !recent_notes.get().is_empty() {
            set_recent_tab_disk_bootstrap_done.set(true);
            return;
        }
        let path = v.clone();
        let set_recent = set_recent_notes.clone();
        set_recent_tab_disk_bootstrap_done.set(true);
        spawn_local(async move {
            let list = tauri_bridge::read_recent_notes(&path).await;
            if !list.is_empty() {
                set_recent.set(list);
            }
        });
    });

    Effect::new(move |_| {
        let v = vault_path.get();
        let mut r = recent_notes.get();
        // When closing with an open note but an empty recent list, persist a
        // single entry for the open note so the next launch can still show it
        // in the Recent notes tab.
        if r.is_empty() && !v.is_empty() {
            let open = current_file.get_untracked();
            if !open.is_empty() {
                let title = open
                    .rsplit('/')
                    .next()
                    .unwrap_or(&open)
                    .to_string();
                r.push(RecentNoteEntry {
                    path: open.clone(),
                    title,
                    last_opened: Date::now() as i64,
                });
            }
        }
        if let Some(win) = leptos::web_sys::window() {
            let state = Object::new();
            let _ = Reflect::set(
                &state,
                &JsValue::from_str("vault_path"),
                &JsValue::from_str(&v),
            );
            let entries_js = serde_wasm_bindgen::to_value(&r).unwrap_or(JsValue::NULL);
            let _ = Reflect::set(&state, &JsValue::from_str("entries"), &entries_js);
            let win_js: &JsValue = win.as_ref();
            let _ = Reflect::set(win_js, &JsValue::from_str("__BEDROCK_STATE__"), &state);
        }
    });

    Effect::new(move |_| {
        let v = vault_path.get();
        let r = recent_notes.get();
        if v.is_empty() || r.is_empty() {
            return;
        }
        if let Some(win) = leptos::web_sys::window() {
            if let Some(prev) = recent_notes_persist_timeout_id.get_untracked() {
                win.clear_timeout_with_handle(prev);
            }
            let v_clone = v.clone();
            let r_clone = r.clone();
            let set_timeout = set_recent_notes_persist_timeout_id;
            let cb = Closure::once(move || {
                set_timeout.set(None);
                spawn_local(async move {
                    tauri_bridge::cache_recent_notes(&v_clone, &r_clone).await;
                });
            });
            if let Ok(id) = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                100,
            ) {
                set_recent_notes_persist_timeout_id.set(Some(id));
                cb.forget();
            }
        }
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
                if let Some(encoded) = tauri_bridge::read_file_base64(&image_path).await {
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
                    tauri_bridge::write_file(&file_path, &new_text).await;
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
            let cb2 = Closure::once(move || {
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
                let _ = win.request_animation_frame(cb2.as_ref().unchecked_ref());
                cb2.forget();
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

    let apply_editor_update = move |new_text: String, sel_start: usize, sel_end: usize, skip_undo: bool| {
        let mut snapshot = editor_snapshot.get_untracked();
        if !skip_undo && snapshot.text != new_text {
            set_undo_stack.update(|u| {
                u.push((snapshot.text.clone(), snapshot.selection));
                if u.len() > 100 {
                    u.remove(0);
                }
            });
            set_redo_stack.set(Vec::new());
        }
        set_composition_dirty.set(false);
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
            let mut lower_notes = note_texts_lower.get_untracked();
            lower_notes.insert(file.clone(), final_text.to_lowercase());
            let cache = build_metadata_cache(&notes, &files.get_untracked());
            set_note_texts.set(notes);
            set_note_texts_lower.set(lower_notes);
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

    let schedule_focus_editor = move || {
        let eref = editor_ref.clone();
        if let Some(win) = leptos::web_sys::window() {
            let cb = Closure::once(move || {
                if let Some(el) = eref.get() {
                    if let Ok(he) = el.dyn_into::<HtmlElement>() {
                        let _ = he.focus();
                    }
                }
            });
            let _ = win.request_animation_frame(cb.as_ref().unchecked_ref());
            cb.forget();
        }
    };

    let select_file = move |filename: String| {
        if let Some(text) = note_texts.get_untracked().get(&filename).cloned() {
            set_current_file.set(filename.clone());
            set_last_opened_file.set(filename.clone());
            push_to_recent_notes(filename.clone());
            set_undo_stack.set(Vec::new());
            set_redo_stack.set(Vec::new());
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
            schedule_focus_editor();
            return;
        }

        let v_path = vault_path.get_untracked();
        if v_path.is_empty() {
            return;
        }
        // Update Recent notes and current_file immediately so the list and fallbacks
        // see the open note before async read completes and before any concurrent refresh.
        set_current_file.set(filename.clone());
        set_last_opened_file.set(filename.clone());
        push_to_recent_notes(filename.clone());
        spawn_local(async move {
            let file_path = format!("{}/{}", v_path, filename);
            if let Some(text) = tauri_bridge::read_file(&file_path).await {
                set_current_file.set(filename.clone());
                push_to_recent_notes(filename.clone());
                set_undo_stack.set(Vec::new());
                set_redo_stack.set(Vec::new());
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
                notes.insert(filename.clone(), text.clone());
                let mut lower_notes = note_texts_lower.get_untracked();
                lower_notes.insert(filename.clone(), text.to_lowercase());
                set_metadata_cache.set(build_metadata_cache(&notes, &files.get_untracked()));
                set_note_texts.set(notes);
                set_note_texts_lower.set(lower_notes);

                let eref = editor_ref.clone();
                if let Some(win) = leptos::web_sys::window() {
                    let cb = Closure::once(move || {
                        if let Some(el) = eref.get() {
                            if let Ok(he) = el.dyn_into::<HtmlElement>() {
                                let _ = he.focus();
                            }
                        }
                    });
                    let _ = win.request_animation_frame(cb.as_ref().unchecked_ref());
                    cb.forget();
                }
            }
        });
    };

    Effect::new(move |_| {
        let Some(win) = leptos::web_sys::window() else { return };
        let Some(doc) = win.document() else { return };
        let handler = Closure::wrap(Box::new(move |e: KeyboardEvent| {
            if (e.meta_key() || e.ctrl_key()) && e.key() == "1" {
                e.prevent_default();
                let list = files.get_untracked();
                if let Some(first) = list.first() {
                    select_file(first.clone());
                }
            }
        }) as Box<dyn FnMut(KeyboardEvent)>);
        let _ = doc.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
        handler.forget();
    });

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
            let text = root.text_content().unwrap_or_default();
            if let Some(selection) = get_selection_byte_offsets(&root).map(|s| s.clamp(text.len()))
            {
                let mut snapshot = editor_snapshot.get_untracked();
                if snapshot.selection != selection {
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
            .and_then(|el| el.text_content())
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
            apply_editor_update(new_text, selection.start, selection.end, false);
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

        let text = root.text_content().unwrap_or_default();
        let (selection, did_collapse) = get_selection_byte_offsets_with_collapsed(&root)
            .map(|(s, c)| (s.clamp(text.len()), c))
            .unwrap_or_else(|| (Selection::cursor(text.len()), false));

        let mut snapshot = editor_snapshot.get_untracked();
        snapshot.replace_from_input(text.clone(), selection);
        set_editor_snapshot.set(snapshot.clone());

        let key = e.key();
        if did_collapse && (key == "ArrowUp" || key == "ArrowDown") {
            return;
        }

        let ctrl_or_cmd = e.ctrl_key() || e.meta_key();

        if ctrl_or_cmd && !e.alt_key() {
            match key.as_str() {
                "z" | "Z" => {
                    if e.shift_key() {
                        if let Some((text, sel)) = redo_stack.get_untracked().last().cloned() {
                            set_redo_stack.update(|r| { r.pop(); });
                            set_undo_stack.update(|u| {
                                u.push((snapshot.text.clone(), snapshot.selection));
                                if u.len() > 100 { u.remove(0); }
                            });
                            apply_editor_update(text, sel.start, sel.end, true);
                            e.prevent_default();
                            return;
                        }
                    } else {
                        if let Some((text, sel)) = undo_stack.get_untracked().last().cloned() {
                            set_undo_stack.update(|u| { u.pop(); });
                            set_redo_stack.update(|r| {
                                r.push((snapshot.text.clone(), snapshot.selection));
                                if r.len() > 100 { r.remove(0); }
                            });
                            apply_editor_update(text, sel.start, sel.end, true);
                            e.prevent_default();
                            return;
                        }
                    }
                }
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
                            false,
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
                            false,
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
                            false,
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
                    false,
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
                    false,
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
                    false,
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
                        false,
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

        let text = root.text_content().unwrap_or_default();
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
                false,
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
        let text = root.text_content().unwrap_or_default();
        let selection = get_selection_byte_offsets(&root)
            .unwrap_or_else(|| Selection::cursor(text.len()))
            .clamp(text.len());
        set_composition_dirty.set(false);
        apply_editor_update(text, selection.start, selection.end, false);
    };

    let run_editor_action = move |action: &'static str| {
        let Some(el) = editor_ref.get() else {
            return;
        };
        let Ok(root) = el.dyn_into::<HtmlElement>() else {
            return;
        };

        let text = root.text_content().unwrap_or_default();
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
                false,
            );
        }
    };

    let save_settings_to_disk = move |s: AppSettings| {
        let v_path = vault_path.get_untracked();
        if v_path.is_empty() {
            return;
        }
        spawn_local(async move {
            tauri_bridge::save_settings(&v_path, &s).await;
        });
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
                tauri_bridge::write_file(&file_path, &initial).await;
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
                tauri_bridge::write_file(&file_path, &initial).await;
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
                tauri_bridge::create_dir(&full_path).await;
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
            tauri_bridge::delete_file(&full_path).await;
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
            tauri_bridge::delete_dir(&full_path).await;
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
                let selected =
                    tauri_bridge::rename_note(&v_path, &old_for_api, &next_for_api).await;
                refresh_vault_snapshot(path_for_refresh, Some(selected));
            });
        }
    };

    let import_from_obsidian_vault = move || {
        spawn_local(async move {
            let Some(report) = tauri_bridge::import_obsidian_vault_with_picker().await else {
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
            let Some(path) = tauri_bridge::pick_bedrock_vault().await else {
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
                <SidebarPanel
                    vault_path=vault_path
                    open_vaults=open_vaults
                    sidebar_tab=sidebar_tab
                    set_sidebar_tab=set_sidebar_tab
                    files=files
                    empty_dirs=empty_dirs
                    expanded_folders=expanded_folders
                    set_expanded_folders=set_expanded_folders
                    sidebar_context_menu=sidebar_context_menu
                    set_sidebar_context_menu=set_sidebar_context_menu
                    current_file=current_file
                    last_opened_file=last_opened_file
                    recent_notes=recent_notes
                    note_texts=note_texts
                    note_texts_lower=note_texts_lower
                    search_query=search_query
                    set_search_query=set_search_query
                    on_open_vault=move || open_bedrock_vault()
                    on_close_vault=move || close_current_vault()
                    on_new_note=move || create_new_note()
                    on_import_obsidian=move || import_from_obsidian_vault()
                    on_open_settings=move || {
                        spawn_local(async move {
                            tauri_bridge::open_settings_window().await;
                        });
                    }
                    on_switch_vault=move |value| switch_to_vault(value)
                    on_select_file=move |filename| select_file(filename)
                    create_note_in_folder=move |path| create_note_in_folder(path)
                    create_folder_in_folder=move |path| create_folder_in_folder(path)
                    delete_folder=move |path| delete_folder(path)
                    delete_note=move |path| delete_note(path)
                />
                <EditorPane>
                    {move || if current_file.get().is_empty() {
                        view! {
                            <RecentNotesPane
                                vault_path=vault_path
                                recent_notes=recent_notes
                                files=files
                                on_select_file=move |filename| select_file(filename)
                            />
                        }
                        .into_any()
                    } else {
                        view! {
                            <TopBar
                                current_file=current_file
                                vault_path=vault_path
                                save_status=save_status
                                on_open_vault=move || open_bedrock_vault()
                                on_import_obsidian=move || import_from_obsidian_vault()
                                on_rename=move || rename_current_note()
                            />
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
                                    style="width: 100%; height: 100%; padding: 2rem 3rem; font-family: var(--font-editor); font-size: var(--editor-font-size); line-height: 1.6; color: var(--text-primary); white-space: pre-wrap; word-wrap: break-word; box-sizing: border-box; overflow-x: auto; overflow-y: auto; outline: none; caret-color: var(--text-primary);"
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
                                        let k = e.key();
                                        if is_selection_navigation_key(&k) {
                                            schedule_selection_sync();
                                        }
                                    }
                                    on:mouseup=move |_| schedule_selection_sync()
                                    on:focus=move |_| schedule_selection_sync()
                                ></div>
                            </div>
                        }
                        .into_any()
                    }}
                </EditorPane>
                <MetadataSidebar
                    files=files
                    current_file=current_file
                    metadata_cache=metadata_cache
                />
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
