use base64::Engine;
use regex::{Captures, Regex};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder};

mod session;

use crate::session::{PendingClose, RecentNotesCache};

pub use crate::session::{
    cache_recent_notes, close_window_now, load_vault_session, read_recent_notes,
    save_recent_notes, save_vault_session,
};

#[derive(serde::Serialize)]
struct VaultNote {
    path: String,
    content: String,
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
    imported_note_colors: usize,
    /// When note colors were imported, the full path→hex map so the frontend can show colors on first paint.
    #[serde(skip_serializing_if = "Option::is_none")]
    imported_note_colors_map: Option<HashMap<String, String>>,
    /// When data.json was copied, the parsed palette (id, name, value) for the "Label color" menu.
    #[serde(skip_serializing_if = "Option::is_none")]
    imported_note_color_palette: Option<Vec<NoteColorPaletteEntry>>,
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
            imported_note_colors: 0,
            imported_note_colors_map: None,
            imported_note_color_palette: None,
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
            imported_note_colors: 0,
            imported_note_colors_map: None,
            imported_note_color_palette: None,
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
struct ObsidianFileColorPaletteEntry {
    id: String,
    #[allow(dead_code)]
    name: String,
    /// Hex color; plugin may store as "color" or "value" in data.json. We serialize as "value" for Obsidian.
    #[serde(rename = "value", alias = "color")]
    color: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
struct ObsidianFileColorFileEntry {
    path: String,
    color: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
struct ObsidianFileColorData {
    #[serde(default, rename = "cascadeColors")]
    cascade_colors: bool,
    #[serde(default, rename = "colorBackground", alias = "backgroundColors")]
    color_background: bool,
    #[serde(default)]
    palette: Vec<ObsidianFileColorPaletteEntry>,
    #[serde(default, rename = "fileColors")]
    file_colors: Vec<ObsidianFileColorFileEntry>,
}

/// Palette entry exposed to the frontend for "Label color" menu (id, name, hex value).
#[derive(serde::Serialize, Clone, Debug)]
struct NoteColorPaletteEntry {
    id: String,
    name: String,
    value: String,
}

/// Return type of load_note_colors so the frontend can compare canonical paths before applying.
#[derive(serde::Serialize, Clone, Debug)]
struct LoadNoteColorsResult {
    canonical_vault_path: String,
    colors: HashMap<String, String>,
}

fn normalize_rel_path(path: &str) -> String {
    path.trim().replace('\\', "/").trim_matches('/').to_string()
}

/// Canonical key for note-colors map so that "MentalHealth" (Obsidian) and "Mental-health" (filesystem) match.
/// Each segment is lowercased and has hyphens/underscores removed.
fn canonical_note_color_key(path: &str) -> String {
    let normalized = normalize_rel_path(path);
    normalized
        .split('/')
        .map(|s| {
            s.to_ascii_lowercase()
                .replace('-', "")
                .replace('_', "")
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Build path -> hex map from obsidian-file-color data (palette + fileColors).
/// Used by import and by load_note_colors fallback when reading .bedrock/obsidian-file-color/data.json.
/// Keys use canonical_note_color_key so that Obsidian paths (e.g. "MentalHealth") match filesystem paths (e.g. "Mental-health").
/// If a fileColor entry's color is not in the palette, we treat it as a hex value if it looks like one (#rgb, #rrggbb, etc.).
fn obsidian_file_color_to_map(data: &ObsidianFileColorData) -> HashMap<String, String> {
    let mut palette_by_id: HashMap<String, String> = HashMap::new();
    for entry in &data.palette {
        if !entry.color.trim().is_empty() {
            let hex = entry.color.trim().to_ascii_lowercase();
            palette_by_id.insert(entry.id.clone(), hex);
        }
    }
    let mut out = HashMap::new();
    for entry in &data.file_colors {
        let hex = palette_by_id
            .get(&entry.color)
            .cloned()
            .or_else(|| {
                let s = entry.color.trim();
                if s.starts_with('#')
                    && (s.len() == 4 || s.len() == 5 || s.len() == 7 || s.len() == 9)
                    && s[1..].chars().all(|c| c.is_ascii_hexdigit())
                {
                    Some(s.to_ascii_lowercase())
                } else {
                    None
                }
            });
        if let Some(hex) = hex {
            let raw_path = normalize_rel_path(&entry.path);
            let key = canonical_note_color_key(&raw_path);
            let is_md = key.ends_with(".md");
            out.insert(key.clone(), hex.clone());
            if !is_md {
                out.insert(format!("{key}.md"), hex);
            }
        }
    }
    out
}

/// Returns true if the vault directory contains `.obsidian` (hybrid vault: Obsidian + Bedrock).
fn is_hybrid_vault(root: &Path) -> bool {
    root.join(".obsidian").is_dir()
}

/// Load existing obsidian-file-color data from .bedrock or .obsidian for palette/options.
/// For hybrid vaults, prefers the source that has non-empty fileColors so we don't show no colours
/// when Obsidian's data.json exists but is empty and .bedrock has the real data.
fn load_obsidian_file_color_data(root: &Path) -> Option<ObsidianFileColorData> {
    // For hybrid vaults (both `.bedrock` and `.obsidian` present), prefer the Obsidian plugin
    // data as the source of truth so changes made in Obsidian are picked up by Bedrock. For
    // Bedrock-only vaults that may have a copied data.json under `.bedrock/obsidian-file-color`,
    // prefer that path.
    let mut paths = Vec::new();
    if is_hybrid_vault(root) {
        paths.push(
            root.join(".obsidian")
                .join("plugins")
                .join("obsidian-file-color")
                .join("data.json"),
        );
        paths.push(
            root.join(".bedrock")
                .join("obsidian-file-color")
                .join("data.json"),
        );
    } else {
        paths.push(
            root.join(".bedrock")
                .join("obsidian-file-color")
                .join("data.json"),
        );
        paths.push(
            root.join(".obsidian")
                .join("plugins")
                .join("obsidian-file-color")
                .join("data.json"),
        );
    }

    let mut best: Option<ObsidianFileColorData> = None;
    for p in paths {
        let rel_display = p.display().to_string();
        if p.is_file() {
            if let Ok(raw) = fs::read_to_string(&p) {
                if let Ok(data) = serde_json::from_str::<ObsidianFileColorData>(&raw) {
                    let n = data.file_colors.len();
                    eprintln!(
                        "[Bedrock] load_obsidian_file_color_data: {} — {} fileColor(s)",
                        rel_display,
                        n
                    );
                    let replace = best.as_ref().map(|b| b.file_colors.len() < n).unwrap_or(true);
                    if replace {
                        best = Some(data);
                    }
                } else {
                    eprintln!("[Bedrock] load_obsidian_file_color_data: {} — parse failed", rel_display);
                }
            } else {
                eprintln!("[Bedrock] load_obsidian_file_color_data: {} — read failed", rel_display);
            }
        } else {
            eprintln!("[Bedrock] load_obsidian_file_color_data: {} — not a file (missing)", rel_display);
        }
    }
    best
}

/// Build Obsidian-format data from our path→hex map for syncing to
/// `.obsidian/plugins/obsidian-file-color/data.json`.
///
/// - Uses the existing palette when present and adds new palette entries for unknown hex values.
/// - Attempts to map our canonical keys back to real filesystem/plugin paths so Obsidian can
///   match `file.path === settings.fileColors[].path` in its explorer.
fn build_obsidian_file_color_data(
    root: &Path,
    colors: &HashMap<String, String>,
    existing: Option<&ObsidianFileColorData>,
) -> ObsidianFileColorData {
    let (mut palette, cascade_colors, color_background) = if let Some(ex) = existing {
        (
            ex.palette.clone(),
            ex.cascade_colors,
            ex.color_background,
        )
    } else {
        (Vec::<ObsidianFileColorPaletteEntry>::new(), false, false)
    };
    let mut hex_to_id: HashMap<String, String> = palette
        .iter()
        .filter(|e| !e.color.trim().is_empty())
        .map(|e| (e.color.trim().to_ascii_lowercase(), e.id.clone()))
        .collect();
    let mut next_id = 0u64;

    // Index of canonical path key -> one or more concrete path strings. We prefer:
    // 1) Paths from the existing plugin data (so we preserve Obsidian's exact casing),
    // 2) Paths from the filesystem (dirs + .md files),
    // 3) Falling back to the raw key from `colors` if nothing else matches.
    let mut path_index: HashMap<String, Vec<String>> = HashMap::new();

    if let Some(ex) = existing {
        for entry in &ex.file_colors {
            let key = canonical_note_color_key(&entry.path);
            path_index
                .entry(key)
                .or_default()
                .push(entry.path.clone());
        }
    }

    // Collect directories relative to the vault root.
    let mut dirs = Vec::<String>::new();
    if collect_relative_dirs(root, root, &mut dirs).is_ok() {
        for rel in dirs {
            let key = canonical_note_color_key(&rel);
            path_index.entry(key).or_default().push(rel);
        }
    }

    // Collect markdown files relative to the vault root.
    let mut notes = Vec::<String>::new();
    if collect_markdown_files(root, root, &mut notes).is_ok() {
        for rel in notes {
            let key = canonical_note_color_key(&rel);
            path_index.entry(key).or_default().push(rel);
        }
    }

    let mut file_colors = Vec::new();
    for (raw_key, hex) in colors {
        let hex_norm = hex.trim().to_ascii_lowercase();
        if hex_norm.is_empty() {
            continue;
        }
        let id = hex_to_id.get(&hex_norm).cloned().unwrap_or_else(|| {
            let id = format!("bedrock_{:x}", next_id);
            next_id += 1;
            palette.push(ObsidianFileColorPaletteEntry {
                id: id.clone(),
                name: format!("Custom {}", hex),
                color: hex.clone(),
            });
            hex_to_id.insert(hex_norm, id.clone());
            id
        });
        let key = canonical_note_color_key(raw_key);
        let path = path_index
            .get(&key)
            .and_then(|v| v.first().cloned())
            .unwrap_or_else(|| raw_key.clone());
        file_colors.push(ObsidianFileColorFileEntry {
            path,
            color: id,
        });
    }
    ObsidianFileColorData {
        cascade_colors,
        color_background,
        palette,
        file_colors,
    }
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

fn current_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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

fn find_vault_root_for_note(note_path: &Path) -> Option<PathBuf> {
    let mut current = note_path.parent()?;
    loop {
        if current.join(".bedrock").is_dir() || current.join("settings.json").is_file() {
            return Some(current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return None,
        }
    }
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

    // Optional: import note label colors from the Obsidian `obsidian-file-color` plugin.
    let mut source_note_colors: HashMap<String, String> = HashMap::new();
    let mut imported_palette: Option<Vec<NoteColorPaletteEntry>> = None;
    let plugin_data_path = source_canon
        .join(".obsidian")
        .join("plugins")
        .join("obsidian-file-color")
        .join("data.json");
    if plugin_data_path.is_file() {
        if let Ok(raw) = fs::read_to_string(&plugin_data_path) {
            if let Ok(parsed) = serde_json::from_str::<ObsidianFileColorData>(&raw) {
                let from_plugin = obsidian_file_color_to_map(&parsed);
                for (key, hex) in from_plugin {
                    source_note_colors.insert(key, hex);
                }
                // Copy the full data.json (including palette) to destination so the vault has
                // the same palette for "Label color" and fallback loading.
                let dest_plugin_dir = destination_canon.join(".bedrock").join("obsidian-file-color");
                if fs::create_dir_all(&dest_plugin_dir).is_ok() {
                    let dest_plugin_path = dest_plugin_dir.join("data.json");
                    let _ = fs::write(&dest_plugin_path, &raw);
                }
                // Build palette for the report so the frontend can show it on first paint.
                imported_palette = Some(
                    parsed
                        .palette
                        .into_iter()
                        .filter(|e| !e.color.trim().is_empty())
                        .map(|e| NoteColorPaletteEntry {
                            id: e.id,
                            name: e.name,
                            value: e.color,
                        })
                        .collect(),
                );
            }
        }
    }

    let mut destination_note_colors: HashMap<String, String> = HashMap::new();

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
            let rel_str = normalize_rel_path(&rel.to_string_lossy());
            let rel_canon = canonical_note_color_key(&rel_str);
            let color = source_note_colors
                .get(&rel_str)
                .or_else(|| source_note_colors.get(&rel_str.to_ascii_lowercase()))
                .or_else(|| source_note_colors.get(&rel_canon))
                .or_else(|| {
                    rel_str
                        .to_ascii_lowercase()
                        .strip_suffix(".md")
                        .and_then(|no_ext| source_note_colors.get(no_ext))
                })
                .cloned();
            if let Some(color) = color {
                if let Ok(prefix) = destination_file.strip_prefix(&destination_canon) {
                    let dest_rel = normalize_rel_path(&prefix.to_string_lossy());
                    let dest_key = canonical_note_color_key(&dest_rel);
                    destination_note_colors.insert(dest_key, color);
                }
            }
            imported_notes += 1;
        } else {
            imported_images += 1;
        }
        if renamed {
            renamed_notes += 1;
        }
    }

    // Merge directory-only colors from the Obsidian plugin (paths that don't end with .md)
    // so that folder colors appear in the Bedrock vault left pane. Use canonical keys so
    // sidebar lookups match.
    for (path, hex) in &source_note_colors {
        if !path.to_ascii_lowercase().ends_with(".md") {
            let key = canonical_note_color_key(path);
            destination_note_colors.insert(key, hex.clone());
        }
    }

    let source_display = source_canon.to_string_lossy().to_string();
    let destination_display = destination_canon.to_string_lossy().to_string();

    // If we imported any note colors, merge them into the destination vault's
    // `.bedrock/note-colors.json` so the frontend can colorize the left pane.
    let imported_note_colors_count = destination_note_colors.len();
    let mut imported_note_colors_map: Option<HashMap<String, String>> = None;
    if !destination_note_colors.is_empty() {
        let colors_path = destination_canon.join(".bedrock").join("note-colors.json");
        let mut existing: HashMap<String, String> =
            if let Ok(raw) = fs::read_to_string(&colors_path) {
                serde_json::from_str::<HashMap<String, String>>(&raw)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(k, v)| (canonical_note_color_key(&k), v))
                    .collect()
            } else {
                HashMap::new()
            };
        for (k, v) in destination_note_colors {
            existing.insert(k, v);
        }
        imported_note_colors_map = Some(existing.clone());
        let dir = destination_canon.join(".bedrock");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut f = fs::File::create(&colors_path).map_err(|e| e.to_string())?;
        let json =
            serde_json::to_string_pretty(&existing).map_err(|e| e.to_string())?;
        f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }

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
        imported_note_colors: imported_note_colors_count,
        imported_note_colors_map,
        imported_note_color_palette: imported_palette,
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
    let root = session::canonicalize_vault_root(path.trim());
    if !root.exists() {
        return Ok(ReadDirResult {
            notes: Vec::new(),
            empty_dirs: Vec::new(),
        });
    }
    let root_str = root.to_string_lossy().to_string();
    let notes = collect_note_paths(&root_str)?;
    let mut all_dirs = Vec::new();
    collect_relative_dirs(&root, &root, &mut all_dirs)?;
    let empty_dirs = all_dirs
        .into_iter()
        .filter(|d| {
            !notes.iter().any(|n| n == d || n.starts_with(&format!("{d}/")))
        })
        .collect();
    Ok(ReadDirResult { notes, empty_dirs })
}

#[tauri::command]
fn read_file(path: &str, cache: State<RecentNotesCache>) -> Result<String, String> {
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;

    // Backend-side recent-notes tracking: whenever a markdown note is read,
    // infer its vault root from the filesystem and update the RecentNotesCache
    // and `.bedrock/recent.json`. This provides a persistence fallback even if
    // the frontend close flow or cache_recent_notes invocations fail or race.
    let note_path = PathBuf::from(path);
    if is_markdown_file(&note_path) {
        if let Some(vault_root) = find_vault_root_for_note(&note_path) {
            let canon_root = session::canonicalize_vault_root(
                &vault_root.to_string_lossy().to_string(),
            );
            let vault_key = canon_root.to_string_lossy().to_string();
            if let Ok(rel) = note_path
                .strip_prefix(&canon_root)
                .or_else(|_| note_path.strip_prefix(&vault_root))
            {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                let title = Path::new(&rel_str)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&rel_str)
                    .to_string();
                    let entry = session::RecentNoteEntry {
                    path: rel_str.clone(),
                    title,
                    last_opened: current_time_millis(),
                };
                    if let Ok(mut guard) = cache.0.lock() {
                    let list = guard.entry(vault_key.clone()).or_insert_with(Vec::new);
                    list.retain(|e| e.path != entry.path);
                    list.insert(0, entry);
                    if list.len() > 50 {
                    list.truncate(50);
                    }
                    let _ = crate::session::write_recent_notes_to_disk(&vault_key, list);
                }
            }
        }
    }

    Ok(content)
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
        // Create initial welcome file with structure for seven-goals verification (headings, tags, wiki-links).
        let welcome_path = vault_path.join("Welcome.md");
        let welcome_content = r#"# Welcome to Bedrock

Bedrock is a fast, premium markdown note-taking tool.

## Features

- Powered by **Rust** and **Tauri**
- Extensible via CSS variables and plugins
- See [[Getting Started]] for next steps

## Quick links

- [[Getting Started]]
- Tags: #welcome #bedrock #demo
"#;
        fs::write(&welcome_path, welcome_content).map_err(|e| e.to_string())?;
        let getting_started_path = vault_path.join("Getting Started.md");
        let getting_started_content = r#"# Getting Started

Return to [[Welcome]] to see the main overview.

Tags: #getting-started #demo
"#;
        fs::write(&getting_started_path, getting_started_content).map_err(|e| e.to_string())?;
    }

    let canon = vault_path
        .canonicalize()
        .unwrap_or(vault_path);
    Ok(canon.to_string_lossy().into_owned())
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
fn load_note_colors(vault_path: &str) -> Result<LoadNoteColorsResult, String> {
    let vault_path = vault_path.trim();
    let root = session::canonicalize_vault_root(vault_path);
    let canonical = root.to_string_lossy().to_string();
    eprintln!(
        "[Bedrock] load_note_colors vault root: {} (hybrid: {})",
        root.display(),
        is_hybrid_vault(&root)
    );
    let note_colors_path = root.join(".bedrock").join("note-colors.json");

    let try_obsidian_fallback = || {
        let data = load_obsidian_file_color_data(&root)?;
        let map = obsidian_file_color_to_map(&data);
        if map.is_empty() && !data.file_colors.is_empty() {
            eprintln!(
                "[Bedrock] obsidian-file-color: {} fileColor(s) but 0 resolved — check palette uses \"value\" or \"color\" for hex",
                data.file_colors.len()
            );
        }
        if map.is_empty() {
            return None;
        }
        Some((map, data))
    };

    let sync_obsidian_to_bedrock = |data: &ObsidianFileColorData| {
        if !is_hybrid_vault(&root) {
            return;
        }
        let dir = root.join(".bedrock").join("obsidian-file-color");
        if fs::create_dir_all(&dir).is_ok() {
            let path = dir.join("data.json");
            if let Ok(json) = serde_json::to_string_pretty(data) {
                let _ = fs::File::create(&path).and_then(|mut f| {
                    f.write_all(json.as_bytes())?;
                    f.sync_all()
                });
            }
        }
    };

    if note_colors_path.is_file() {
        let raw = match fs::read_to_string(&note_colors_path) {
            Ok(s) => s,
            Err(_) => return Ok(LoadNoteColorsResult { canonical_vault_path: canonical.clone(), colors: HashMap::new() }),
        };
        let parsed = serde_json::from_str::<HashMap<String, String>>(&raw)
            .unwrap_or_default();
        let normalized: HashMap<String, String> = parsed
            .into_iter()
            .map(|(k, v)| (canonical_note_color_key(&k), v))
            .collect();
        // If we have entries, use them.
        if !normalized.is_empty() {
            let sample: Vec<&str> = normalized.keys().take(3).map(String::as_str).collect();
            eprintln!(
                "[Bedrock] loaded {} note color(s) from note-colors.json for vault: {} — sample keys: {:?}",
                normalized.len(),
                root.display(),
                sample
            );
            return Ok(LoadNoteColorsResult { canonical_vault_path: canonical, colors: normalized });
        }
        // note-colors.json exists but is empty (e.g. failed write or stale); try fallback.
        if let Some((map, data)) = try_obsidian_fallback() {
        if !map.is_empty() {
            let sample: Vec<&str> = map.keys().take(3).map(String::as_str).collect();
            eprintln!(
                "[Bedrock] loaded {} note color(s) from obsidian-file-color (fallback) for vault: {} — sample keys: {:?}",
                map.len(),
                root.display(),
                sample
            );
        }
            sync_obsidian_to_bedrock(&data);
            let dir = root.join(".bedrock");
            let _ = fs::create_dir_all(&dir);
            let path = dir.join("note-colors.json");
            if let Ok(json) = serde_json::to_string_pretty(&map) {
                let _ = fs::File::create(&path).and_then(|mut f| {
                    f.write_all(json.as_bytes())?;
                    f.sync_all()
                });
            }
            return Ok(LoadNoteColorsResult { canonical_vault_path: canonical, colors: map });
        }
        return Ok(LoadNoteColorsResult { canonical_vault_path: canonical, colors: normalized });
    }

    // Fallback: Bedrock vault may have a copy of obsidian-file-color data (e.g. from a prior
    // import or manual copy). Load it so left-pane colors work without re-importing.
    // Map keys use canonical_note_color_key so Obsidian paths (e.g. "MentalHealth") match
    // filesystem paths (e.g. "Mental-health") in the sidebar.
    // For hybrid vaults, sync the loaded Obsidian data to .bedrock so both stay in sync.
    if let Some((map, data)) = try_obsidian_fallback() {
        if !map.is_empty() {
            let sample: Vec<&str> = map.keys().take(3).map(String::as_str).collect();
            eprintln!(
                "[Bedrock] loaded {} note color(s) from obsidian-file-color for vault: {} — sample keys: {:?}",
                map.len(),
                root.display(),
                sample
            );
        }
        sync_obsidian_to_bedrock(&data);
        let dir = root.join(".bedrock");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("note-colors.json");
        if let Ok(json) = serde_json::to_string_pretty(&map) {
            let _ = fs::File::create(&path).and_then(|mut f| {
                f.write_all(json.as_bytes())?;
                f.sync_all()
            });
        }
        return Ok(LoadNoteColorsResult { canonical_vault_path: canonical, colors: map });
    }
    eprintln!(
        "[Bedrock] load_note_colors: returning 0 colors for vault: {} (no note-colors.json or obsidian-file-color data)",
        root.display()
    );
    Ok(LoadNoteColorsResult { canonical_vault_path: canonical, colors: HashMap::new() })
}

#[tauri::command]
fn canonicalize_vault_path(vault_path: &str) -> Result<String, String> {
    let root = session::canonicalize_vault_root(vault_path.trim());
    Ok(root.to_string_lossy().to_string())
}

#[tauri::command]
fn load_note_color_palette(vault_path: &str) -> Result<Vec<NoteColorPaletteEntry>, String> {
    let vault_path = vault_path.trim();
    let root = session::canonicalize_vault_root(vault_path);
    let paths = if is_hybrid_vault(&root) {
        vec![
            root.join(".obsidian")
                .join("plugins")
                .join("obsidian-file-color")
                .join("data.json"),
            root.join(".bedrock")
                .join("obsidian-file-color")
                .join("data.json"),
        ]
    } else {
        vec![
            root.join(".bedrock")
                .join("obsidian-file-color")
                .join("data.json"),
            root.join(".obsidian")
                .join("plugins")
                .join("obsidian-file-color")
                .join("data.json"),
        ]
    };
    for path in paths {
        if path.is_file() {
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(data) = serde_json::from_str::<ObsidianFileColorData>(&raw) {
                    let palette = data
                        .palette
                        .into_iter()
                        .filter(|e| !e.color.trim().is_empty())
                        .map(|e| NoteColorPaletteEntry {
                            id: e.id,
                            name: e.name,
                            value: e.color,
                        })
                        .collect();
                    return Ok(palette);
                }
            }
        }
    }
    Ok(Vec::new())
}

#[tauri::command]
fn save_note_colors(
    vault_path: &str,
    colors: HashMap<String, String>,
) -> Result<(), String> {
    let vault_path = vault_path.trim();
    let root = session::canonicalize_vault_root(vault_path);
    let dir = root.join(".bedrock");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("note-colors.json");
    let json = serde_json::to_string_pretty(&colors).map_err(|e| e.to_string())?;
    let mut f = fs::File::create(&path).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;

    // Hybrid vault: sync same data to .obsidian/plugins/obsidian-file-color so Obsidian and Bedrock stay in sync.
    if is_hybrid_vault(&root) {
        let existing = load_obsidian_file_color_data(&root);
        let data = build_obsidian_file_color_data(&root, &colors, existing.as_ref());
        let json_data = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
        let bedrock_plugin = root.join(".bedrock").join("obsidian-file-color");
        let _ = fs::create_dir_all(&bedrock_plugin);
        let _ = fs::write(bedrock_plugin.join("data.json"), &json_data);
        let obsidian_plugin = root
            .join(".obsidian")
            .join("plugins")
            .join("obsidian-file-color");
        if fs::create_dir_all(&obsidian_plugin).is_ok() {
            let _ = fs::write(obsidian_plugin.join("data.json"), &json_data);
        }
    }

    Ok(())
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

    #[test]
    fn imports_obsidian_file_color_palette_into_note_colors() {
        let source = unique_temp_dir("obsidian-source-file-color");
        let destination = unique_temp_dir("bedrock-destination-file-color");

        // Minimal Obsidian vault layout with the file-color plugin enabled.
        fs::create_dir_all(
            source.join(".obsidian/plugins/obsidian-file-color"),
        )
        .unwrap();
        fs::create_dir_all(source.join("Notes")).unwrap();
        fs::create_dir_all(source.join("Personal")).unwrap();

        // Notes that should be imported.
        fs::write(
            source.join("Notes/Project Alpha.md"),
            "# Project Alpha\n",
        )
        .unwrap();
        fs::write(
            source.join("Personal/Journal.md"),
            "# Journal\n",
        )
        .unwrap();

        // Obsidian `obsidian-file-color` data.json matching the documented shape.
        let plugin_data = r##"{
  "palette": [
    { "id": "12345", "name": "Urgent", "color": "#ff0000" }
  ],
  "fileColors": [
    { "path": "Notes/Project Alpha.md", "color": "12345" },
    { "path": "Personal/Journal", "color": "12345" }
  ],
  "cascadeColors": false,
  "backgroundColors": true
}"##;
        fs::write(
            source.join(".obsidian/plugins/obsidian-file-color/data.json"),
            plugin_data,
        )
        .unwrap();

        let report = import_obsidian_vault_notes(&source, &destination).unwrap();
        assert!(report.success);
        assert_eq!(report.imported_notes, 2);
        assert_eq!(
            report.imported_note_colors, 3,
            "Notes/Project Alpha.md, Personal/Journal.md, Personal/Journal (dir) => 3 color entries"
        );

        // Notes exist in the destination vault.
        assert!(destination.join("Notes/Project Alpha.md").exists());
        assert!(destination.join("Personal/Journal.md").exists());

        // Colors were merged into `.bedrock/note-colors.json` with vault-relative paths (lowercase keys).
        let colors_path = destination.join(".bedrock/note-colors.json");
        let raw = fs::read_to_string(&colors_path).unwrap();
        let parsed: HashMap<String, String> =
            serde_json::from_str(&raw).expect("valid colors map");

        assert_eq!(
            parsed.get("notes/project alpha.md"),
            Some(&"#ff0000".to_string())
        );
        // The plugin uses "Personal/Journal" (without `.md`); we support that by
        // also inserting a key with `.md` when building source_note_colors.
        assert_eq!(
            parsed.get("personal/journal.md"),
            Some(&"#ff0000".to_string())
        );
        // Directory-only path from the plugin (path without .md) must also be present
        // so folder colouring works in the left pane.
        assert_eq!(
            parsed.get("personal/journal"),
            Some(&"#ff0000".to_string()),
            "directory colors from obsidian-file-color must be imported"
        );

        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(destination);
    }

