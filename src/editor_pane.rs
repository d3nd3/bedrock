use leptos::prelude::*;

#[component]
pub fn EditorPane(children: Children) -> impl IntoView {
    view! {
        <section class="editor-pane" style="flex: 1; display: flex; flex-direction: column; background: var(--bg-primary); min-width: 0;">
            {children()}
        </section>
    }
}

