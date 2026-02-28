use leptos::task::spawn_local;

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use regex::Regex;
use std::sync::OnceLock;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

#[derive(Serialize)]
struct ReadDirArgs<'a> { path: &'a str }
#[derive(Serialize)]
struct ReadFileArgs<'a> { path: &'a str }
#[derive(Serialize)]
struct WriteFileArgs<'a> { path: &'a str, content: &'a str }
#[derive(Serialize)]
struct SaveSettingsArgs<'a> { vault_path: &'a str, settings: &'a str }
#[derive(Serialize)]
struct VaultPathArgs<'a> { vault_path: &'a str }

#[derive(Serialize, Deserialize, Clone, Debug)]
struct AppSettings {
    font_size: u32,
    accent_color: String,
    bg_primary: String,
    bg_secondary: String,
    text_primary: String,
    md_h1_color: String,
    md_h2_color: String,
    md_h3_color: String,
    md_h4_color: String,
    md_bold_color: String,
    md_italic_color: String,
    md_code_bg: String,
    md_code_text: String,
    md_quote_color: String,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self { 
            font_size: 16, 
            accent_color: "#6366f1".to_string(), 
            bg_primary: "#ffffff".to_string(),
            bg_secondary: "#f4f5f7".to_string(),
            text_primary: "#1a1a1a".to_string(),
            md_h1_color: "#1a1a1a".to_string(),
            md_h2_color: "#1a1a1a".to_string(),
            md_h3_color: "#1a1a1a".to_string(),
            md_h4_color: "#1a1a1a".to_string(),
            md_bold_color: "#4f46e5".to_string(),
            md_italic_color: "#1a1a1a".to_string(),
            md_code_bg: "#e9ecef".to_string(),
            md_code_text: "#1a1a1a".to_string(),
            md_quote_color: "#9ca3af".to_string(),
        }
    }
}

fn highlight_markdown(text: &str) -> String {
    static RE_H1: OnceLock<Regex> = OnceLock::new();
    static RE_H2: OnceLock<Regex> = OnceLock::new();
    static RE_H3: OnceLock<Regex> = OnceLock::new();
    static RE_H4: OnceLock<Regex> = OnceLock::new();
    static RE_BOLD: OnceLock<Regex> = OnceLock::new();
    static RE_ITALIC: OnceLock<Regex> = OnceLock::new();
    static RE_CODE: OnceLock<Regex> = OnceLock::new();
    static RE_QUOTE: OnceLock<Regex> = OnceLock::new();

    let re_h1 = RE_H1.get_or_init(|| Regex::new(r"(?m)^(#[^\S\n]+.*)$").unwrap());
    let re_h2 = RE_H2.get_or_init(|| Regex::new(r"(?m)^(##[^\S\n]+.*)$").unwrap());
    let re_h3 = RE_H3.get_or_init(|| Regex::new(r"(?m)^(###[^\S\n]+.*)$").unwrap());
    let re_h4 = RE_H4.get_or_init(|| Regex::new(r"(?m)^(####[^\S\n]+.*)$").unwrap());
    let re_bold = RE_BOLD.get_or_init(|| Regex::new(r"\*\*(.+?)\*\*").unwrap());
    let re_italic = RE_ITALIC.get_or_init(|| Regex::new(r"\*([^*]+)\*").unwrap());
    let re_code = RE_CODE.get_or_init(|| Regex::new(r"`([^`]+)`").unwrap());
    let re_quote = RE_QUOTE.get_or_init(|| Regex::new(r"(?m)^(&gt;.*)$").unwrap());

    let mut html = text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;");
    
    html = re_h1.replace_all(&html, "<span class=\"hl-h1\">$1</span>").to_string();
    html = re_h2.replace_all(&html, "<span class=\"hl-h2\">$1</span>").to_string();
    html = re_h3.replace_all(&html, "<span class=\"hl-h3\">$1</span>").to_string();
    html = re_h4.replace_all(&html, "<span class=\"hl-h4\">$1</span>").to_string();
    html = re_bold.replace_all(&html, "<span class=\"hl-bold\">$1</span>").to_string();
    html = re_italic.replace_all(&html, "<span class=\"hl-italic\">$1</span>").to_string();
    html = re_code.replace_all(&html, "<span class=\"hl-code\">$1</span>").to_string();
    html = re_quote.replace_all(&html, "<span class=\"hl-quote\">$1</span>").to_string();
    
    // Add an extra newline to handle trailing returns properly matching textarea height
    html.push_str("\n ");
    html
}