    /// Real obsidian-file-color plugin uses "value" for hex in palette, not "color".
    #[test]
    fn imports_obsidian_file_color_palette_with_value_field() {
        let source = unique_temp_dir("obsidian-source-value");
        let destination = unique_temp_dir("bedrock-destination-value");

        fs::create_dir_all(source.join(".obsidian/plugins/obsidian-file-color")).unwrap();
        fs::create_dir_all(source.join("Gaming")).unwrap();

        fs::write(source.join("Gaming/Note.md"), "# Gaming\n").unwrap();

        // Same shape as real plugin: palette uses "value", not "color".
        let plugin_data = r##"{
  "cascadeColors": false,
  "colorBackground": false,
  "palette": [
    { "id": "red-id", "name": "Red", "value": "#ff0000" },
    { "id": "blue-id", "name": "Blue", "value": "#001eff" }
  ],
  "fileColors": [
    { "path": "Gaming", "color": "red-id" },
    { "path": "Gaming/Note.md", "color": "blue-id" }
  ]
}"##;
        fs::write(
            source.join(".obsidian/plugins/obsidian-file-color/data.json"),
            plugin_data,
        )
        .unwrap();

        let report = import_obsidian_vault_notes(&source, &destination).unwrap();
        assert!(report.success);
        assert_eq!(report.imported_notes, 1);
        assert_eq!(report.imported_note_colors, 2, "Gaming (dir) + Gaming/Note.md");

