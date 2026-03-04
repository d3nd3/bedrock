use crate::app_state::RecentNoteEntry;
use js_sys::Date;
use leptos::prelude::*;
use wasm_bindgen::JsValue;

#[component]
pub fn RecentNotesPane<FSelect>(
    vault_path: ReadSignal<String>,
    recent_notes: ReadSignal<Vec<RecentNoteEntry>>,
    files: ReadSignal<Vec<String>>,
    on_select_file: FSelect,
) -> impl IntoView
where
    FSelect: Fn(String) + 'static + Clone + Send,
{
    view! {
        {move || {
            let has_vault = !vault_path.get().is_empty();
            if !has_vault {
                return view! {
                    <div style="flex: 1; display: flex; align-items: center; justify-content: center; color: var(--text-muted);">
                        "Open a Bedrock vault to begin."
                    </div>
                }
                .into_any();
            }

            let recents = recent_notes.get();
            let files_in_vault = files.get();
            let valid: Vec<RecentNoteEntry> = recents
                .into_iter()
                .filter(|e| files_in_vault.contains(&e.path))
                .collect();

            if !valid.is_empty() {
                let handler = on_select_file.clone();
                return view! {
                    <div style="flex: 1; display: flex; align-items: center; justify-content: center; padding: 2rem;">
                        <div style="max-width: 520px; width: 100%; background: var(--bg-secondary); border: 1px solid var(--border-color); border-radius: var(--radius-lg); padding: 1.25rem 1.5rem; box-shadow: 0 10px 30px rgba(0,0,0,0.08);">
                            <h3 style="margin: 0 0 0.65rem 0; font-size: 0.95rem; color: var(--text-primary);">
                                "Recent notes"
                            </h3>
                            <p style="margin: 0 0 0.9rem 0; font-size: 0.82rem; color: var(--text-muted);">
                                "Pick a recent note to continue where you left off."
                            </p>
                            <div style="display: flex; flex-direction: column; gap: 0.2rem; max-height: 260px; overflow-y: auto;">
                                {valid
                                    .into_iter()
                                    .map(|entry| {
                                        let filename = entry.path.clone();
                                        let name = entry.title.clone();
                                        let ts = entry.last_opened;
                                        let last_opened: String = if ts > 0 {
                                            let d = Date::new(&JsValue::from_f64(ts as f64));
                                            d.to_locale_string("default", &JsValue::UNDEFINED).into()
                                        } else {
                                            String::new()
                                        };
                                        let on_click = handler.clone();
                                        view! {
                                            <div
                                                style="padding: 0.4rem 0.55rem; border-radius: var(--radius-md); cursor: pointer; font-size: 0.85rem; display: flex; flex-direction: column; gap: 0.05rem; transition: background 0.15s ease;"
                                                on:click=move |_| on_click(filename.clone())
                                                title=entry.path.clone()
                                            >
                                                <div style="display: flex; align-items: baseline; gap: 0.35rem;">
                                                    <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                                        {name}
                                                    </span>
                                                    {move || if !last_opened.is_empty() {
                                                        view! {
                                                            <span style="margin-left: auto; font-size: 0.72rem; color: var(--text-muted);">
                                                                {format!("Last opened {}", last_opened)}
                                                            </span>
                                                        }
                                                        .into_any()
                                                    } else {
                                                        view! { <></> }.into_any()
                                                    }}
                                                </div>
                                                <span style="font-size: 0.74rem; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                                    {entry.path.clone()}
                                                </span>
                                            </div>
                                        }
                                    })
                                    .collect::<Vec<_>>()}
                            </div>
                        </div>
                    </div>
                }
                .into_any();
            }

            view! {
                <div style="flex: 1; display: flex; align-items: center; justify-content: center; color: var(--text-muted);">
                    "Select a note from the sidebar to start editing."
                </div>
            }
            .into_any()
        }}
    }
}

