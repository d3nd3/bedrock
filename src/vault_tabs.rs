use crate::path_utils::vault_display_name;
use leptos::prelude::*;

#[component]
pub fn VaultTabs<FOpen, FClose, FNew, FImport, FSettings, FSwitch, FCooldown>(
    vault_path: ReadSignal<String>,
    open_vaults: ReadSignal<Vec<String>>,
    on_open_vault: FOpen,
    on_close_vault: FClose,
    on_new_note: FNew,
    on_import_obsidian: FImport,
    on_open_settings: FSettings,
    on_switch_vault: FSwitch,
    on_vault_switch_gesture: FCooldown,
) -> impl IntoView
where
    FOpen: Fn() + 'static + Clone,
    FClose: Fn() + 'static + Clone,
    FNew: Fn() + 'static + Clone,
    FImport: Fn() + 'static + Clone,
    FSettings: Fn() + 'static + Clone,
    FSwitch: Fn(String) + 'static + Clone + Send,
    FCooldown: Fn() + 'static + Clone + Send,
{
    let (dropdown_open, set_dropdown_open) = signal(false);
    let cooldown_for_button = on_vault_switch_gesture.clone();

    view! {
        <div class="sidebar-header" style="display: flex; flex-direction: column; gap: 0.75rem; padding: 0.65rem 0.75rem; border-bottom: 1px solid var(--border-color); overflow: hidden; min-width: 0; width: 100%; max-width: 100%; flex-shrink: 0; box-sizing: border-box;">
            <div style="position: relative; display: flex; flex-direction: column; gap: 0.25rem; min-width: 0; width: 100%; max-width: 100%; overflow: visible; box-sizing: border-box;">
                <button
                    type="button"
                    data-vault-switcher="true"
                    style="width: 100%; min-width: 0; max-width: 100%; font-size: 0.78rem; padding: 0.2rem 0.35rem; border-radius: var(--radius-sm); border: 1px solid var(--border-color); background: var(--bg-primary); color: var(--text-primary); overflow: hidden; text-overflow: ellipsis; box-sizing: border-box; text-align: left; cursor: pointer;"
                    on:click=move |_| {
                        cooldown_for_button();
                        set_dropdown_open.update(|open| *open = !*open);
                    }
                >
                    {move || {
                        let path = vault_path.get();
                        if path.is_empty() {
                            "No open vaults".to_string()
                        } else {
                            vault_display_name(&path)
                        }
                    }}
                </button>
                {move || {
                    if !dropdown_open.get() {
                        return view! { <></> }.into_any();
                    }
                    let vaults = open_vaults.get();
                    if vaults.is_empty() {
                        return view! { <></> }.into_any();
                    }
                    let gesture = on_vault_switch_gesture.clone();
                    let switch_fn = on_switch_vault.clone();
                    view! {
                        <>
                            <div
                                style="position: fixed; inset: 0; z-index: 999;"
                                aria-hidden="true"
                                on:click=move |_| set_dropdown_open.set(false)
                            />
                            <ul
                                style="position: absolute; top: 100%; left: 0; right: 0; margin: 0.15rem 0 0 0; padding: 0.25rem 0; list-style: none; background: var(--bg-primary); border: 1px solid var(--border-color); border-radius: var(--radius-sm); box-shadow: 0 4px 12px rgba(0,0,0,0.15); z-index: 1000; max-height: 12rem; overflow-y: auto;"
                                role="listbox"
                            >
                                {vaults.into_iter().map(|path| {
                                    let label = vault_display_name(&path);
                                    let is_current = path == vault_path.get();
                                    let path_clone = path.clone();
                                    let g = gesture.clone();
                                    let s = switch_fn.clone();
                                    view! {
                                        <li role="option" aria-selected=is_current>
                                            <button
                                                type="button"
                                                style="width: 100%; font-size: 0.78rem; padding: 0.25rem 0.5rem; border: none; background: transparent; color: var(--text-primary); text-align: left; cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;"
                                                on:click=move |_| {
                                                    g();
                                                    s(path_clone.clone());
                                                    set_dropdown_open.set(false);
                                                }
                                            >
                                                {label}
                                            </button>
                                        </li>
                                    }
                                }).collect::<Vec<_>>()}
                            </ul>
                        </>
                    }.into_any()
                }}
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
            <div style="display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; flex-shrink: 0; min-width: 0; overflow: hidden;">
                <span style="font-weight: 700; color: var(--accent-color); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;">
                    "Bedrock"
                </span>
                <div style="display: flex; gap: 0.3rem; align-items: center; flex-shrink: 0; overflow: visible;">
                    <button
                        data-open-vault="true"
                        on:click=move |_| on_open_vault.clone()()
                        style="background: transparent; border: none; font-size: 0.95rem; cursor: pointer; color: var(--text-muted); flex-shrink: 0;"
                        title="Open or add a vault"
                    >
                        "🗂"
                    </button>
                    <button
                        on:click=move |_| on_close_vault.clone()()
                        style="background: transparent; border: none; font-size: 0.95rem; cursor: pointer; color: var(--text-muted); flex-shrink: 0;"
                        title="Close current vault"
                    >
                        "✕"
                    </button>
                    <button
                        on:click=move |_| on_new_note.clone()()
                        style="background: transparent; border: none; font-size: 1.1rem; cursor: pointer; color: var(--text-muted); flex-shrink: 0;"
                        title="New note"
                    >
                        "+"
                    </button>
                    <button
                        data-import-obsidian="true"
                        on:click=move |_| on_import_obsidian.clone()()
                        style="background: transparent; border: none; font-size: 1.1rem; cursor: pointer; color: var(--text-muted); flex-shrink: 0; padding: 0; min-width: 1.5rem;"
                        title="Import from Obsidian vault"
                    >
                        "⇪"
                    </button>
                    <button
                        on:click=move |_| on_open_settings.clone()()
                        style="background: transparent; border: none; font-size: 1.1rem; cursor: pointer; color: var(--text-muted); flex-shrink: 0;"
                        title="Settings"
                    >
                        "⚙"
                    </button>
                </div>
            </div>
        </div>
    }
}
