use crate::path_utils::vault_display_name;
use leptos::prelude::*;

#[component]
pub fn VaultTabs<FOpen, FClose, FNew, FImport, FSettings, FSwitch>(
    vault_path: ReadSignal<String>,
    open_vaults: ReadSignal<Vec<String>>,
    on_open_vault: FOpen,
    on_close_vault: FClose,
    on_new_note: FNew,
    on_import_obsidian: FImport,
    on_open_settings: FSettings,
    on_switch_vault: FSwitch,
) -> impl IntoView
where
    FOpen: Fn() + 'static + Clone,
    FClose: Fn() + 'static + Clone,
    FNew: Fn() + 'static + Clone,
    FImport: Fn() + 'static + Clone,
    FSettings: Fn() + 'static + Clone,
    FSwitch: Fn(String) + 'static + Clone,
{
    view! {
        <div class="sidebar-header" style="display: flex; flex-direction: column; gap: 0.5rem; padding: 0.65rem 0.75rem; border-bottom: 1px solid var(--border-color);">
            <div style="display: flex; align-items: center; justify-content: space-between; gap: 0.5rem;">
                <span style="font-weight: 700; color: var(--accent-color);">
                    "Bedrock"
                </span>
                <div style="display: flex; gap: 0.3rem; align-items: center;">
                    <button
                        on:click=move |_| on_open_vault.clone()()
                        style="background: transparent; border: none; font-size: 0.95rem; cursor: pointer; color: var(--text-muted);"
                        title="Open or add a vault"
                    >
                        "🗂"
                    </button>
                    <button
                        on:click=move |_| on_close_vault.clone()()
                        style="background: transparent; border: none; font-size: 0.95rem; cursor: pointer; color: var(--text-muted);"
                        title="Close current vault"
                    >
                        "✕"
                    </button>
                    <button
                        on:click=move |_| on_new_note.clone()()
                        style="background: transparent; border: none; font-size: 1.1rem; cursor: pointer; color: var(--text-muted);"
                        title="New note"
                    >
                        "+"
                    </button>
                    <button
                        on:click=move |_| on_import_obsidian.clone()()
                        style="background: transparent; border: none; font-size: 1rem; cursor: pointer; color: var(--text-muted);"
                        title="Import from Obsidian vault"
                    >
                        "⇪"
                    </button>
                    <button
                        on:click=move |_| on_open_settings.clone()()
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
                    on:change=move |e| on_switch_vault.clone()(event_target_value(&e))
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
    }
}

