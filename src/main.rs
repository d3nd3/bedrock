mod app;
mod app_state;
mod editor_core;
mod markdown_syntax;
mod metadata_sidebar;
mod path_utils;
mod editor_pane;
mod sidebar_tree;
mod sidebar_panel;
mod tauri_bridge;
mod top_bar;
mod vault_tabs;
mod recent_notes_pane;

use app::*;
use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| {
        view! {
            <App/>
        }
    })
}
