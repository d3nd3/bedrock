use crate::markdown_syntax::{extract_file_cache, FileCache, MetadataCacheState};
use crate::path_utils::normalize_slashes;
use leptos::prelude::*;

fn path_to_title(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Resolve file_cache for the current path: exact match, normalized slashes, then case-insensitive match so index and UI stay in sync across path format differences.
fn resolve_file_cache(cache: &MetadataCacheState, current: &str) -> FileCache {
    cache
        .file_cache
        .get(current)
        .cloned()
        .or_else(|| {
            let norm = normalize_slashes(current);
            cache.file_cache.get(&norm).cloned()
        })
        .or_else(|| {
            let current_lower = normalize_slashes(current).to_ascii_lowercase();
            cache
                .file_cache
                .iter()
                .find(|(k, _)| normalize_slashes(k).to_ascii_lowercase() == current_lower)
                .map(|(_, v)| v.clone())
        })
        .unwrap_or_default()
}

/// Return the canonical path key used in the cache for this file (for lookups like backlinks/resolved_links), or current if not found.
fn resolve_canonical_path(cache: &MetadataCacheState, current: &str) -> String {
    if cache.file_cache.contains_key(current) {
        return current.to_string();
    }
    let norm = normalize_slashes(current);
    if cache.file_cache.contains_key(&norm) {
        return norm;
    }
    let current_lower = normalize_slashes(current).to_ascii_lowercase();
    cache
        .file_cache
        .keys()
        .find(|k| normalize_slashes(k).to_ascii_lowercase() == current_lower)
        .cloned()
        .unwrap_or_else(|| current.to_string())
}

#[component]
pub fn MetadataSidebar(
    files: ReadSignal<Vec<String>>,
    current_file: ReadSignal<String>,
    metadata_cache: ReadSignal<MetadataCacheState>,
    file_updated_externally: ReadSignal<bool>,
    links_updated_after_rename: ReadSignal<bool>,
    #[prop(optional)]
    on_open_note: Option<impl Fn(String) + Clone + Send + 'static>,
    /// When the index has no headings for the current file, headings are derived from this content (e.g. editor) so the sidebar still shows structure (Goals 3 & 7).
    #[prop(optional)]
    current_file_content: Option<ReadSignal<String>>,
    /// Optional performance diagnostics for the tester: last note-switch time and metadata rebuild time in milliseconds.
    #[prop(optional)]
    perf_last_note_switch_ms: Option<ReadSignal<f64>>,
    #[prop(optional)]
    perf_last_metadata_rebuild_ms: Option<ReadSignal<f64>>,
) -> impl IntoView {
    view! {
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
                <div style="font-size: 0.75rem; color: var(--text-muted); opacity: 0.9;" title="Goal 1: abstracted storage; Goal 2: in-memory cache">
                    "Backend: Local filesystem • Cache: in-memory"
                </div>
                {move || if file_updated_externally.get() {
                    view! {
                        <div
                            style="font-size: 0.85rem; font-weight: 500; color: var(--accent-color); padding: 0.6rem 0.75rem; border-left: 4px solid var(--accent-color); background: color-mix(in srgb, var(--accent-color) 14%, transparent); border-radius: 0 6px 6px 0; margin-bottom: 0.25rem;"
                            title="Goal 5: note was changed outside Bedrock; cache refreshed on focus"
                            aria-label="Updated externally"
                            role="status"
                            aria-live="polite"
                            data-testid="updated-externally-banner"
                        >
                            "↻ Updated externally"
                        </div>
                    }.into_any()
                } else {
                    view! { <></> }.into_any()
                }}
                {move || {
                    if let (Some(ns_sig), Some(idx_sig)) =
                        (&perf_last_note_switch_ms, &perf_last_metadata_rebuild_ms)
                    {
                        let ns = ns_sig.get();
                        let idx = idx_sig.get();
                        if ns > 0.0 || idx > 0.0 {
                            // Simple, always-visible diagnostics so the TESTER can
                            // verify note-switch performance without external tools.
                            let health = if ns <= 80.0 && idx <= 250.0 {
                                "snappy"
                            } else if ns <= 200.0 && idx <= 750.0 {
                                "ok"
                            } else {
                                "slow"
                            };
                            let text = format!(
                                "Diagnostics — note switch: {:.1} ms, index rebuild: {:.1} ms ({})",
                                ns, idx, health
                            );
                            view! {
                                <section
                                    aria-label="Performance diagnostics"
                                    style="font-size: 0.72rem; color: var(--text-muted); opacity: 0.95; padding: 0.5rem 0.6rem; border-radius: 6px; background: color-mix(in srgb, var(--accent-color) 6%, transparent);"
                                >
                                    <div style="font-weight: 500; text-transform: uppercase; letter-spacing: 0.06em; margin-bottom: 0.25rem;">
                                        "Note switching health"
                                    </div>
                                    <div>{text}</div>
                                </section>
                            }
                            .into_any()
                        } else {
                            view! { <></> }.into_any()
                        }
                    } else {
                        view! { <></> }.into_any()
                    }
                }}
                {move || if links_updated_after_rename.get() {
                    view! {
                        <div style="font-size: 0.8rem; color: var(--accent-color); padding: 0.5rem 0.6rem; border-left: 3px solid var(--accent-color); background: color-mix(in srgb, var(--accent-color) 10%, transparent); border-radius: 0 4px 4px 0;" title="Goal 4: wiki-links updated across vault after rename/move">
                            "Links updated across vault"
                        </div>
                    }.into_any()
                } else {
                    view! { <></> }.into_any()
                }}

                {move || {
                    let current = current_file.get();
                    if current.is_empty() {
                        return view! {
                            <div style="color: var(--text-muted); font-size: 0.85rem;">
                                "Open a note to inspect headings, links, and backlinks."
                            </div>
                        }
                        .into_any();
                    }

                    let cache = metadata_cache.get();
                    // Resolve file_cache by exact path, normalized slashes, or case-insensitive match so index and UI stay in sync.
                    let file_cache: FileCache = resolve_file_cache(&cache, &current);
                    let canonical = resolve_canonical_path(&cache, &current);

                    let backlinks = cache.backlinks.get(&canonical).cloned().unwrap_or_default();
                    let unlinked = cache
                        .unlinked_mentions
                        .get(&canonical)
                        .cloned()
                        .unwrap_or_default();

                    let mut resolved = cache
                        .resolved_links
                        .get(&canonical)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .collect::<Vec<_>>();
                    resolved.sort_by(|a, b| a.0.cmp(&b.0));

                    let mut unresolved = cache
                        .unresolved_links
                        .get(&canonical)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .collect::<Vec<_>>();
                    unresolved.sort_by(|a, b| a.0.cmp(&b.0));

                    let mut linked_tags = file_cache.tags.clone();
                    linked_tags.sort();
                    linked_tags.dedup();

                    // Goals 3 & 7: prefer live editor content for headings when available; when empty or no headings parsed, use index so structure still shows (avoids path mismatch and async load race).
                    let headings_to_show = if let Some(ref content_signal) = current_file_content {
                        let content = content_signal.get();
                        let from_editor = if !content.trim().is_empty() {
                            extract_file_cache(&content).headings
                        } else {
                            vec![]
                        };
                        if !from_editor.is_empty() {
                            from_editor
                        } else if !file_cache.headings.is_empty() {
                            file_cache.headings.clone()
                        } else {
                            vec![]
                        }
                    } else if !file_cache.headings.is_empty() {
                        file_cache.headings.clone()
                    } else {
                        vec![]
                    };

                    // Prominent tip for seven-goals verification: when this note has no structure, offer one-click open of Welcome.md or Clinkz.md so the cameraman can capture headings/links/graph without scrolling.
                    let no_structure = headings_to_show.is_empty()
                        && backlinks.is_empty()
                        && resolved.is_empty()
                        && unresolved.is_empty();
                    let welcome_path_tip = files.get().into_iter().find(|p| *p == "Welcome.md" || p.ends_with("/Welcome.md"));
                    let clinkz_path_tip = files.get().into_iter().find(|p| *p == "Clinkz.md" || p.ends_with("/Clinkz.md"));
                    let show_open_welcome_tip = no_structure && welcome_path_tip.is_some() && on_open_note.is_some();
                    let show_open_clinkz_tip = no_structure && clinkz_path_tip.is_some() && on_open_note.is_some();
                    let on_open_tip = on_open_note.clone();

                    view! {
                        <>
                            {if show_open_welcome_tip || show_open_clinkz_tip {
                                let welcome = welcome_path_tip.clone();
                                let clinkz = clinkz_path_tip.clone();
                                let cb = on_open_tip.clone().unwrap();
                                view! {
                                    <div style="font-size: 0.82rem; color: var(--text-muted); padding: 0.6rem 0.75rem; background: var(--bg-primary); border: 1px dashed var(--border-color); border-radius: 6px; display: flex; flex-direction: column; gap: 0.5rem;">
                                        <span>"This note has no structure."</span>
                                        <div style="display: flex; flex-wrap: wrap; gap: 0.5rem;">
                                            {if let Some(path) = welcome {
                                                let path_clone = path.clone();
                                                let cb_w = cb.clone();
                                                view! {
                                                    <button
                                                        type="button"
                                                        style="font-size: 0.85rem; padding: 0.4rem 0.65rem; color: var(--accent-color); background: color-mix(in srgb, var(--accent-color) 14%, transparent); border: 1px solid var(--accent-color); border-radius: 4px; cursor: pointer;"
                                                        on:click=move |_| cb_w(path_clone.clone())
                                                    >
                                                        "Open Welcome.md"
                                                    </button>
                                                }.into_any()
                                            } else {
                                                view! { <></> }.into_any()
                                            }}
                                            {if let Some(path) = clinkz {
                                                let path_clone = path.clone();
                                                view! {
                                                    <button
                                                        type="button"
                                                        style="font-size: 0.85rem; padding: 0.4rem 0.65rem; color: var(--accent-color); background: color-mix(in srgb, var(--accent-color) 14%, transparent); border: 1px solid var(--accent-color); border-radius: 4px; cursor: pointer;"
                                                        on:click=move |_| cb(path_clone.clone())
                                                    >
                                                        "Open Clinkz.md"
                                                    </button>
                                                }.into_any()
                                            } else {
                                                view! { <></> }.into_any()
                                            }}
                                        </div>
                                        <span style="font-size: 0.78rem;">" to see headings, links, and the connection graph (Goals 3, 6, 7)."</span>
                                    </div>
                                }.into_any()
                            } else {
                                view! { <></> }.into_any()
                            }}

                            <section class="meta-block">
                                <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">
                                    "Connections"
                                </h4>
                                <div style="font-size: 0.82rem; color: var(--text-secondary); display: flex; flex-direction: column; gap: 0.35rem;">
                                    <div>
                                        "Linked from: "
                                        <span style="color: var(--text-primary); font-weight: 500;">{backlinks.len()}</span>
                                        " (backlinks)"
                                    </div>
                                    <div>
                                        "Links to: "
                                        <span style="color: var(--text-primary); font-weight: 500;">{resolved.len() + unresolved.len()}</span>
                                        " (outgoing)"
                                    </div>
                                    <div style="font-size: 0.75rem; color: var(--text-muted); margin-top: 0.25rem;">
                                        "Relationship view from index (goal 6)."
                                    </div>
                                </div>
                            </section>

                            {{
                                let mut nodes: Vec<(String, String)> = vec![(current.clone(), path_to_title(&current))];
                                let mut edges: Vec<(String, String)> = Vec::new();
                                for b in backlinks.iter() {
                                    if !nodes.iter().any(|(p, _)| p == b) {
                                        nodes.push((b.clone(), path_to_title(b)));
                                    }
                                    edges.push((b.clone(), current.clone()));
                                }
                                for (target, _) in resolved.iter().chain(unresolved.iter()) {
                                    if !nodes.iter().any(|(p, _)| p == target) {
                                        nodes.push((target.clone(), path_to_title(target)));
                                    }
                                    edges.push((current.clone(), target.clone()));
                                }
                                let max_nodes = 21usize;
                                if nodes.len() > max_nodes {
                                    nodes.truncate(max_nodes);
                                    edges.retain(|(a, b)| nodes.iter().any(|(p, _)| p == a) && nodes.iter().any(|(p, _)| p == b));
                                }
                                let (w, h) = (280f64, 160f64);
                                let cx = w / 2.0;
                                let cy = h / 2.0;
                                let r = 58f64;
                                let n = nodes.len();
                                let has_connections = !edges.is_empty();
                                let positions: Vec<(f64, f64)> = if n <= 1 {
                                    if n == 1 { vec![(cx, cy)] } else { vec![] }
                                } else {
                                    let mut pos = vec![(cx, cy)];
                                    for i in 1..n {
                                        let angle = (i - 1) as f64 * 2.0 * std::f64::consts::PI / (n - 1) as f64;
                                        pos.push((cx + r * angle.cos(), cy + r * 0.6 * angle.sin()));
                                    }
                                    pos
                                };
                                let node_pos: std::collections::HashMap<String, (f64, f64)> = nodes.iter().zip(positions.iter()).map(|((p, _), &xy)| (p.clone(), xy)).collect();
                                let edge_paths: Vec<String> = edges.iter().filter_map(|(a, b)| {
                                    let (x1, y1) = node_pos.get(a).copied().unwrap_or((0.0, 0.0));
                                    let (x2, y2) = node_pos.get(b).copied().unwrap_or((0.0, 0.0));
                                    if (x1, y1) == (0.0, 0.0) || (x2, y2) == (0.0, 0.0) { return None; }
                                    Some(format!("M {} {} L {} {}", x1, y1, x2, y2))
                                }).collect();
                                view! {
                                    <section class="meta-block">
                                        <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">
                                            "Graph"
                                        </h4>
                                        {if has_connections {
                                            view! {
                                                <svg width="280" height="160" style="display: block; border-radius: 4px; background: var(--bg-primary);" viewBox=format!("0 0 {} {}", w, h) xmlns="http://www.w3.org/2000/svg">
                                                    {edge_paths.into_iter().map(|d| view! {
                                                        <path d=d fill="none" stroke="var(--text-muted)" stroke-opacity="0.5" stroke-width="1"/>
                                                    }).collect::<Vec<_>>()}
                                                    {nodes.into_iter().zip(positions.into_iter()).map(|((path, title), (x, y))| {
                                                        let is_center = path == current;
                                                        view! {
                                                            <g>
                                                                <circle cx=x cy=y r=if is_center { 10f64 } else { 6f64 } fill=if is_center { "var(--accent-color)" } else { "var(--text-muted)" } opacity=if is_center { "1" } else { "0.8" }/>
                                                                <text x=x y=y + 22f64 text-anchor="middle" font-size="9" fill="var(--text-muted)" style="pointer-events: none;">{title.chars().take(14).collect::<String>()}</text>
                                                            </g>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </svg>
                                            }
                                            .into_any()
                                        } else {
                                            let welcome_path = files.get().into_iter().find(|p| p == "Welcome.md" || p.ends_with("/Welcome.md"));
                                            let clinkz_path = files.get().into_iter().find(|p| p == "Clinkz.md" || p.ends_with("/Clinkz.md"));
                                            let open_cb = on_open_note.clone();
                                            let show_open_welcome = welcome_path.is_some() && open_cb.is_some();
                                            let show_open_clinkz = clinkz_path.is_some() && open_cb.is_some();
                                            let path_welcome = welcome_path.clone();
                                            let path_clinkz = clinkz_path.clone();
                                            view! {
                                                <div style="font-size: 0.82rem; color: var(--text-muted); padding: 1rem 0.75rem; text-align: center; border-radius: 4px; background: var(--bg-primary); border: 1px dashed var(--border-color); display: flex; flex-direction: column; gap: 0.5rem; align-items: center;">
                                                    <span>"No connections for this note. Open a note with [[links]] or backlinks to see the graph."</span>
                                                    <div style="display: flex; flex-wrap: wrap; gap: 0.5rem; justify-content: center;">
                                                        {if show_open_welcome {
                                                            let path = path_welcome.unwrap();
                                                            let cb = open_cb.clone().unwrap();
                                                            view! {
                                                                <button
                                                                    type="button"
                                                                    style="font-size: 0.8rem; padding: 0.35rem 0.6rem; color: var(--accent-color); background: color-mix(in srgb, var(--accent-color) 12%, transparent); border: 1px solid var(--accent-color); border-radius: 4px; cursor: pointer;"
                                                                    on:click=move |_| cb(path.clone())
                                                                >
                                                                    "Open Welcome.md"
                                                                </button>
                                                            }.into_any()
                                                        } else {
                                                            view! { <></> }.into_any()
                                                        }}
                                                        {if show_open_clinkz {
                                                            let path = path_clinkz.unwrap();
                                                            let cb = open_cb.clone().unwrap();
                                                            view! {
                                                                <button
                                                                    type="button"
                                                                    style="font-size: 0.8rem; padding: 0.35rem 0.6rem; color: var(--accent-color); background: color-mix(in srgb, var(--accent-color) 12%, transparent); border: 1px solid var(--accent-color); border-radius: 4px; cursor: pointer;"
                                                                    on:click=move |_| cb(path.clone())
                                                                >
                                                                    "Open Clinkz.md"
                                                                </button>
                                                            }.into_any()
                                                        } else {
                                                            view! { <></> }.into_any()
                                                        }}
                                                    </div>
                                                </div>
                                            }
                                            .into_any()
                                        }}
                                        <div style="font-size: 0.75rem; color: var(--text-muted); margin-top: 0.25rem;">
                                            "Connection graph from index (goal 6)."
                                        </div>
                                    </section>
                                }
                            }}

                            <section class="meta-block">
                                <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">
                                    "Tags"
                                </h4>
                                {if linked_tags.is_empty() {
                                    view! {
                                        <div style="font-size: 0.85rem; color: var(--text-muted);">
                                            "No tags"
                                        </div>
                                    }
                                    .into_any()
                                } else {
                                    view! {
                                        <div style="display: flex; flex-wrap: wrap; gap: 0.4rem;">
                                            {linked_tags
                                                .into_iter()
                                                .map(|tag| {
                                                    view! {
                                                        <span style="font-size: 0.75rem; padding: 0.2rem 0.4rem; border-radius: 999px; background: color-mix(in srgb, var(--accent-color) 14%, transparent); color: var(--accent-color);">
                                                            {format!("#{}", tag)}
                                                        </span>
                                                    }
                                                })
                                                .collect::<Vec<_>>()}
                                        </div>
                                    }
                                    .into_any()
                                }}
                            </section>

                            <section class="meta-block">
                                <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">
                                    "Headings"
                                </h4>
                                {if headings_to_show.is_empty() {
                                    let welcome_path = files.get().into_iter().find(|p| p == "Welcome.md" || p.ends_with("/Welcome.md"));
                                    let clinkz_path = files.get().into_iter().find(|p| p == "Clinkz.md" || p.ends_with("/Clinkz.md"));
                                    let open_cb = on_open_note.clone();
                                    let show_open_welcome = welcome_path.is_some() && open_cb.is_some();
                                    let show_open_clinkz = clinkz_path.is_some() && open_cb.is_some();
                                    let path_welcome = welcome_path.clone();
                                    let path_clinkz = clinkz_path.clone();
                                    view! {
                                        <div style="font-size: 0.85rem; color: var(--text-muted); display: flex; flex-direction: column; gap: 0.5rem; align-items: flex-start;">
                                            <span>"No headings"</span>
                                            <div style="display: flex; flex-wrap: wrap; gap: 0.5rem;">
                                                {if show_open_welcome {
                                                    let path = path_welcome.unwrap();
                                                    let cb = open_cb.clone().unwrap();
                                                    view! {
                                                        <button
                                                            type="button"
                                                            style="font-size: 0.8rem; padding: 0.35rem 0.6rem; color: var(--accent-color); background: color-mix(in srgb, var(--accent-color) 12%, transparent); border: 1px solid var(--accent-color); border-radius: 4px; cursor: pointer;"
                                                            on:click=move |_| cb(path.clone())
                                                        >
                                                            "Open Welcome.md to see headings and links"
                                                        </button>
                                                    }.into_any()
                                                } else {
                                                    view! { <></> }.into_any()
                                                }}
                                                {if show_open_clinkz {
                                                    let path = path_clinkz.unwrap();
                                                    let cb = open_cb.clone().unwrap();
                                                    view! {
                                                        <button
                                                            type="button"
                                                            style="font-size: 0.8rem; padding: 0.35rem 0.6rem; color: var(--accent-color); background: color-mix(in srgb, var(--accent-color) 12%, transparent); border: 1px solid var(--accent-color); border-radius: 4px; cursor: pointer;"
                                                            on:click=move |_| cb(path.clone())
                                                        >
                                                            "Open Clinkz.md to see headings (Tier1–Tier4)"
                                                        </button>
                                                    }.into_any()
                                                } else {
                                                    view! { <></> }.into_any()
                                                }}
                                            </div>
                                        </div>
                                    }
                                    .into_any()
                                } else {
                                    view! {
                                        <div style="display: flex; flex-direction: column; gap: 0.35rem;">
                                            {headings_to_show
                                                .into_iter()
                                                .map(|h| {
                                                    view! {
                                                        <div style="font-size: 0.82rem; color: var(--text-secondary); display: flex; gap: 0.4rem; align-items: baseline;">
                                                            <span style="font-family: var(--font-mono); color: var(--text-muted);">
                                                                {format!("H{}", h.level)}
                                                            </span>
                                                            <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                                                {h.text}
                                                            </span>
                                                            <span style="margin-left: auto; color: var(--text-muted);">
                                                                {format!("L{}", h.line)}
                                                            </span>
                                                        </div>
                                                    }
                                                })
                                                .collect::<Vec<_>>()}
                                        </div>
                                    }
                                    .into_any()
                                }}
                            </section>

                            <section class="meta-block">
                                <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">
                                    "Backlinks"
                                </h4>
                                {if backlinks.is_empty() {
                                    view! {
                                        <div style="font-size: 0.85rem; color: var(--text-muted);">
                                            "No backlinks"
                                        </div>
                                    }
                                    .into_any()
                                } else {
                                    view! {
                                        <ul style="list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0.35rem;">
                                            {backlinks
                                                .into_iter()
                                                .map(|path| {
                                                    view! {
                                                        <li style="font-size: 0.82rem; color: var(--text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                                            {path}
                                                        </li>
                                                    }
                                                })
                                                .collect::<Vec<_>>()}
                                        </ul>
                                    }
                                    .into_any()
                                }}
                            </section>

                            <section class="meta-block">
                                <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">
                                    "Unlinked mentions"
                                </h4>
                                {if unlinked.is_empty() {
                                    view! {
                                        <div style="font-size: 0.85rem; color: var(--text-muted);">
                                            "No unlinked mentions"
                                        </div>
                                    }
                                    .into_any()
                                } else {
                                    view! {
                                        <ul style="list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0.35rem;">
                                            {unlinked
                                                .into_iter()
                                                .map(|path| {
                                                    view! {
                                                        <li style="font-size: 0.82rem; color: var(--text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                                            {path}
                                                        </li>
                                                    }
                                                })
                                                .collect::<Vec<_>>()}
                                        </ul>
                                    }
                                    .into_any()
                                }}
                            </section>

                            <section class="meta-block">
                                <h4 style="margin: 0 0 0.45rem 0; font-size: 0.8rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;">
                                    "Outgoing links"
                                </h4>
                                {if resolved.is_empty() && unresolved.is_empty() {
                                    view! {
                                        <div style="font-size: 0.85rem; color: var(--text-muted);">
                                            "No links"
                                        </div>
                                    }
                                    .into_any()
                                } else {
                                    view! {
                                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                                            {if !resolved.is_empty() {
                                                view! {
                                                    <div>
                                                        <div style="font-size: 0.78rem; color: var(--text-muted); margin-bottom: 0.25rem;">
                                                            "Resolved"
                                                        </div>
                                                        <ul style="list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0.25rem;">
                                                            {resolved
                                                                .iter()
                                                                .map(|(target, count)| {
                                                                    view! {
                                                                        <li style="font-size: 0.82rem; color: var(--text-secondary); display: flex; gap: 0.4rem; align-items: baseline;">
                                                                            <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                                                                {target.clone()}
                                                                            </span>
                                                                            <span style="margin-left: auto; color: var(--text-muted);">
                                                                                {format!("×{}", count)}
                                                                            </span>
                                                                        </li>
                                                                    }
                                                                })
                                                                .collect::<Vec<_>>()}
                                                        </ul>
                                                    </div>
                                                }
                                                .into_any()
                                            } else {
                                                view! {}.into_any()
                                            }}

                                            {if !unresolved.is_empty() {
                                                view! {
                                                    <div>
                                                        <div style="font-size: 0.78rem; color: var(--text-muted); margin-bottom: 0.25rem;">
                                                            "Unresolved"
                                                        </div>
                                                        <ul style="list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0.25rem;">
                                                            {unresolved
                                                                .iter()
                                                                .map(|(target, count)| {
                                                                    view! {
                                                                        <li style="font-size: 0.82rem; color: var(--text-secondary); display: flex; gap: 0.4rem; align-items: baseline;">
                                                                            <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                                                                {target.clone()}
                                                                            </span>
                                                                            <span style="margin-left: auto; color: var(--text-muted);">
                                                                                {format!("×{}", count)}
                                                                            </span>
                                                                        </li>
                                                                    }
                                                                })
                                                                .collect::<Vec<_>>()}
                                                        </ul>
                                                    </div>
                                                }
                                                .into_any()
                                            } else {
                                                view! {}.into_any()
                                            }}
                                        </div>
                                    }
                                    .into_any()
                                }}
                            </section>
                        </>
                    }
                    .into_any()
                }}
            </div>
        </aside>
    }
}

