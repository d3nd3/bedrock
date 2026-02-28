use base64::Engine;
use regex::{Captures, Regex};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

#[derive(serde::Serialize)]
struct VaultNote {
    path: String,
    content: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
struct VaultSessionState {
    open_vaults: Vec<String>,
    active_vault: Option<String>,
}

#[derive(serde::Serialize, Clone, Debug)]
struct VaultImportReport {
    success: bool,
    cancelled: bool,
    message: String,
    source_vault: Option<String>,
    destination_vault: Option<String>,
    scanned_notes: usize,
    imported_notes: usize,
    scanned_images: usize,
    imported_images: usize,
    renamed_notes: usize,
}

impl VaultImportReport {
    fn cancelled(message: impl Into<String>) -> Self {
        Self {
            success: false,
            cancelled: true,
            message: message.into(),
            source_vault: None,
            destination_vault: None,
            scanned_notes: 0,
            imported_notes: 0,
            scanned_images: 0,
            imported_images: 0,
            renamed_notes: 0,
        }
    }

    fn failed(
        message: impl Into<String>,
        source_vault: Option<String>,
        destination_vault: Option<String>,
    ) -> Self {
        Self {
            success: false,
            cancelled: false,
            message: message.into(),
            source_vault,
            destination_vault,
            scanned_notes: 0,
            imported_notes: 0,
            scanned_images: 0,
            imported_images: 0,
            renamed_notes: 0,
        }
    }
}

fn normalize_vault_session_state(state: VaultSessionState) -> VaultSessionState {
    let normalize_path = |raw: &str| -> Option<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let candidate = PathBuf::from(trimmed);
        if candidate.is_dir() {
            if let Ok(canon) = candidate.canonicalize() {
                return Some(canon.to_string_lossy().to_string());
            }
        }
        if candidate.is_absolute() {
            return Some(trimmed.to_string());
        }
        None
    };

    let mut open_vaults = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    for path in state.open_vaults {
        let Some(normalized) = normalize_path(&path) else {
            continue;
        };
        if seen.insert(normalized.clone()) {
            open_vaults.push(normalized);
        }
    }

    let mut active_vault = state.active_vault.and_then(|raw| normalize_path(&raw));

    if let Some(active) = active_vault.clone() {
        if let Some(existing) = open_vaults.iter().find(|p| *p == &active) {
            active_vault = Some(existing.clone());
        } else if let Some(first) = open_vaults.first().cloned() {
            active_vault = Some(first);
        } else {
            active_vault = Some(active);
        }
    } else if let Some(first) = open_vaults.first().cloned() {
        active_vault = Some(first);
    }

    VaultSessionState {
        open_vaults,
        active_vault,
    }
}

fn vault_session_state_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("vault-session.json"))
}

fn vault_session_fallback_path(app: &AppHandle) -> Result<PathBuf, String> {
    let docs = app.path().document_dir().map_err(|e| e.to_string())?;
    let default_vault = docs.join("BedrockVault");
    ensure_bedrock_layout(&default_vault)?;
    Ok(default_vault.join(".bedrock").join("vault-session.json"))
}

fn write_vault_session_to_path(path: &Path, state: &VaultSessionState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

fn read_vault_session_from_path(path: &Path) -> Option<VaultSessionState> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<VaultSessionState>(&raw).ok()
}

