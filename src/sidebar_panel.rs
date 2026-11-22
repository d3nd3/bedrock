use crate::app_state::{NoteColorPaletteEntry, RecentNoteEntry};
use crate::path_utils::{canonical_note_color_key, resolve_note_color};
use crate::sidebar_tree::{
    add_empty_dirs_to_tree, build_file_tree, build_sidebar_entries, SidebarContextMenu,
    SidebarEntry,
};
use crate::vault_tabs::VaultTabs;
use js_sys::Date;
use leptos::prelude::*;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use wasm_bindgen::JsValue;

fn fuzzy_token_match(haystack: &str, token: &str) -> (bool, bool) {
    if token.is_empty() {
        return (true, false);
    }
    if haystack.contains(token) {
        return (true, false);
    }

    let mut boundaries = Vec::new();
    boundaries.push(0usize);
    for (idx, _) in token.char_indices() {
        if idx != 0 {
            boundaries.push(idx);
        }
    }
    if boundaries.len() < 4 {
        return (false, false);
    }
    boundaries.push(token.len());

    let last = boundaries.len() - 1;
    for i in 0..last {
        let start_a = boundaries[0];
        let end_a = boundaries[i];
        let start_b = boundaries[i + 1];
        let end_b = boundaries[last];

        let mut shortened = String::new();
        shortened.push_str(&token[start_a..end_a]);
        shortened.push_str(&token[start_b..end_b]);

        if haystack.contains(&shortened) {
            return (true, true);
        }
    }

    (false, false)
}

const DEFAULT_LABEL_COLORS: &[(&str, &str)] = &[
    ("Orange", "#f97316"),
    ("Green", "#22c55e"),
    ("Blue", "#3b82f6"),
    ("Purple", "#a855f7"),
];

#[component]
pub fn SidebarPanel<
    FOpen,
    FClose,
    FNew,
    FImport,
    FSettings,
    FSwitch,
    FCooldown,
    FSelect,
    FCreateNoteInFolder,
    FOpenInNewPane,
    FCreateFolderInFolder,
    FDeleteFolder,
    FDeleteNote,
    FSetColor,