        let colors_path = destination.join(".bedrock/note-colors.json");
        let raw = fs::read_to_string(&colors_path).unwrap();
        let parsed: HashMap<String, String> = serde_json::from_str(&raw).unwrap();

        assert_eq!(parsed.get("gaming"), Some(&"#ff0000".to_string()));
        assert_eq!(parsed.get("gaming/note.md"), Some(&"#001eff".to_string()));

        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(destination);
    }

    /// load_note_colors falls back to .bedrock/obsidian-file-color/data.json when
    /// note-colors.json is missing (e.g. Bedrock vault with plugin data copied in).
    #[test]
    fn load_note_colors_fallback_obsidian_file_color() {
        let vault = unique_temp_dir("vault-load-colors-fallback");
        fs::create_dir_all(vault.join(".bedrock/obsidian-file-color")).unwrap();
        let plugin_data = r##"{
  "palette": [ { "id": "c1", "name": "Red", "value": "#ff0000" } ],
  "fileColors": [
    { "path": "Gaming", "color": "c1" },
    { "path": "Dev/Note.md", "color": "c1" }
  ]
}"##;
        fs::write(
            vault.join(".bedrock/obsidian-file-color/data.json"),
            plugin_data,
        )
        .unwrap();

        let colors = load_note_colors(vault.to_str().unwrap()).unwrap();
        assert_eq!(colors.colors.get("gaming"), Some(&"#ff0000".to_string()));
        assert_eq!(colors.colors.get("dev/note.md"), Some(&"#ff0000".to_string()));