fn persist_vault_session_state(app: &AppHandle, state: &VaultSessionState) -> Result<(), String> {
    let mut wrote = false;
    let mut last_error = None::<String>;

    if let Ok(path) = vault_session_state_path(app) {
        match write_vault_session_to_path(&path, state) {
            Ok(_) => wrote = true,
            Err(err) => last_error = Some(err),
        }
    }

    if let Ok(path) = vault_session_fallback_path(app) {
        match write_vault_session_to_path(&path, state) {
            Ok(_) => wrote = true,
            Err(err) => last_error = Some(err),
        }
    }

    if wrote {
        Ok(())
    } else {
        Err(last_error.unwrap_or_else(|| "Unable to persist vault session state.".to_string()))
    }
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

fn collect_relative_dirs(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let read_dir = fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in read_dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() && !entry.file_name().to_string_lossy().starts_with('.') {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            out.push(rel);
            collect_relative_dirs(root, &path, out)?;
        }
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct ReadDirResult {
    notes: Vec<String>,
    empty_dirs: Vec<String>,
}

fn ensure_bedrock_layout(vault_path: &Path) -> Result<(), String> {
    if !vault_path.exists() {
        fs::create_dir_all(vault_path).map_err(|e| e.to_string())?;
    }

    let plugins_path = vault_path.join(".plugins");
    if !plugins_path.exists() {
        fs::create_dir_all(&plugins_path).map_err(|e| e.to_string())?;
    }
    let theme_css = plugins_path.join("theme.css");
    if !theme_css.exists() {
        fs::write(
            &theme_css,
            "/* Put custom plugin CSS here to override default CSS variables */\n/* :root { --bg-primary: #000000; } */\n",
        )
        .map_err(|e| e.to_string())?;
    }

    let config_path = vault_path.join(".bedrock");
    if !config_path.exists() {
        fs::create_dir_all(&config_path).map_err(|e| e.to_string())?;
    }

    let settings_path = vault_path.join("settings.json");
    if !settings_path.exists() {
        fs::write(&settings_path, "{}").map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

fn is_importable_image_extension(ext: &str) -> bool {
    matches!(
        ext,
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "svg"
            | "tif"
            | "tiff"
            | "ico"
            | "avif"
            | "heic"
            | "heif"
    )
}

fn is_importable_asset(path: &Path) -> bool {
    if is_markdown_file(path) {
        return true;
    }
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| is_importable_image_extension(&ext.to_ascii_lowercase()))
        .unwrap_or(false)
}

fn collect_importable_files_for_import(
    root: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let read_dir = fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in read_dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if file_name.starts_with('.') {
            continue;
        }

        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        if file_type.is_symlink() {
            continue;
        }

        let path = entry.path();
        if file_type.is_dir() {
            collect_importable_files_for_import(root, &path, out)?;
            continue;
        }

        if file_type.is_file() && is_importable_asset(&path) {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_path_buf();
            out.push(rel);
        }
    }

    Ok(())
}

fn unique_import_target_path(
    destination_root: &Path,
    rel_path: &Path,
) -> Result<(PathBuf, bool), String> {
    let direct_target = destination_root.join(rel_path);
    if !direct_target.exists() {
        return Ok((direct_target, false));
    }

    let parent = rel_path.parent().unwrap_or_else(|| Path::new(""));
    let stem = rel_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Imported note");
    let ext = rel_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("md");

    for idx in 1..=10_000usize {
        let filename = format!("{stem} (import {idx}).{ext}");
        let candidate = destination_root.join(parent).join(filename);
        if !candidate.exists() {
            return Ok((candidate, true));
        }
    }

    Err(format!(
        "Unable to find a unique destination filename for {}",
        rel_path.display()
    ))
}

fn import_obsidian_vault_notes(
    source_vault: &Path,
    destination_vault: &Path,
) -> Result<VaultImportReport, String> {
    if !source_vault.exists() || !source_vault.is_dir() {
        return Err("Source path is not a directory.".to_string());
    }
    if !source_vault.join(".obsidian").is_dir() {
        return Err(
            "Selected source is not an Obsidian vault (missing `.obsidian` folder).".to_string(),
        );
    }

    let source_canon = source_vault.canonicalize().map_err(|e| e.to_string())?;
    if !destination_vault.exists() {
        fs::create_dir_all(destination_vault).map_err(|e| e.to_string())?;
    }
    let destination_canon = destination_vault
        .canonicalize()
        .map_err(|e| e.to_string())?;

    if destination_canon == source_canon || destination_canon.starts_with(&source_canon) {
        return Err(
            "Destination vault must not be the same as, or inside, the source Obsidian vault."
                .to_string(),
        );
    }

    ensure_bedrock_layout(&destination_canon)?;

    let mut rel_import_files = Vec::<PathBuf>::new();
    collect_importable_files_for_import(&source_canon, &source_canon, &mut rel_import_files)?;
    rel_import_files.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));

    let mut imported_notes = 0usize;
    let mut imported_images = 0usize;
    let mut renamed_notes = 0usize;
    let scanned_notes = rel_import_files
        .iter()
        .filter(|path| is_markdown_file(path))
        .count();
    let scanned_images = rel_import_files.len().saturating_sub(scanned_notes);

    for rel in &rel_import_files {
        let source_file = source_canon.join(rel);
        let (destination_file, renamed) = unique_import_target_path(&destination_canon, rel)?;

        if let Some(parent) = destination_file.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let content = fs::read(&source_file).map_err(|e| e.to_string())?;
        fs::write(&destination_file, content).map_err(|e| e.to_string())?;

        if is_markdown_file(rel) {
            imported_notes += 1;
        } else {
            imported_images += 1;
        }
        if renamed {
            renamed_notes += 1;
        }
    }

    let source_display = source_canon.to_string_lossy().to_string();
    let destination_display = destination_canon.to_string_lossy().to_string();

    Ok(VaultImportReport {
        success: true,
        cancelled: false,
        message: format!(
            "Imported {imported_notes} notes and {imported_images} images from `{source_display}` into `{destination_display}`.{}",
            if renamed_notes > 0 {
                format!(" {renamed_notes} files were renamed to avoid overwriting existing notes.")
            } else {
                String::new()
            }
        ),
        source_vault: Some(source_display),
        destination_vault: Some(destination_display),
        scanned_notes,
        imported_notes,
        scanned_images,
        imported_images,
        renamed_notes,
    })
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn pick_folder(title: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title(title).pick_folder()
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn confirm_import(source_vault: &Path, destination_vault: &Path) -> bool {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

    let source = source_vault.to_string_lossy();
    let destination = destination_vault.to_string_lossy();
    let description = format!(
        "Copy notes and images from:\n{source}\n\nto destination:\n{destination}\n\nThe source Obsidian vault will not be modified."
    );

    matches!(
        MessageDialog::new()
            .set_level(MessageLevel::Info)
            .set_title("Confirm Vault Import")
            .set_description(&description)
            .set_buttons(MessageButtons::YesNo)
            .show(),
        MessageDialogResult::Yes
    )
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
fn read_dir(path: &str) -> Result<ReadDirResult, String> {
    let root = Path::new(path);
    if !root.exists() {
        return Ok(ReadDirResult { notes: Vec::new(), empty_dirs: Vec::new() });
    }
    let notes = collect_note_paths(path)?;
    let mut all_dirs = Vec::new();
    collect_relative_dirs(root, root, &mut all_dirs)?;
    let empty_dirs = all_dirs
        .into_iter()
        .filter(|d| {
            !notes.iter().any(|n| n == d || n.starts_with(&format!("{d}/")))
        })
        .collect();
    Ok(ReadDirResult { notes, empty_dirs })
}

#[tauri::command]
fn read_file(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn read_file_base64(path: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[tauri::command]
fn write_file(path: &str, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_dir(path: &str) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_file(path: &str) -> Result<(), String> {
    fs::remove_file(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_dir(path: &str) -> Result<(), String> {
    fs::remove_dir_all(path).map_err(|e| e.to_string())
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

    let needs_welcome = !vault_path.exists();
    ensure_bedrock_layout(&vault_path)?;
    if needs_welcome {
        // Create an initial welcome file
        let welcome_path = vault_path.join("Welcome.md");
        fs::write(&welcome_path, "# Welcome to Bedrock\n\nBedrock is a fast, premium markdown note-taking tool.\n\n- Powered by **Rust** and **Tauri**\n- Extensible via CSS variables and plugins.\n").map_err(|e| e.to_string())?;
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

fn recent_notes_path(vault_path: &str) -> PathBuf {
    Path::new(vault_path).join(".bedrock").join("recent.json")
}

#[derive(serde::Deserialize)]
struct SaveRecentNotesPayload {
    vault_path: String,
    paths: Vec<String>,
}

#[tauri::command]
fn read_recent_notes(vault_path: &str) -> Vec<String> {
    let path = recent_notes_path(vault_path);
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default()
}

#[tauri::command]
fn save_recent_notes(payload: SaveRecentNotesPayload) -> Result<(), String> {
    let dir = Path::new(&payload.vault_path).join(".bedrock");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("recent.json");
    let json = serde_json::to_string(&payload.paths).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

#[tauri::command]
fn import_obsidian_vault_with_picker() -> VaultImportReport {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        return VaultImportReport::failed(
            "Vault import via folder picker is currently supported on desktop builds only.",
            None,
            None,
        );
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let Some(source_vault) = pick_folder("Choose source Obsidian vault (read-only)") else {
            return VaultImportReport::cancelled(
                "Import cancelled. No source Obsidian vault selected.",
            );
        };

        let Some(destination_vault) = pick_folder("Choose destination Bedrock vault") else {
            return VaultImportReport::cancelled(
                "Import cancelled. No destination Bedrock vault selected.",
            );
        };

        if !confirm_import(&source_vault, &destination_vault) {
            return VaultImportReport::cancelled(
                "Import cancelled. Confirmation was not accepted.",
            );
        }

        match import_obsidian_vault_notes(&source_vault, &destination_vault) {
            Ok(report) => report,
            Err(err) => VaultImportReport::failed(
                err,
                Some(source_vault.to_string_lossy().to_string()),
                Some(destination_vault.to_string_lossy().to_string()),
            ),
        }
    }
}

#[tauri::command]
fn pick_bedrock_vault() -> Result<Option<String>, String> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        Err("Opening a vault with a native folder picker is desktop-only for now.".to_string())
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let Some(path) = pick_folder("Choose Bedrock vault") else {
            return Ok(None);
        };
        ensure_bedrock_layout(&path)?;
        let canon = path.canonicalize().map_err(|e| e.to_string())?;
        Ok(Some(canon.to_string_lossy().to_string()))
    }
}

#[tauri::command]
fn load_vault_session(app: AppHandle) -> Result<VaultSessionState, String> {
    let mut parsed = None::<VaultSessionState>;

    if let Ok(path) = vault_session_state_path(&app) {
        if path.exists() {
            parsed = read_vault_session_from_path(&path);
        }
    }
    if parsed.is_none() {
        if let Ok(path) = vault_session_fallback_path(&app) {
            if path.exists() {
                parsed = read_vault_session_from_path(&path);
            }
        }
    }

    let parsed = parsed.unwrap_or_default();
    let normalized = normalize_vault_session_state(parsed);
    persist_vault_session_state(&app, &normalized)?;
    Ok(normalized)
}

#[tauri::command]
fn save_vault_session(
    app: AppHandle,
    open_vaults: Vec<String>,
    active_vault: Option<String>,
) -> Result<VaultSessionState, String> {
    let normalized = normalize_vault_session_state(VaultSessionState {
        open_vaults,
        active_vault,
    });
    persist_vault_session_state(&app, &normalized)?;
    Ok(normalized)
}

#[cfg(test)]
mod import_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let pid = std::process::id();
        std::env::temp_dir().join(format!("bedrock-{prefix}-{pid}-{nanos}"))
    }

    #[test]
    fn imports_markdown_files_without_mutating_source() {
        let source = unique_temp_dir("obsidian-source");
        let destination = unique_temp_dir("bedrock-destination");
        fs::create_dir_all(source.join(".obsidian")).unwrap();
        fs::create_dir_all(source.join("notes/nested")).unwrap();
        fs::create_dir_all(source.join("Assets")).unwrap();
        fs::write(source.join("notes/nested/One.md"), "# One\n").unwrap();
        fs::write(source.join("notes/nested/Two.md"), "# Two\n").unwrap();
        fs::write(source.join("root.png"), b"png-bytes").unwrap();
        fs::write(source.join("Assets/photo.jpg"), b"jpg-bytes").unwrap();

        let source_before = fs::read_to_string(source.join("notes/nested/One.md")).unwrap();
        let source_png_before = fs::read(source.join("root.png")).unwrap();
        let report = import_obsidian_vault_notes(&source, &destination).unwrap();
        let source_after = fs::read_to_string(source.join("notes/nested/One.md")).unwrap();
        let source_png_after = fs::read(source.join("root.png")).unwrap();

        assert!(report.success);
        assert_eq!(report.imported_notes, 2);
        assert_eq!(report.imported_images, 2);
        assert_eq!(report.scanned_notes, 2);
        assert_eq!(report.scanned_images, 2);
        assert_eq!(source_before, source_after);
        assert_eq!(source_png_before, source_png_after);
        assert!(destination.join("notes/nested/One.md").exists());
        assert!(destination.join("notes/nested/Two.md").exists());
        assert!(destination.join("root.png").exists());
        assert!(destination.join("Assets/photo.jpg").exists());

        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(destination);
    }

    #[test]
    fn rejects_destination_inside_source() {
        let source = unique_temp_dir("obsidian-source-nested");
        let destination = source.join("imports/bedrock");
        fs::create_dir_all(source.join(".obsidian")).unwrap();
        fs::write(source.join("Note.md"), "A").unwrap();

        let err = import_obsidian_vault_notes(&source, &destination).unwrap_err();
        assert!(err.contains("inside"));

        let _ = fs::remove_dir_all(source);
    }

    #[test]
    fn renames_conflicting_destination_files() {
        let source = unique_temp_dir("obsidian-source-conflict");
        let destination = unique_temp_dir("bedrock-destination-conflict");
        fs::create_dir_all(source.join(".obsidian")).unwrap();
        fs::create_dir_all(source.join("folder")).unwrap();
        fs::create_dir_all(destination.join("folder")).unwrap();

        fs::write(source.join("folder/Note.md"), "from source").unwrap();
        fs::write(destination.join("folder/Note.md"), "existing").unwrap();

        let report = import_obsidian_vault_notes(&source, &destination).unwrap();
        assert_eq!(report.imported_notes, 1);
        assert_eq!(report.renamed_notes, 1);

        let renamed_path = destination.join("folder/Note (import 1).md");
        assert!(renamed_path.exists());
        assert_eq!(fs::read_to_string(renamed_path).unwrap(), "from source");
        assert_eq!(
            fs::read_to_string(destination.join("folder/Note.md")).unwrap(),
            "existing"
        );

        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(destination);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|_| Ok(()))
        .invoke_handler(tauri::generate_handler![
            read_dir,
            read_file,
            read_file_base64,
            write_file,
            create_dir,
            delete_file,
            delete_dir,
            read_vault_notes,
            rename_note,
            init_vault,
            load_plugins_css,
            save_settings,
            load_settings,
            open_settings_window,
            import_obsidian_vault_with_picker,
            pick_bedrock_vault,
            load_vault_session,
            save_vault_session,
            read_recent_notes,
            save_recent_notes,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, e| {
            #[cfg(target_os = "linux")]
            if let RunEvent::Ready = e {
                let icon_bytes = include_bytes!("../icons/32x32.png");
                if let (Some(w), Ok(icon)) = (
                    app.get_webview_window("main"),
                    tauri::image::Image::from_bytes(icon_bytes),
                ) {
                    let _ = w.set_icon(icon);
                }
            }
        });
}