>(
    vault_path: ReadSignal<String>,
    open_vaults: ReadSignal<Vec<String>>,
    sidebar_tab: ReadSignal<String>,
    set_sidebar_tab: WriteSignal<String>,
    files: ReadSignal<Vec<String>>,
    empty_dirs: ReadSignal<Vec<String>>,
    expanded_folders: ReadSignal<HashSet<String>>,
    set_expanded_folders: WriteSignal<HashSet<String>>,
    sidebar_context_menu: ReadSignal<Option<SidebarContextMenu>>,
    set_sidebar_context_menu: WriteSignal<Option<SidebarContextMenu>>,
    current_file: ReadSignal<String>,
    last_opened_file: ReadSignal<String>,
    recent_notes: ReadSignal<Vec<RecentNoteEntry>>,
    note_texts: ReadSignal<HashMap<String, String>>,
    note_texts_lower: ReadSignal<HashMap<String, String>>,
    note_colors: ReadSignal<HashMap<String, String>>,
    note_color_palette: ReadSignal<Vec<NoteColorPaletteEntry>>,
    note_colors_hydrated: ReadSignal<bool>,
    note_colors_epoch: ReadSignal<u32>,
    search_query: ReadSignal<String>,
    set_search_query: WriteSignal<String>,
    on_open_vault: FOpen,
    on_close_vault: FClose,
    on_new_note: FNew,
    on_import_obsidian: FImport,
    on_open_settings: FSettings,
    on_switch_vault: FSwitch,
    on_vault_switch_gesture: FCooldown,
    on_select_file: FSelect,
    create_note_in_folder: FCreateNoteInFolder,
    create_folder_in_folder: FCreateFolderInFolder,
    delete_folder: FDeleteFolder,
    delete_note: FDeleteNote,
    on_set_note_color: FSetColor,
    #[prop(optional)]
    focus_search_trigger: Option<ReadSignal<u32>>,
    #[prop(optional)]
    on_open_in_new_pane: Option<FOpenInNewPane>,
) -> impl IntoView
where
    FOpen: Fn() + 'static + Clone,
    FClose: Fn() + 'static + Clone,
    FNew: Fn() + 'static + Clone,
    FImport: Fn() + 'static + Clone,
    FSettings: Fn() + 'static + Clone,
    FSwitch: Fn(String) + 'static + Clone + Send,
    FCooldown: Fn() + 'static + Clone + Send,
    FSelect: Fn(String) + 'static + Clone + Send,
    FCreateNoteInFolder: Fn(String) + 'static + Clone + Send,
    FOpenInNewPane: Fn(String) + 'static + Clone + Send,
    FCreateFolderInFolder: Fn(String) + 'static + Clone + Send,
    FDeleteFolder: Fn(String) + 'static + Clone + Send,
    FDeleteNote: Fn(String) + 'static + Clone + Send,
    FSetColor: Fn(String, Option<String>) + 'static + Clone + Send,
{
    let open_vaults_signal = open_vaults;
    let vault_path_signal = vault_path;
    let sidebar_tab_signal = sidebar_tab;
    let set_sidebar_tab_signal = set_sidebar_tab;
    let files_signal = files;
    let empty_dirs_signal = empty_dirs;
    let expanded_folders_read = expanded_folders;
    let set_expanded_folders_signal = set_expanded_folders;
    let sidebar_context_menu_read = sidebar_context_menu;
    let set_sidebar_context_menu_signal = set_sidebar_context_menu;
    let current_file_signal = current_file;
    let last_opened_file_signal = last_opened_file;
    let recent_notes_signal = recent_notes;
    let note_texts_signal = note_texts;
    let note_texts_lower_signal = note_texts_lower;
    let note_colors_signal = note_colors;
    let note_color_palette_signal = note_color_palette;
    let note_colors_hydrated_signal = note_colors_hydrated;
    let note_colors_epoch_signal = note_colors_epoch;
    let search_query_signal = search_query;
    let set_search_query_signal = set_search_query;
    let search_input_ref = NodeRef::<leptos::html::Input>::new();

    if let Some(trigger) = focus_search_trigger {
        let search_input_ref = search_input_ref.clone();
        Effect::new(move |_| {
            let _ = trigger.get();
            if let Some(el) = search_input_ref.get() {
                let _ = el.focus();
            }
        });
    }

    view! {
        <nav class="sidebar" style="width: var(--sidebar-width); min-width: var(--sidebar-width); border-right: 1px solid var(--border-color); display: flex; flex-direction: column; background: var(--bg-secondary); transition: all 0.3s ease; overflow: hidden; box-sizing: border-box;">
            <div style="flex-shrink: 0; min-width: 0; overflow: hidden; width: 100%; max-width: 100%; box-sizing: border-box;">
            <VaultTabs
                vault_path=vault_path_signal
                open_vaults=open_vaults_signal
                on_open_vault=on_open_vault
                on_close_vault=on_close_vault
                on_new_note=on_new_note
                on_import_obsidian=on_import_obsidian
                on_open_settings=on_open_settings
                on_switch_vault=on_switch_vault
                on_vault_switch_gesture=on_vault_switch_gesture
            />
            </div>
            <div class="sidebar-tab-row" style="border-bottom: 1px solid var(--border-color); padding: 0 0.5rem; flex-shrink: 0;">
                <button
                    data-sidebar-tab="search"
                    style=move || format!(
                        "padding: 0.4rem 0.5rem; font-size: 0.8rem; border: none; border-radius: 0; background: transparent; color: {}; border-bottom: 2px solid {};",
                        if sidebar_tab_signal.get() == "search" { "var(--accent-color)" } else { "var(--text-muted)" },
                        if sidebar_tab_signal.get() == "search" { "var(--accent-color)" } else { "transparent" }
                    )
                    on:click=move |_| set_sidebar_tab_signal.set("search".to_string())
                >
                    "Search"
                </button>
                <button
                    data-sidebar-tab="files"
                    style=move || format!(
                        "padding: 0.4rem 0.5rem; font-size: 0.8rem; border: none; border-radius: 0; background: transparent; color: {}; border-bottom: 2px solid {};",
                        if sidebar_tab_signal.get() == "files" { "var(--accent-color)" } else { "var(--text-muted)" },
                        if sidebar_tab_signal.get() == "files" { "var(--accent-color)" } else { "transparent" }
                    )
                    on:click=move |_| set_sidebar_tab_signal.set("files".to_string())
                >
                    "Files"
                </button>
                <button
                    data-sidebar-tab="recent"
                    title="Recent notes"
                    style=move || format!(
                        "padding: 0.4rem 0.5rem; font-size: 0.8rem; border: none; border-radius: 0; background: transparent; color: {}; border-bottom: 2px solid {};",
                        if sidebar_tab_signal.get() == "recent" { "var(--accent-color)" } else { "var(--text-muted)" },
                        if sidebar_tab_signal.get() == "recent" { "var(--accent-color)" } else { "transparent" }
                    )
                    on:click=move |_| set_sidebar_tab_signal.set("recent".to_string())
                >
                    "Recent"
                </button>
            </div>
            <div class="file-list" style="flex: 1; min-width: 0; overflow-y: auto; overflow-x: hidden; padding: 0.75rem 0.5rem;">
                {move || {
                    let tab = sidebar_tab_signal.get();
                    if tab == "recent" {
                        let open = current_file_signal.get();
                        let last = last_opened_file_signal.get();
                        let files_in_vault = files_signal.get();
                        let recent = recent_notes_signal.get();
                        let mut valid: Vec<RecentNoteEntry> = Vec::new();

                        let primary = if !open.is_empty() { open.clone() } else { last.clone() };
                        if !primary.is_empty() && files_in_vault.contains(&primary) {
                            let title = primary
                                .rsplit('/')
                                .next()
                                .unwrap_or(&primary)
                                .to_string();
                            valid.push(RecentNoteEntry {
                                path: primary.clone(),
                                title,
                                last_opened: Date::now() as i64,
                            });
                        }
                        for e in recent.into_iter().filter(|e| files_in_vault.contains(&e.path)) {
                            if valid.iter().any(|v| v.path == e.path) {
                                continue;
                            }
                            valid.push(e);
                        }

                        if valid.is_empty() {
                            let open_now = current_file_signal.get_untracked();
                            let last_now = last_opened_file_signal.get_untracked();
                            let files_now = files_signal.get_untracked();
                            let mut candidate = String::new();
                            if !open_now.is_empty() && files_now.contains(&open_now) {
                                candidate = open_now.clone();
                            } else if !last_now.is_empty() && files_now.contains(&last_now) {
                                candidate = last_now.clone();
                            }
                            if !candidate.is_empty() {
                                let title = candidate
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or(&candidate)
                                    .to_string();
                                valid.push(RecentNoteEntry {
                                    path: candidate.clone(),
                                    title,
                                    last_opened: Date::now() as i64,
                                });
                            }
                        }

                        if valid.is_empty() {
                            let open_final = current_file_signal.get_untracked();
                            let last_final = last_opened_file_signal.get_untracked();
                            let candidate = if !open_final.is_empty() { open_final } else { last_final };
                            if !candidate.is_empty() {
                                let title = candidate
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or(&candidate)
                                    .to_string();
                                valid.push(RecentNoteEntry {
                                    path: candidate.clone(),
                                    title,
                                    last_opened: Date::now() as i64,
                                });
                            }
                        }

                        if valid.is_empty() {
                            return view! {
                                <div style="padding: 0.5rem 0.75rem; font-size: 0.82rem; color: var(--text-muted);">
                                    "No recent notes."
                                </div>
                            }
                            .into_any();
                        }

                        let select_handler = on_select_file.clone();
                        let open_in_new_opt = on_open_in_new_pane.clone();
                        let note_colors_for_recent = note_colors_signal;
                        let set_context_menu_recent = set_sidebar_context_menu_signal;
                        return view! {
                            <>
                                {valid
                                    .into_iter()
                                    .map(|entry| {
                                        let filename = entry.path.clone();
                                        let active_path = entry.path.clone();
                                        let name = entry.title.clone();
                                        let ts = entry.last_opened;
                                        let last_opened: String = if ts > 0 {
                                            let d = Date::new(&JsValue::from_f64(ts as f64));
                                            d.to_locale_string("default", &JsValue::UNDEFINED).into()
                                        } else {
                                            String::new()
                                        };
                                        let is_active = move || current_file_signal.get() == active_path;
                                        let row_select = select_handler.clone();
                                        let open_in_new_row = open_in_new_opt.clone();
                                        let path_for_color = entry.path.clone();
                                        let path_for_style = path_for_color.clone();
                                        let note_colors_for_style = note_colors_for_recent;
                                        let set_ctx = set_context_menu_recent;
                                            view! {
                                                <div
                                                    style=move || {
                                                        let colors = note_colors_for_style.get();
                                                        let custom_color =
                                                            resolve_note_color(&colors, &path_for_style);
                                                        let (bg_styles, color_styles) = if is_active()
                                                        {
                                                            (
                                                                "background: var(--accent-color); \
                                                             color: white;"
                                                                    .to_string(),
                                                                String::new(),
                                                            )
                                                        } else if let Some(c) = custom_color {
                                                            (
                                                                String::new(),
                                                                format!("--label-color: {c}; color: {c}; border-left: 4px solid {c};"),
                                                            )
                                                        } else {
                                                            (
                                                                String::new(),
                                                                "color: var(--text-secondary);"
                                                                    .to_string(),
                                                            )
                                                        };
                                                    format!(
                                                        "padding: 0.35rem 0.65rem 0.35rem 1.1rem; \
                                                         cursor: pointer; border-radius: \
                                                         var(--radius-md); margin-bottom: 2px; \
                                                         font-size: 0.84rem; transition: \
                                                         background 0.2s, color 0.2s; \
                                                         min-width: 0; {bg_styles} {color_styles}"
                                                    )
                                                }
                                                on:click=move |ev: leptos::ev::MouseEvent| {
                                                    if ev.ctrl_key() || ev.meta_key() {
                                                        if let Some(ref open_in_new) = open_in_new_row {
                                                            open_in_new(filename.clone());
                                                        }
                                                    } else {
                                                        row_select(filename.clone());
                                                    }
                                                }
                                                on:contextmenu=move |ev: leptos::ev::MouseEvent| {
                                                    ev.prevent_default();
                                                    set_ctx.set(Some(SidebarContextMenu::File {
                                                        path: path_for_color.clone(),
                                                        x: ev.client_x() as f64,
                                                        y: ev.client_y() as f64,
                                                    }));
                                                }
                                                title=entry.path.clone()
                                            >
                                                <div style="display: flex; align-items: baseline; gap: 0.35rem; min-width: 0;">
                                                    <span style="flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                                        {name}
                                                    </span>
                                                    {move || if !last_opened.is_empty() {
                                                        view! {
                                                            <span style="margin-left: auto; flex-shrink: 0; font-size: 0.72rem; color: var(--text-muted);">
                                                                {format!("Last opened {}", last_opened)}
                                                            </span>
                                                        }
                                                        .into_any()
                                                    } else {
                                                        view! { <></> }.into_any()
                                                    }}
                                                </div>
                                                <div style="font-size: 0.74rem; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;">
                                                    {entry.path.clone()}
                                                </div>
                                            </div>
                                        }
                                    })
                                    .collect::<Vec<_>>()}
                            </>
                        }
                        .into_any();
                    } else if tab == "search" {
                        let select_handler = on_select_file.clone();
                        let open_in_new_opt = on_open_in_new_pane.clone();
                        let note_colors_for_search = note_colors_signal;
                        let set_context_menu_search = set_sidebar_context_menu_signal;
                        return view! {
                            <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                                <input
                                    node_ref=search_input_ref
                                    r#type="text"
                                    placeholder="Search notes..."
                                    style="width: 100%; padding: 0.3rem 0.5rem; font-size: 0.8rem; border-radius: var(--radius-sm); border: 1px solid var(--border-color); background: var(--bg-primary); color: var(--text-primary);"
                                    prop:value=move || search_query_signal.get()
                                    on:input=move |ev| {
                                        set_search_query_signal.set(event_target_value(&ev));
                                    }
                                />
                                <div style="padding: 0.1rem 0.1rem; font-size: 0.75rem; color: var(--text-muted);">
                                    "Searches note titles, paths, and contents (case-insensitive with small typo tolerance)."
                                </div>
                                {move || {
                                    let query = search_query_signal.get();
                                    let trimmed = query.trim().to_string();
                                    let lower_query = trimmed.to_lowercase();

                                    if lower_query.len() < 2 {
                                        return view! {
                                            <div style="padding: 0.25rem 0.1rem; font-size: 0.8rem; color: var(--text-muted);">
                                                "Type at least 2 characters to search note contents and paths."
                                            </div>
                                        }
                                        .into_any();
                                    }

                                    let tokens: Vec<String> = lower_query
                                        .split_whitespace()
                                        .filter(|t| !t.is_empty())
                                        .map(|t| t.to_string())
                                        .collect();

                                    if tokens.is_empty() {
                                        return view! {
                                            <div style="padding: 0.25rem 0.1rem; font-size: 0.8rem; color: var(--text-muted);">
                                                "Type at least 2 characters to search note contents and paths."
                                            </div>
                                        }
                                        .into_any();
                                    }

                                    let files_in_vault = files_signal.get();
                                    let notes_lower = note_texts_lower_signal.get_untracked();

                                    let mut results: Vec<(String, String, f32)> = Vec::new();
                                    for path in files_in_vault.iter() {
                                        let path_lower = path.to_lowercase();
                                        let mut score = 0.0f32;
                                        let mut all_tokens_matched = true;

                                        for token in &tokens {
                                            let mut matched = false;
                                            let mut fuzzy = false;

                                            let (m1, f1) = fuzzy_token_match(&path_lower, token);
                                            if m1 {
                                                matched = true;
                                                fuzzy = f1;
                                                score += if f1 { 3.5 } else { 5.0 };
                                            }

                                            if !matched {
                                                if let Some(lower_text) = notes_lower.get(path) {
                                                    let (m2, f2) =
                                                        fuzzy_token_match(lower_text, token);
                                                    if m2 {
                                                        matched = true;
                                                        fuzzy = f2;
                                                        score += if f2 { 2.0 } else { 3.0 };
                                                    }
                                                }
                                            }

                                            if !matched {
                                                all_tokens_matched = false;
                                                break;
                                            }

                                            if fuzzy {
                                                let len = token.chars().count() as f32;
                                                score += len.min(6.0) * 0.1;
                                            }
                                        }

                                        if all_tokens_matched {
                                            let title = path
                                                .rsplit('/')
                                                .next()
                                                .unwrap_or(path)
                                                .to_string();
                                            results.push((path.clone(), title, score));
                                        }
                                    }

                                    if results.is_empty() {
                                        return view! {
                                            <div style="padding: 0.25rem 0.1rem; font-size: 0.8rem; color: var(--text-muted);">
                                                "No notes match that search."
                                            </div>
                                        }
                                        .into_any();
                                    }

                                    results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(Ordering::Equal));

                                    view! {
                                        <>
                                            {results
                                                .into_iter()
                                                .take(100)
                                                .map(|(path, title, _score)| {
                                                    let filename = path.clone();
                                                    let path_for_title = path.clone();
                                                    let path_display = path;
                                                    let row_select = select_handler.clone();
                                                    let open_in_new_row = open_in_new_opt.clone();
                                                    let path_for_color = path_display.clone();
                                                    let path_for_style = path_for_color.clone();
                                                    let note_colors_for_style =
                                                        note_colors_for_search;
                                                    let set_ctx = set_context_menu_search;
                                                    view! {
                                                        <div
                                                            style=move || {
                                                                let colors =
                                                                    note_colors_for_style.get();
                                                                let custom_color = resolve_note_color(
                                                                    &colors,
                                                                    &path_for_style,
                                                                );
                                                                let (color_value, border_style) = custom_color
                                                                    .as_ref()
                                                                    .map(|c| (c.as_str(), format!("--label-color: {c}; border-left: 4px solid {c};")))
                                                                    .unwrap_or(("var(--text-secondary)", String::new()));
                                                                format!(
                                                                    "padding: 0.3rem 0.6rem 0.3rem \
                                                                     0.8rem; cursor: pointer; \
                                                                     border-radius: \
                                                                     var(--radius-md); \
                                                                     margin-bottom: 2px; \
                                                                     font-size: 0.82rem; \
                                                                     color: {}; {} transition: \
                                                                     background 0.15s ease; \
                                                                     min-width: 0;",
                                                                    color_value,
                                                                    border_style
                                                                )
                                                            }
                                                            on:click=move |ev: leptos::ev::MouseEvent| {
                                                                if ev.ctrl_key() || ev.meta_key() {
                                                                    if let Some(ref open_in_new) = open_in_new_row {
                                                                        open_in_new(filename.clone());
                                                                    }
                                                                } else {
                                                                    row_select(filename.clone());
                                                                }
                                                            }
                                                            on:contextmenu=move |ev: leptos::ev::MouseEvent| {
                                                                ev.prevent_default();
                                                                set_ctx.set(Some(SidebarContextMenu::File {
                                                                    path: path_for_color.clone(),
                                                                    x: ev.client_x() as f64,
                                                                    y: ev.client_y() as f64,
                                                                }));
                                                            }
                                                            title=path_for_title
                                                        >
                                                            <div style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;">
                                                                {title}
                                                            </div>
                                                            <div style="font-size: 0.74rem; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;">
                                                                {path_display}
                                                            </div>
                                                        </div>
                                                    }
                                                })
                                                .collect::<Vec<_>>()
                                            }
                                        </>
                                    }
                                    .into_any()
                                }}
                            </div>
                        }
                        .into_any();
                    }

                    // Re-run when note_colors is hydrated (e.g. from fallback or import) so coloured rows appear.
                    let _ = note_colors_signal.get();
                    let _ = note_colors_hydrated_signal.get();
                    let _ = note_colors_epoch_signal.get();
                    let files_in_vault = files_signal.get();
                    // Defer drawing the tree until note_colors are hydrated so the first paint shows
                    // label colours (from .bedrock/obsidian-file-color fallback or note-colors.json).
                    if !note_colors_hydrated_signal.get() {
                        return view! {
                            <div style="padding: 0.5rem 0.75rem; font-size: 0.82rem; color: var(--text-muted);">
                                "Loading..."
                            </div>
                        }
                        .into_any();
                    }
                    if files_in_vault.is_empty() {
                        let msg = if vault_path_signal.get().is_empty() {
                            "Open a Bedrock vault to see notes."
                        } else {
                            "No markdown notes in this vault."
                        };
                        return view! {
                            <div style="padding: 0.5rem 0.75rem; font-size: 0.82rem; color: var(--text-muted);">
                                {msg}
                            </div>
                        }
                        .into_any();
                    }

                    let mut tree = build_file_tree(&files_in_vault);
                    add_empty_dirs_to_tree(&mut tree, &empty_dirs_signal.get());
                    let rows = build_sidebar_entries(&tree, &expanded_folders_read.get());

                    // Capture note_colors so this block re-runs when they change; row closures use live signal.
                    let _note_colors_snapshot = note_colors_signal.get();

                    let select_handler = on_select_file.clone();
                    let open_in_new_opt = on_open_in_new_pane.clone();
                    let set_expanded = set_expanded_folders_signal;
                    let set_context_menu = set_sidebar_context_menu_signal;
                    let current_file_for_rows = current_file_signal;
                    let note_colors_for_files = note_colors_signal;
                    let epoch_signal = note_colors_epoch_signal;
                    view! {
                        <div
                            class="file-list-note-colors"
                            aria-label=move || {
                                let n = note_colors_for_files.get().len();
                                if n > 0 {
                                    format!("Files, {} label colors", n)
                                } else {
                                    "Files".to_string()
                                }
                            }
                            data-note-colors-count=move || {
                                let colors = note_colors_for_files.get();
                                colors.len().to_string()
                            }
                            data-note-colors-sample=move || {
                                let colors = note_colors_for_files.get();
                                let keys: Vec<String> = colors.keys().take(3).cloned().collect();
                                keys.join(",")
                            }
                            data-note-colors-epoch=move || note_colors_epoch_signal.get().to_string()
                        >
                        <>
                            {rows
                                .into_iter()
                                .map(|row| {
                                    match row {
                                        SidebarEntry::Folder { path, name, depth, note_count, expanded } => {
                                            let toggle_path = path.clone();
                                            let context_path = path.clone();
                                            let path_for_attr = path.clone();
                                            let path_for_style = path.clone();
                                            let path_for_label_hex = path.clone();
                                            let path_for_class = path.clone();
                                            let path_for_name_span = path.clone();
                                            let path_canonical_key = canonical_note_color_key(&path);
                                            // Use live signal so attributes and style update when note_colors load async
                                            let note_colors_for_attr = note_colors_signal;
                                            let note_colors_for_style = note_colors_signal;
                                            let indent = 0.45 + (depth as f32 * 0.95);
                                            let chevron = if expanded { "▾" } else { "▸" };
                                            let row_bg = if expanded {
                                                "background: color-mix(in srgb, var(--accent-color) 11%, transparent);"
                                            } else {
                                                ""
                                            };

                                            view! {
                                                <div
                                                    class=move || {
                                                        let has = resolve_note_color(&note_colors_for_attr.get(), &path_for_class).is_some();
                                                        if has { "folder-item has-label-color" } else { "folder-item" }
                                                    }
                                                    data-has-label-color=move || if resolve_note_color(&note_colors_for_attr.get(), &path_for_attr).is_some() { "true" } else { "false" }
                                                    data-path-for-color=path_canonical_key.clone()
                                                    data-label-hex=move || resolve_note_color(&note_colors_for_attr.get(), &path_for_label_hex).unwrap_or_default()
                                                    style=move || {
                                                        let _ = epoch_signal.get();
                                                        let colors = note_colors_for_style.get();
                                                        let color_style = resolve_note_color(&colors, &path_for_style)
                                                            .map(|c| format!("--label-color: {c}; color: {c}; border-left: 4px solid {c}; background: color-mix(in srgb, {c} 14%, transparent); box-sizing: border-box;"))
                                                            .unwrap_or_else(|| "color: var(--text-secondary);".to_string());
                                                        format!(
                                                            "display: flex; align-items: center; gap: 0.4rem; padding: 0.34rem 0.5rem 0.34rem {indent}rem; cursor: pointer; border-radius: var(--radius-md); margin-bottom: 2px; font-size: 0.82rem; transition: background 0.15s ease; min-width: 0; box-sizing: border-box; {row_bg} {color_style}"
                                                        )
                                                    }
                                                    on:click=move |_| {
                                                        set_expanded.update(|expanded_set| {
                                                            if expanded_set.contains(&toggle_path) {
                                                                expanded_set.remove(&toggle_path);
                                                            } else {
                                                                expanded_set.insert(toggle_path.clone());
                                                            }
                                                        });
                                                    }
                                                    on:contextmenu=move |ev: leptos::ev::MouseEvent| {
                                                        ev.prevent_default();
                                                        set_context_menu.set(Some(SidebarContextMenu::Folder { path: context_path.clone(), x: ev.client_x() as f64, y: ev.client_y() as f64 }));
                                                    }
                                                    title=path
                                                >
                                                    <span style="width: 0.8rem; text-align: center; color: var(--text-muted);">
                                                        {chevron}
                                                    </span>
                                                    <span
                                                        style=move || {
                                                            let _ = epoch_signal.get();
                                                            let colors = note_colors_for_style.get();
                                                            let hex = resolve_note_color(&colors, &path_for_name_span);
                                                            match hex {
                                                                Some(c) => format!("overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; flex: 1; color: {c};"),
                                                                None => "overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; flex: 1;".to_string(),
                                                            }
                                                        }
                                                    >
                                                        {name}
                                                    </span>
                                                    <span style="margin-left: auto; font-size: 0.72rem; color: var(--text-muted);">
                                                        {note_count.to_string()}
                                                    </span>
                                                </div>
                                            }
                                            .into_any()
                                        }
                                        SidebarEntry::File { path, name, depth } => {
                                            let filename = path.clone();
                                            let active_path = path.clone();
                                            let context_file_path = path.clone();
                                            let path_canonical_key = canonical_note_color_key(&path);
                                            let note_colors_for_attr_f = note_colors_signal;
                                            let note_colors_for_style_f = note_colors_signal;
                                            let is_active = move || current_file_for_rows.get() == active_path;
                                            let indent = 1.5 + (depth as f32 * 0.95);
                                            let row_select = select_handler.clone();
                                            let open_in_new_row = open_in_new_opt.clone();
                                            let set_context_menu_row = set_context_menu.clone();
                                            let path_for_attr = path.clone();
                                            let path_for_style = path.clone();
                                            let path_for_label_hex = path.clone();
                                            let path_for_class_f = path.clone();
                                            let path_for_name_span_f = path.clone();
                                            view! {
                                                <div
                                                    class=move || {
                                                        let has = resolve_note_color(&note_colors_for_attr_f.get(), &path_for_class_f).is_some();
                                                        if has { "file-item has-label-color" } else { "file-item" }
                                                    }
                                                    data-has-label-color=move || if resolve_note_color(&note_colors_for_attr_f.get(), &path_for_attr).is_some() { "true" } else { "false" }
                                                    data-path-for-color=path_canonical_key.clone()
                                                    data-label-hex=move || resolve_note_color(&note_colors_for_attr_f.get(), &path_for_label_hex).unwrap_or_default()
                                                    style=move || {
                                                        let _ = epoch_signal.get();
                                                        let colors = note_colors_for_style_f.get();
                                                        let custom_color =
                                                            resolve_note_color(&colors, &path_for_style);
                                                        let (bg_styles, color_styles) =
                                                            if is_active() {
                                                                (
                                                                    "background: \
                                                                     var(--accent-color); color: \
                                                                     white;"
                                                                        .to_string(),
                                                                    String::new(),
                                                                )
                                                            } else if let Some(c) = custom_color {
                                                                (format!("background: color-mix(in srgb, {c} 14%, transparent);"), format!("--label-color: {c}; color: {c}; border-left: 4px solid {c}; box-sizing: border-box;"))
                                                            } else {
                                                                (
                                                                    String::new(),
                                                                    "color: var(--text-secondary);"
                                                                        .to_string(),
                                                                )
                                                            };
                                                        format!(
                                                            "padding: 0.38rem 0.65rem 0.38rem \
                                                             {indent}rem; cursor: pointer; \
                                                             border-radius: var(--radius-md); \
                                                             margin-bottom: 2px; font-size: \
                                                             0.84rem; transition: background \
                                                             0.2s, color 0.2s; min-width: 0; \
                                                             box-sizing: border-box; \
                                                             {bg_styles} {color_styles}"
                                                        )
                                                    }
                                                    on:click=move |ev: leptos::ev::MouseEvent| {
                                                        if ev.ctrl_key() || ev.meta_key() {
                                                            if let Some(ref open_in_new) = open_in_new_row {
                                                                open_in_new(filename.clone());
                                                            }
                                                        } else {
                                                            row_select(filename.clone());
                                                        }
                                                    }
                                                    on:contextmenu=move |ev: leptos::ev::MouseEvent| {
                                                        ev.prevent_default();
                                                        set_context_menu_row.set(Some(SidebarContextMenu::File { path: context_file_path.clone(), x: ev.client_x() as f64, y: ev.client_y() as f64 }));
                                                    }
                                                    title=path
                                                >
                                                    <span
                                                        style=move || {
                                                            let _ = epoch_signal.get();
                                                            let colors = note_colors_for_style_f.get();
                                                            let hex = resolve_note_color(&colors, &path_for_name_span_f);
                                                            match hex {
                                                                Some(c) => format!("display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; color: {c};"),
                                                                None => "display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;".to_string(),
                                                            }
                                                        }
                                                    >
                                                        {name}
                                                    </span>
                                                </div>
                                            }
                                            .into_any()
                                        }
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </>
                        </div>
                    }
                    .into_any()
                }}
            </div>
        </nav>
        {move || match sidebar_context_menu_read.get() {
            Some(SidebarContextMenu::Folder { path, x, y }) => {
                let path_for_note = path.clone();
                let path_for_folder = path.clone();
                let path_for_delete = path.clone();
                let path_for_color = path.clone();
                let close_menu = set_sidebar_context_menu_signal;
                let create_note = create_note_in_folder.clone();
                let create_folder = create_folder_in_folder.clone();
                let delete_folder_cb = delete_folder.clone();
                let set_color_cb = on_set_note_color.clone();
                let set_color_cb_for_palette = set_color_cb.clone();
                let path_for_color_palette = path_for_color.clone();
                view! {
                    <div
                        style="position: fixed; inset: 0; z-index: 1000;"
                        on:click=move |_| close_menu.set(None)
                    >
                        <div
                            style=format!("position: absolute; left: {}px; top: {}px; background: var(--bg-secondary); border: 1px solid var(--border-color); border-radius: var(--radius-md); padding: 0.25rem 0; box-shadow: 0 4px 12px rgba(0,0,0,0.15); min-width: 11rem;", x, y)
                            on:click=move |ev| ev.stop_propagation()
                        >
                            <div style="padding: 0.35rem 0.75rem; font-size: 0.78rem; font-weight: 500; color: var(--text-muted); border-bottom: 1px solid color-mix(in srgb, var(--border-color) 65%, transparent);">
                                "Label color"
                            </div>
                            {move || {
                                let palette = note_color_palette_signal.get();
                                let colors: Vec<(String, String)> = if palette.is_empty() {
                                    DEFAULT_LABEL_COLORS
                                        .iter()
                                        .map(|(n, v)| (n.to_string(), v.to_string()))
                                        .collect()
                                } else {
                                    palette
                                        .into_iter()
                                        .map(|e| (e.name, e.value))
                                        .collect()
                                };
                                colors
                                    .into_iter()
                                    .map(|(name, value)| {
                                        let path_f = path_for_color_palette.clone();
                                        let cb = set_color_cb_for_palette.clone();
                                        let close = close_menu.clone();
                                        let value_style = value.clone();
                                        view! {
                                            <button
                                                style="display: flex; align-items: center; gap: 0.4rem; width: 100%; padding: 0.38rem 0.75rem; text-align: left; font-size: 0.84rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary);"
                                                on:click=move |_| {
                                                    cb(path_f.clone(), Some(value_style.clone()));
                                                    close.set(None);
                                                }
                                            >
                                                <span style=format!("width: 0.85rem; height: 0.85rem; border-radius: 999px; background: {};", value)></span>
                                                <span>{name}</span>
                                            </button>
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            }}
                            <button
                                style="display: block; width: 100%; padding: 0.4rem 0.75rem; text-align: left; font-size: 0.84rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary); border-top: 1px solid color-mix(in srgb, var(--border-color) 65%, transparent);"
                                on:click={
                                    let cb = set_color_cb.clone();
                                    let path = path_for_color.clone();
                                    move |_| {
                                        cb(path.clone(), None);
                                        close_menu.set(None);
                                    }
                                }
                            >
                                "Clear color"
                            </button>
                            <button
                                style="display: block; width: 100%; padding: 0.4rem 0.75rem; text-align: left; font-size: 0.85rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary); border-top: 1px solid color-mix(in srgb, var(--border-color) 65%, transparent);"
                                on:click=move |_| create_note(path_for_note.clone())
                            >
                                "New note"
                            </button>
                            <button
                                style="display: block; width: 100%; padding: 0.4rem 0.75rem; text-align: left; font-size: 0.85rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary);"
                                on:click=move |_| create_folder(path_for_folder.clone())
                            >
                                "New folder"
                            </button>
                            <button
                                style="display: block; width: 100%; padding: 0.4rem 0.75rem; text-align: left; font-size: 0.85rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary);"
                                on:click=move |_| delete_folder_cb(path_for_delete.clone())
                            >
                                "Delete folder"
                            </button>
                        </div>
                    </div>
                }
                .into_any()
            }
            Some(SidebarContextMenu::File { path, x, y }) => {
                let path_for_delete = path.clone();
                let close_menu = set_sidebar_context_menu_signal;
                let delete_note_cb = delete_note.clone();
                let set_color_cb = on_set_note_color.clone();
                let set_color_cb_for_palette = set_color_cb.clone();
                let path_for_palette = path.clone();
                view! {
                    <div
                        style="position: fixed; inset: 0; z-index: 1000;"
                        on:click=move |_| close_menu.set(None)
                    >
                        <div
                            style=format!("position: absolute; left: {}px; top: {}px; background: var(--bg-secondary); border: 1px solid var(--border-color); border-radius: var(--radius-md); padding: 0.25rem 0; box-shadow: 0 4px 12px rgba(0,0,0,0.15); min-width: 11rem;", x, y)
                            on:click=move |ev| ev.stop_propagation()
                        >
                            <div style="padding: 0.35rem 0.75rem; font-size: 0.78rem; font-weight: 500; color: var(--text-muted); border-bottom: 1px solid color-mix(in srgb, var(--border-color) 65%, transparent);">
                                "Label color"
                            </div>
                            {move || {
                                let palette = note_color_palette_signal.get();
                                let colors: Vec<(String, String)> = if palette.is_empty() {
                                    DEFAULT_LABEL_COLORS
                                        .iter()
                                        .map(|(n, v)| (n.to_string(), v.to_string()))
                                        .collect()
                                } else {
                                    palette
                                        .into_iter()
                                        .map(|e| (e.name, e.value))
                                        .collect()
                                };
                                colors
                                    .into_iter()
                                    .map(|(name, value)| {
                                        let path_f = path_for_palette.clone();
                                        let cb = set_color_cb_for_palette.clone();
                                        let close = close_menu.clone();
                                        let value_style = value.clone();
                                        view! {
                                            <button
                                                style="display: flex; align-items: center; gap: 0.4rem; width: 100%; padding: 0.38rem 0.75rem; text-align: left; font-size: 0.84rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary);"
                                                on:click=move |_| {
                                                    cb(path_f.clone(), Some(value_style.clone()));
                                                    close.set(None);
                                                }
                                            >
                                                <span style=format!("width: 0.85rem; height: 0.85rem; border-radius: 999px; background: {};", value)></span>
                                                <span>{name}</span>
                                            </button>
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            }}
                            <button
                                style="display: block; width: 100%; padding: 0.4rem 0.75rem; text-align: left; font-size: 0.84rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary); border-top: 1px solid color-mix(in srgb, var(--border-color) 65%, transparent);"
                                on:click={
                                    let cb = set_color_cb.clone();
                                    let path = path.clone();
                                    move |_| {
                                        cb(path.clone(), None);
                                        close_menu.set(None);
                                    }
                                }
                            >
                                "Clear color"
                            </button>
                            <button
                                style="display: block; width: 100%; padding: 0.4rem 0.75rem; text-align: left; font-size: 0.85rem; background: transparent; border: none; cursor: pointer; color: var(--text-primary);"
                                on:click=move |_| delete_note_cb(path_for_delete.clone())
                            >
                                "Delete note"
                            </button>
                        </div>
                    </div>
                }
                .into_any()
            }
            None => view! { <></> }.into_any(),
        }}
    }
}