        // Fallback persists to note-colors.json so next launch and UI have consistent data.
        let colors_path = vault.join(".bedrock/note-colors.json");
        assert!(colors_path.is_file(), "fallback should write note-colors.json");
        let raw = fs::read_to_string(&colors_path).unwrap();
        let persisted: HashMap<String, String> = serde_json::from_str(&raw).unwrap();
        assert_eq!(persisted.get("gaming"), Some(&"#ff0000".to_string()));
        assert_eq!(persisted.get("dev/note.md"), Some(&"#ff0000".to_string()));

        let _ = fs::remove_dir_all(vault);
    }

    /// When fileColors[].color is a hex string (no palette id match), we still produce a non-empty map.
    #[test]
    fn load_note_colors_hex_in_file_colors() {
        let vault = unique_temp_dir("vault-hex-file-colors");
        fs::create_dir_all(vault.join(".bedrock/obsidian-file-color")).unwrap();
        // Plugin variant that stores hex directly in fileColors (no palette, or palette ids don't match).
        let plugin_data = r##"{
  "palette": [],
  "fileColors": [
    { "path": "AI", "color": "#22c55e" },
    { "path": "Gaming", "color": "#3B82F6" }
  ]
}"##;
        fs::write(
            vault.join(".bedrock/obsidian-file-color/data.json"),
            plugin_data,
        )
        .unwrap();

