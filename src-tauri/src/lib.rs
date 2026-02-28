use regex::{Captures, Regex};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(serde::Serialize)]
struct VaultNote {
    path: String,
    content: String,
}

fn normalize_rel_path(path: &str) -> String {
    path.trim().replace('\\', "/").trim_matches('/').to_string()
}

fn ensure_markdown_extension(path: &str) -> String {
    let normalized = normalize_rel_path(path);
    if normalized.to_ascii_lowercase().ends_with(".md") {
        normalized
    } else {
        format!("{normalized}.md")
    }
}

fn normalize_link_key(path: &str) -> String {
    normalize_rel_path(path).to_ascii_lowercase()
}

fn strip_md(path: &str) -> String {
    let normalized = normalize_rel_path(path);
    if normalized.to_ascii_lowercase().ends_with(".md") {
        normalized[..normalized.len() - 3].to_string()
    } else {
        normalized
    }
}

fn collect_markdown_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let read_dir = fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in read_dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if file_name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(root, &path, out)?;
            continue;
        }
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        out.push(rel);
    }
    Ok(())
}

fn collect_note_paths(vault_path: &str) -> Result<Vec<String>, String> {
    let root = Path::new(vault_path);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    collect_markdown_files(root, root, &mut entries)?;
    entries.sort();
    Ok(entries)
}

fn split_wikilink_inner(inner: &str) -> (String, Option<String>, Option<String>) {
    let (target_and_heading, alias) = match inner.split_once('|') {
        Some((left, right)) => (left.to_string(), Some(right.to_string())),
        None => (inner.to_string(), None),
    };
    let (target, heading) = match target_and_heading.split_once('#') {
        Some((left, right)) => (left.to_string(), Some(right.to_string())),
        None => (target_and_heading, None),
    };
    (target, heading, alias)
}

fn rewrite_wiki_links(
    content: &str,
    old_path: &str,
    new_path: &str,
    include_stem_match: bool,
) -> (String, bool) {
    let wiki_re = Regex::new(r"\[\[([^\]]+)\]\]").expect("valid wiki link regex");
    let old_no_ext = strip_md(old_path);
    let new_no_ext = strip_md(new_path);
    let old_stem = Path::new(&old_no_ext)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&old_no_ext)
        .to_string();
    let new_stem = Path::new(&new_no_ext)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&new_no_ext)
        .to_string();

    let mut old_refs = HashSet::new();
    old_refs.insert(normalize_link_key(old_path));
    old_refs.insert(normalize_link_key(&old_no_ext));
    if include_stem_match {
        old_refs.insert(normalize_link_key(&old_stem));
    }

    let mut changed = false;
    let rewritten = wiki_re.replace_all(content, |caps: &Captures| {
        let whole = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
        let inner = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let (target_raw, heading, alias) = split_wikilink_inner(inner);
        let target_trimmed = target_raw.trim();
        if target_trimmed.is_empty() {
            return whole.to_string();
        }
        let target_key = normalize_link_key(target_trimmed);
        if !old_refs.contains(&target_key) {
            return whole.to_string();
        }

        changed = true;
        let had_path = target_trimmed.contains('/');
        let had_ext = target_trimmed.to_ascii_lowercase().ends_with(".md");
        let replacement_base = if had_path {
            new_no_ext.clone()
        } else {
            new_stem.clone()
        };
        let mut rebuilt = if had_ext {
            format!("{replacement_base}.md")
        } else {
            replacement_base
        };
        if let Some(heading_part) = heading {
            if !heading_part.is_empty() {
                rebuilt.push('#');
                rebuilt.push_str(&heading_part);
            }
        }
        if let Some(alias_part) = alias {
            rebuilt.push('|');
            rebuilt.push_str(&alias_part);
        }
        format!("[[{rebuilt}]]")
    });
    (rewritten.into_owned(), changed)
}

#[tauri::command]
fn read_dir(path: &str) -> Result<Vec<String>, String> {
    collect_note_paths(path)
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
fn read_vault_notes(vault_path: &str) -> Result<Vec<VaultNote>, String> {
    let root = PathBuf::from(vault_path);
    let mut notes = Vec::new();
    for rel_path in collect_note_paths(vault_path)? {
        let abs = root.join(&rel_path);
        let content = fs::read_to_string(abs).unwrap_or_default();
        notes.push(VaultNote {
            path: rel_path,
            content,
        });
    }
    Ok(notes)
}

#[tauri::command]
fn rename_note(vault_path: &str, old_path: &str, new_path: &str) -> Result<String, String> {
    let root = Path::new(vault_path);
    let old_rel = ensure_markdown_extension(old_path);
    let new_rel = ensure_markdown_extension(new_path);
    if old_rel.is_empty() || new_rel.is_empty() {
        return Err("Note paths cannot be empty".to_string());
    }
    if old_rel == new_rel {
        return Ok(new_rel);
    }

    let old_abs = root.join(&old_rel);
    let new_abs = root.join(&new_rel);
    if !old_abs.exists() {
        return Err(format!("Note does not exist: {old_rel}"));
    }

    let old_stem = Path::new(&old_rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let stem_occurrences = collect_note_paths(vault_path)?
        .into_iter()
        .filter(|path| {
            Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|stem| stem.eq_ignore_ascii_case(&old_stem))
                .unwrap_or(false)
        })
        .count();
    let include_stem_match = stem_occurrences <= 1;

    if let Some(parent) = new_abs.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::rename(&old_abs, &new_abs).map_err(|e| e.to_string())?;

    for rel in collect_note_paths(vault_path)? {
        let abs = root.join(&rel);
        let content = fs::read_to_string(&abs).map_err(|e| e.to_string())?;
        let (rewritten, changed) =
            rewrite_wiki_links(&content, &old_rel, &new_rel, include_stem_match);
        if changed {
            fs::write(abs, rewritten).map_err(|e| e.to_string())?;
        }
    }
    Ok(new_rel)
}

#[tauri::command]
fn init_vault(app_handle: tauri::AppHandle) -> Result<String, String> {
    use tauri::Manager;
    let docs = app_handle
        .path()
        .document_dir()
        .map_err(|e| e.to_string())?;
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

    // Bedrock config space for future plugin/app state.
    let config_path = vault_path.join(".bedrock");
    if !config_path.exists() {
        fs::create_dir_all(&config_path).map_err(|e| e.to_string())?;
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
        WebviewWindowBuilder::new(
            &app,
            "settings",
            WebviewUrl::App("index.html?settings=true".into()),
        )
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
            read_vault_notes,
            rename_note,
            init_vault,
            load_plugins_css,
            save_settings,
            load_settings,
            open_settings_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
