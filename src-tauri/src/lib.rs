use std::fs;
use tauri::{AppHandle, Manager, WebviewWindowBuilder, WebviewUrl, Emitter};

#[tauri::command]
fn read_dir(path: &str) -> Result<Vec<String>, String> {
    let mut entries = Vec::new();
    let read_dir_result = fs::read_dir(path).map_err(|e| e.to_string())?;
    for entry in read_dir_result {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') && name != "settings.json" {
            entries.push(name);
        }
    }
    entries.sort();
    Ok(entries)
}

#[tauri::command]
fn read_file(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn write_file(path: &str, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|e| e.to_string())
}

#[tauri::command]
fn init_vault(app_handle: tauri::AppHandle) -> Result<String, String> {
    use tauri::Manager;
    let docs = app_handle.path().document_dir().map_err(|e| e.to_string())?;
    let vault_path = docs.join("BedrockVault");
    
    if !vault_path.exists() {
        fs::create_dir_all(&vault_path).map_err(|e| e.to_string())?;
        // Create an initial welcome file
        let welcome_path = vault_path.join("Welcome.md");
        fs::write(&welcome_path, "# Welcome to Bedrock\n\nBedrock is a fast, premium markdown note-taking tool.\n\n- Powered by **Rust** and **Tauri**\n- Extensible via CSS variables and plugins.\n").map_err(|e| e.to_string())?;
    }
    
    // Also init the plugins directory
    let plugins_path = vault_path.join(".plugins");
    if !plugins_path.exists() {
        fs::create_dir_all(&plugins_path).map_err(|e| e.to_string())?;
        let dummy_plugin = plugins_path.join("theme.css");
        fs::write(&dummy_plugin, "/* Put custom plugin CSS here to override default CSS variables */\n/* :root { --bg-primary: #000000; } */\n").map_err(|e| e.to_string())?;
    }
    
    // Also init settings
    let settings_path = vault_path.join("settings.json");
    if !settings_path.exists() {
        fs::write(&settings_path, "{}").map_err(|e| e.to_string())?;
    }
    
    Ok(vault_path.to_string_lossy().into_owned())
}

#[tauri::command]
fn load_plugins_css(vault_path: &str) -> Result<String, String> {
    let mut compiled_css = String::new();
    let plugins_dir = format!("{}/.plugins", vault_path);
    if let Ok(entries) = fs::read_dir(plugins_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|ext| ext == "css") {
                if let Ok(css_content) = fs::read_to_string(&p) {
                    compiled_css.push_str(&css_content);
                    compiled_css.push('\n');
                }
            }
        }
    }
    Ok(compiled_css)
}

#[tauri::command]
fn save_settings(app: AppHandle, vault_path: &str, settings: &str) -> Result<(), String> {
    let settings_path = format!("{}/settings.json", vault_path);
    fs::write(settings_path, settings).map_err(|e| e.to_string())?;
    let _ = app.emit("settings-updated", settings);
    Ok(())
}

#[tauri::command]
fn open_settings_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window.set_focus().unwrap();
    } else {
        WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("index.html?settings=true".into()))
            .title("Theme Settings")
            .inner_size(800.0, 700.0)
            .build()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn load_settings(vault_path: &str) -> Result<String, String> {
    let settings_path = format!("{}/settings.json", vault_path);
    fs::read_to_string(settings_path).or_else(|_| Ok("{}".to_string()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            read_dir,
            read_file,
            write_file,
            init_vault,
            load_plugins_css,
            save_settings,
            load_settings,
            open_settings_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