        let colors = load_note_colors(vault.to_str().unwrap()).unwrap();
        assert_eq!(colors.colors.get("ai"), Some(&"#22c55e".to_string()));
        assert_eq!(colors.colors.get("gaming"), Some(&"#3b82f6".to_string()));

        let _ = fs::remove_dir_all(vault);
    }

    /// Hybrid vault: both .obsidian and .bedrock exist; we load from the source with
    /// the most fileColors and sync to .bedrock so both stay in sync.
    #[test]
    fn load_note_colors_hybrid_vault_prefers_most_file_colors() {
        let vault = unique_temp_dir("vault-hybrid-file-colors");
        fs::create_dir_all(vault.join(".obsidian/plugins/obsidian-file-color")).unwrap();
        fs::create_dir_all(vault.join(".bedrock/obsidian-file-color")).unwrap();

        // Obsidian plugin has 2 fileColors
        let obsidian_data = r##"{
  "palette": [ { "id": "c1", "name": "Red", "value": "#ff0000" } ],
  "fileColors": [
    { "path": "AI", "color": "c1" },
    { "path": "Gaming", "color": "c1" }
  ]
}"##;
        fs::write(
            vault.join(".obsidian/plugins/obsidian-file-color/data.json"),
            obsidian_data,
        )
        .unwrap();

        // .bedrock copy has 3 fileColors (more than Obsidian)
        let bedrock_data = r##"{
  "palette": [ { "id": "c1", "name": "Red", "value": "#ff0000" } ],
  "fileColors": [
    { "path": "AI", "color": "c1" },
    { "path": "Gaming", "color": "c1" },
    { "path": "Dev", "color": "c1" }
  ]
}"##;
        fs::write(
            vault.join(".bedrock/obsidian-file-color/data.json"),
            bedrock_data,
        )
        .unwrap();

        let colors = load_note_colors(vault.to_str().unwrap()).unwrap();
        // Should prefer the source with more fileColors (.bedrock has 3); map also adds .md variants
        assert!(colors.colors.len() >= 3);
        assert_eq!(colors.colors.get("ai"), Some(&"#ff0000".to_string()));
        assert_eq!(colors.colors.get("gaming"), Some(&"#ff0000".to_string()));
        assert_eq!(colors.colors.get("dev"), Some(&"#ff0000".to_string()));

        // Fallback should persist to note-colors.json and sync .bedrock copy
        let note_colors_path = vault.join(".bedrock/note-colors.json");
        assert!(note_colors_path.is_file());
        let raw = fs::read_to_string(&note_colors_path).unwrap();
        let persisted: HashMap<String, String> = serde_json::from_str(&raw).unwrap();
        assert_eq!(persisted.get("ai"), Some(&"#ff0000".to_string()));
        assert_eq!(persisted.get("dev"), Some(&"#ff0000".to_string()));

        let _ = fs::remove_dir_all(vault);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(RecentNotesCache::default())
        .manage(PendingClose::default())
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
            load_note_colors,
            canonicalize_vault_path,
            load_note_color_palette,
            save_note_colors,
            save_settings,
            load_settings,
            open_settings_window,
            import_obsidian_vault_with_picker,
            pick_bedrock_vault,
            load_vault_session,
            save_vault_session,
            read_recent_notes,
            save_recent_notes,
            cache_recent_notes,
            close_window_now,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if window.label() == "main" {
                    // Flush in-memory recent notes to disk so persistence survives even if
                    // the frontend close flow didn't run or had empty state (e.g. timeout).
                    if let Some(cache) = window.app_handle().try_state::<RecentNotesCache>() {
                        crate::session::flush_recent_notes_cache(&cache);
                    }
                    window.app_handle().exit(0);
                }
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let allow = window
                    .app_handle()
                    .try_state::<PendingClose>()
                    .map(|p| crate::session::is_close_allowed(&p))
                    .unwrap_or(false);
                if allow {
                    return;
                }
                // Persist recent notes from backend cache before the frontend close flow,
                // so persistence survives even when the webview never runs the close handler
                // (e.g. release build) or sends empty state. Delay briefly so any in-flight
                // cache_recent_notes from the frontend (debounced persist, push_to_recent_notes)
                // can complete and update the cache first.
                api.prevent_close();
                let app = window.app_handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    if let Some(cache) = app.try_state::<RecentNotesCache>() {
                        crate::session::flush_recent_notes_cache(&cache);
                    }
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.emit("save-state-and-close", ());
                    }
                });
            }
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, e| {
            #[cfg(target_os = "linux")]
            if let RunEvent::Ready = e {
                let icon_bytes = include_bytes!("../icons/32x32.png");
                if let (Some(window), Ok(icon)) = (
                    app.get_webview_window("main"),
                    tauri::image::Image::from_bytes(icon_bytes),
                ) {
                    let _ = window.set_icon(icon);
                }
            }
        });
}
