use crate::path_utils::vault_display_name;
use leptos::prelude::*;

#[component]
pub fn TopBar<FOpen, FImport, FRename>(
    current_file: ReadSignal<String>,
    vault_path: ReadSignal<String>,
    save_status: ReadSignal<String>,
    on_open_vault: FOpen,
    on_import_obsidian: FImport,
    on_rename: FRename,
) -> impl IntoView
where
    FOpen: Fn() + 'static + Clone,
    FImport: Fn() + 'static + Clone,
    FRename: Fn() + 'static + Clone,
{
    view! {
        <header class="topbar" style="height: var(--topbar-height); border-bottom: 1px solid var(--border-color); display: flex; align-items: center; justify-content: space-between; padding: 0 1.5rem; color: var(--text-muted); font-size: 0.9rem; gap: 1rem;">
            <div style="display: flex; align-items: center; gap: 0.75rem; min-width: 0;">
                <div style="display: flex; flex-direction: column; min-width: 0; gap: 0.1rem;">
                    <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                        {move || current_file.get()}
                    </span>
                    <span
                        style="font-size: 0.72rem; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;"
                        title=move || vault_path.get()
                    >
                        {move || format!("Vault: {}", vault_display_name(&vault_path.get()))}
                    </span>
                </div>
                <span style="font-size: 0.8rem; color: var(--text-muted);">
                    {move || save_status.get()}
                </span>
            </div>
            <div style="display: flex; gap: 0.5rem; flex-shrink: 0; align-items: center;">
                <button
                    style="padding: 0.25rem 0.5rem; font-size: 1rem; background: transparent; border: none; cursor: pointer; color: var(--text-muted);"
                    title="Open Vault"
                    on:click=move |_| on_open_vault()
                >
                    "🗂"
                </button>
                <button
                    data-import-obsidian="true"
                    style="padding: 0.25rem 0.5rem; font-size: 1rem; background: transparent; border: none; cursor: pointer; color: var(--text-muted);"
                    title="Import from Obsidian vault"
                    on:click=move |_| on_import_obsidian()
                >
                    "⇪"
                </button>
                <button
                    style="padding: 0.25rem 0.5rem; font-size: 1rem; background: transparent; border: none; cursor: pointer; color: var(--text-muted);"
                    title="Rename"
                    on:click=move |_| on_rename()
                >
                    "✎"
                </button>
            </div>
        </header>
    }
}

