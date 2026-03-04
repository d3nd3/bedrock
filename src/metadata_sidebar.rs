use crate::markdown_syntax::{FileCache, MetadataCacheState};
use leptos::prelude::*;

#[component]
pub fn MetadataSidebar(
    files: ReadSignal<Vec<String>>,
    current_file: ReadSignal<String>,
    metadata_cache: ReadSignal<MetadataCacheState>,
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
                    let file_cache: FileCache =
                        cache.file_cache.get(&current).cloned().unwrap_or_default();

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
                                {if file_cache.headings.is_empty() {
                                    view! {
                                        <div style="font-size: 0.85rem; color: var(--text-muted);">
                                            "No headings"
                                        </div>
                                    }
                                    .into_any()
                                } else {
                                    view! {
                                        <div style="display: flex; flex-direction: column; gap: 0.35rem;">
                                            {file_cache
                                                .headings
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