#[component]
pub fn App() -> impl IntoView {
    let (vault_path, set_vault_path) = signal(String::new());
    let (files, set_files) = signal(Vec::<String>::new());
    let (current_file, set_current_file) = signal(String::new());
    let (content, set_content) = signal(String::new());
    let (parsed_html, set_parsed_html) = signal(String::new());
    let (plugin_css, set_plugin_css) = signal(String::new());
    
    // Editor sync signals
    let (scroll_top, set_scroll_top) = signal(0);
    
    let (settings, set_settings) = signal(AppSettings::default());

    let closure = Closure::<dyn FnMut(leptos::web_sys::CustomEvent)>::new(move |e: leptos::web_sys::CustomEvent| {
        if let Some(detail) = e.detail().as_string() {
            if let Ok(s) = serde_json::from_str::<AppSettings>(&detail) {
                set_settings.set(s);
            }
        }
    });
    let _ = window().add_event_listener_with_callback("bedrock-settings", closure.as_ref().unchecked_ref());
    closure.forget();

    let is_settings_window = window().location().search().unwrap_or_default().contains("settings=true");

    Effect::new(move |_| {
        spawn_local(async move {
            let path_val = invoke("init_vault", JsValue::NULL).await;
            if let Some(path_str) = path_val.as_string() {
                set_vault_path.set(path_str.clone());
                let dir_args = serde_wasm_bindgen::to_value(&ReadDirArgs { path: &path_str }).unwrap();
                let vault_args = serde_wasm_bindgen::to_value(&VaultPathArgs { vault_path: &path_str }).unwrap();
                let dir_val = invoke("read_dir", dir_args).await;
                if let Ok(dir_list) = serde_wasm_bindgen::from_value::<Vec<String>>(dir_val) {
                    set_files.set(dir_list);
                }
                let css_val = invoke("load_plugins_css", vault_args.clone()).await;
                if let Some(css_str) = css_val.as_string() {
                    set_plugin_css.set(css_str);
                }
                let s_val = invoke("load_settings", vault_args).await;
                if let Some(s_str) = s_val.as_string() {
                    if let Ok(s) = serde_json::from_str::<AppSettings>(&s_str) {
                         set_settings.set(s);
                    }
                }
            }
        });
    });

    let select_file = move |filename: String| {
        spawn_local(async move {
            let v_path = vault_path.get_untracked();
            let file_path = format!("{}/{}", v_path, filename);
            let args = serde_wasm_bindgen::to_value(&ReadFileArgs { path: &file_path }).unwrap();
            let text_val = invoke("read_file", args).await;
            if let Some(text) = text_val.as_string() {
                set_content.set(text.clone());
                set_parsed_html.set(highlight_markdown(&text));
                set_current_file.set(filename);
            }
        });
    };

    let update_content = move |ev| {
        let new_text = event_target_value(&ev);
        set_content.set(new_text.clone());
        set_parsed_html.set(highlight_markdown(&new_text));
        
        let filename = current_file.get_untracked();
        if !filename.is_empty() {
             let v_path = vault_path.get_untracked();
             let file_path = format!("{}/{}", v_path, filename);
             spawn_local(async move {
                 let args = serde_wasm_bindgen::to_value(&WriteFileArgs { path: &file_path, content: &new_text }).unwrap();
                 invoke("write_file", args).await;
             });
        }
    };

    let save_settings_to_disk = move |s: AppSettings| {
        let v_path = vault_path.get_untracked();
        if !v_path.is_empty() {
            let s_json = serde_json::to_string(&s).unwrap();
            spawn_local(async move {
                 let args = serde_wasm_bindgen::to_value(&SaveSettingsArgs { vault_path: &v_path, settings: &s_json }).unwrap();
                 invoke("save_settings", args).await;
            });
        }
    };

    let create_new_note = move || {
        let v_path = vault_path.get_untracked();
        if v_path.is_empty() { return; }
        if let Ok(Some(raw)) = window().prompt_with_message("New note name") {
            let name = raw.trim();
            if name.is_empty() { return; }
            let mut filename = name.to_string();
            if !filename.ends_with(".md") { filename.push_str(".md"); }
            let file_path = format!("{}/{}", v_path, filename);
            let initial = "# New Note\n\n".to_string();
            let v_path_clone = v_path.clone();
            let filename_clone = filename.clone();
            let initial_clone = initial.clone();
            spawn_local(async move {
                let args = serde_wasm_bindgen::to_value(&WriteFileArgs { path: &file_path, content: &initial }).unwrap();
                invoke("write_file", args).await;
                let dir_args = serde_wasm_bindgen::to_value(&ReadDirArgs { path: &v_path_clone }).unwrap();
                let dir_val = invoke("read_dir", dir_args).await;
                if let Ok(dir_list) = serde_wasm_bindgen::from_value::<Vec<String>>(dir_val) {
                    set_files.set(dir_list);
                }
                set_current_file.set(filename_clone);
                set_content.set(initial_clone.clone());
                set_parsed_html.set(highlight_markdown(&initial_clone));
            });
        }
    };

    let dynamic_style = move || {
        let s = settings.get();
        format!("--editor-font-size: {}px; --accent-color: {}; --bg-primary: {}; --bg-secondary: {}; --text-primary: {}; --md-h1-color: {}; --md-h2-color: {}; --md-h3-color: {}; --md-h4-color: {}; --md-bold-color: {}; --md-italic-color: {}; --md-code-bg: {}; --md-code-text: {}; --md-quote-color: {};", 
        s.font_size, s.accent_color, s.bg_primary, s.bg_secondary, s.text_primary, s.md_h1_color, s.md_h2_color, s.md_h3_color, s.md_h4_color, s.md_bold_color, s.md_italic_color, s.md_code_bg, s.md_code_text, s.md_quote_color)
    };

    let app_view = move || {
        if is_settings_window {
            view! {
                <div style="flex: 1; padding: 3rem; overflow-y: auto;">
                    <h2 style="margin-top: 0; margin-bottom: 2rem;">"Theme Settings"</h2>
                            
                    <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 1.5rem;">
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label style="font-weight: 600; font-size: 0.9em;">"Editor Font Size (px)"</label>
                            <input style="padding: 0.5rem; border-radius: 4px; border: 1px solid var(--border-color); background: var(--bg-secondary); color: var(--text-primary); width: 100%; box-sizing: border-box;" type="number" prop:value=move || settings.get().font_size.to_string() on:input=move |e| {
                                let mut s = settings.get_untracked(); s.font_size = event_target_value(&e).parse().unwrap_or(16); set_settings.set(s.clone()); save_settings_to_disk(s);
                            } />
                        </div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label style="font-weight: 600; font-size: 0.9em;">"Accent Color"</label>
                            <input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().accent_color.clone() on:input=move |e| {
                                let mut s = settings.get_untracked(); s.accent_color = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s);
                            } />
                        </div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label style="font-weight: 600; font-size: 0.9em;">"Background Primary"</label>
                            <input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().bg_primary.clone() on:input=move |e| {
                                let mut s = settings.get_untracked(); s.bg_primary = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s);
                            } />
                        </div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label style="font-weight: 600; font-size: 0.9em;">"Background Secondary"</label>
                            <input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().bg_secondary.clone() on:input=move |e| {
                                let mut s = settings.get_untracked(); s.bg_secondary = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s);
                            } />
                        </div>
                        <div style="display: flex; flex-direction: column; gap: 0.5rem;">
                            <label style="font-weight: 600; font-size: 0.9em;">"Text Primary"</label>
                            <input style="padding: 0; border: none; border-radius: 4px; height: 35px; width: 100%; cursor: pointer;" type="color" prop:value=move || settings.get().text_primary.clone() on:input=move |e| {
                                let mut s = settings.get_untracked(); s.text_primary = event_target_value(&e); set_settings.set(s.clone()); save_settings_to_disk(s);
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
            }.into_any()
        } else {
            view! {
                <nav class="sidebar" style="width: var(--sidebar-width); border-right: 1px solid var(--border-color); display: flex; flex-direction: column; background: var(--bg-secondary); transition: all 0.3s ease;">
                    <div class="sidebar-header" style="height: var(--topbar-height); display: flex; align-items: center; justify-content: space-between; padding: 0 1rem; border-bottom: 1px solid var(--border-color); font-weight: 600; color: var(--accent-color);">
                        <span>"Bedrock"</span>
                        <div style="display: flex; gap: 0.5rem; align-items: center;">
                            <button
                                on:click=move |_| create_new_note()
                                style="background: transparent; border: none; font-size: 1.2rem; cursor: pointer; color: var(--text-muted);"
                                title="New note"
                            >
                                "+"
                            </button>
                            <button 
                                on:click=move |_| {
                                    spawn_local(async move {
                                        invoke("open_settings_window", JsValue::NULL).await;
                                    });
                                }
                                style="background: transparent; border: none; font-size: 1.2rem; cursor: pointer; color: var(--text-muted);"
                                title="Settings"
                            >
                                "⚙"
                            </button>
                        </div>
                    </div>
                    <div class="file-list" style="flex: 1; overflow-y: auto; padding: 0.75rem 0.5rem;">
                        {move || files.get().into_iter().map(|f| {
                            let filename = f.clone();
                            let f_clone = f.clone();
                            let is_active = move || current_file.get() == f_clone;
                            
                            view! {
                                <div 
                                    class="file-item" 
                                    style=move || format!("padding: 0.5rem 0.75rem; cursor: pointer; border-radius: var(--radius-md); margin-bottom: 4px; font-size: 0.9rem; transition: background 0.2s, color 0.2s; {}", if is_active() { "background: var(--accent-color); color: white;" } else { "color: var(--text-secondary);" })
                                    on:click=move |_| select_file(filename.clone())
                                >
                                    {f}
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                </nav>
                <section class="editor-pane" style="flex: 1; display: flex; flex-direction: column; background: var(--bg-primary);">
                    {move || if current_file.get().is_empty() {
                        view! {
                            <div style="flex: 1; display: flex; align-items: center; justify-content: center; color: var(--text-muted);">
                                "Select a note from the sidebar to start editing."
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <header class="topbar" style="height: var(--topbar-height); border-bottom: 1px solid var(--border-color); display: flex; align-items: center; padding: 0 1.5rem; color: var(--text-muted); font-size: 0.9rem;">
                                {move || current_file.get()}
                            </header>
                            <div class="editor-container" style="flex: 1; position: relative; overflow: hidden; background: var(--bg-primary);">
                                <div 
                                    class="markdown-highlight-layer"
                                    style="position: absolute; top: 0; left: 0; width: 100%; height: 100%; padding: 2rem 3rem; font-family: var(--font-editor); font-size: var(--editor-font-size); line-height: 1.6; color: var(--text-primary); white-space: pre-wrap; word-wrap: break-word; pointer-events: none; box-sizing: border-box; overflow-y: hidden;"
                                    inner_html=move || parsed_html.get()
                                    prop:scrollTop=move || scroll_top.get()
                                ></div>
                                <textarea 
                                    class="raw-editor"
                                    style="position: absolute; top: 0; left: 0; width: 100%; height: 100%; padding: 2rem 3rem; font-family: var(--font-editor); font-size: var(--editor-font-size); line-height: 1.6; color: transparent; background: transparent; caret-color: var(--text-primary); outline: none; border: none; resize: none; box-sizing: border-box; overflow-y: auto;"
                                    prop:value=move || content.get()
                                    on:input=update_content
                                    on:scroll=move |e| {
                                        let target: leptos::web_sys::Element = event_target(&e);
                                        set_scroll_top.set(target.scroll_top());
                                    }
                                    placeholder="Start writing markdown..."
                                    spellcheck="false"
                                ></textarea>
                            </div>
                        }.into_any()
                    }}
                </section>
            }.into_any()
        }
    };

    view! {
        <style>{ move || plugin_css.get() }</style>
        <main class="app-layout" style=move || format!("display: flex; height: 100vh; width: 100vw; background: var(--bg-primary); color: var(--text-primary); {}", dynamic_style())>
            {app_view}
        </main>
    }
}
