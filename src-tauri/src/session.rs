use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Manager, State, WebviewWindow};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct RecentNoteEntry {
    pub path: String,
    pub title: String,
    pub last_opened: i64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct VaultSessionState {
    pub open_vaults: Vec<String>,
    pub active_vault: Option<String>,
    #[serde(default)]
    pub recent_notes: HashMap<String, Vec<RecentNoteEntry>>,
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

    let mut recent_notes = HashMap::<String, Vec<RecentNoteEntry>>::new();
    for (raw_path, entries) in state.recent_notes.into_iter() {
        let Some(normalized) = normalize_path(&raw_path) else {
            continue;
        };
        if entries.is_empty() {
            continue;
        }
        // Prefer the last non-empty list we see for a given vault.
        recent_notes.insert(normalized, entries);
    }

    VaultSessionState {
        open_vaults,
        active_vault,
        recent_notes,
    }
}

fn merge_vault_session_state(
    base: Option<VaultSessionState>,
    open_vaults: Vec<String>,
    active_vault: Option<String>,
    active_vault_recent_notes: Option<Vec<RecentNoteEntry>>,
) -> VaultSessionState {
    let mut normalized = normalize_vault_session_state(VaultSessionState {
        open_vaults,
        active_vault,
        recent_notes: base
            .as_ref()
            .map(|b| b.recent_notes.clone())
            .unwrap_or_default(),
    });

    if let (Some(active), Some(entries)) = (&normalized.active_vault, &active_vault_recent_notes) {
        if !entries.is_empty() {
            let key = canonicalize_vault_root(active).to_string_lossy().to_string();
            normalized.recent_notes.insert(key, entries.clone());
        }
    }

    for vault in &normalized.open_vaults {
        let key = canonicalize_vault_root(vault).to_string_lossy().to_string();
        if normalized
            .recent_notes
            .get(&key)
            .map(|v| v.is_empty())
            .unwrap_or(true)
        {
            let entries = read_recent_notes_from_disk(vault);
            if !entries.is_empty() {
                normalized.recent_notes.insert(key, entries);
            }
        }
    }

    normalized
}

fn vault_session_state_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("vault-session.json"))
}

fn vault_session_fallback_path(app: &AppHandle) -> Result<PathBuf, String> {
    let docs = app.path().document_dir().map_err(|e| e.to_string())?;
    let default_vault = docs.join("BedrockVault");
    crate::ensure_bedrock_layout(&default_vault)?;
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

fn recent_notes_path(vault_path: &str) -> PathBuf {
    let root = Path::new(vault_path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(vault_path));
    root.join(".bedrock").join("recent.json")
}

pub(crate) fn canonicalize_vault_root(vault_path: &str) -> PathBuf {
    Path::new(vault_path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(vault_path))
}

pub struct RecentNotesCache(pub(crate) Mutex<HashMap<String, Vec<RecentNoteEntry>>>);

impl Default for RecentNotesCache {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

pub struct PendingClose(AtomicBool);

impl Default for PendingClose {
    fn default() -> Self {
        Self(AtomicBool::new(false))
    }
}

fn read_recent_notes_from_disk(vault_path: &str) -> Vec<RecentNoteEntry> {
    let path = recent_notes_path(vault_path);
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            return Vec::new();
        }
    };

    if let Ok(entries) = serde_json::from_str::<Vec<RecentNoteEntry>>(&raw) {
        return entries;
    }

    if let Ok(paths) = serde_json::from_str::<Vec<String>>(&raw) {
        let entries: Vec<RecentNoteEntry> = paths
            .into_iter()
            .map(|p| {
                let title = Path::new(&p)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&p)
                    .to_string();
                RecentNoteEntry {
                    path: p,
                    title,
                    last_opened: 0,
                }
            })
            .collect();
        let _ = write_recent_notes_to_disk(vault_path, &entries);
        return entries;
    }

    Vec::new()
}

pub(crate) fn write_recent_notes_to_disk(
    vault_path: &str,
    entries: &[RecentNoteEntry],
) -> Result<(), String> {
    let root = canonicalize_vault_root(vault_path);
    let dir = root.join(".bedrock");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("recent.json");
    let json = serde_json::to_string(entries).map_err(|e| e.to_string())?;
    let mut f = fs::File::create(&path).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}

fn cache_recent_notes_impl(
    vault_path: &str,
    entries: &[RecentNoteEntry],
    cache: &RecentNotesCache,
) -> Result<(), String> {
    let canon = canonicalize_vault_root(vault_path);
    let key = canon.to_string_lossy().to_string();

    // Never overwrite with empty if we already have data (avoids close-flow sending
    // empty and wiping persistence; keep existing disk/cache for next launch).
    if entries.is_empty() {
        if let Ok(c) = cache.0.lock() {
            if c.get(&key).map(|v| !v.is_empty()).unwrap_or(false) {
                return Ok(());
            }
        }
        let path = recent_notes_path(canon.to_string_lossy().as_ref());
        if path.exists() {
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(parsed) = serde_json::from_str::<Vec<RecentNoteEntry>>(&raw) {
                    if !parsed.is_empty() {
                        return Ok(());
                    }
                }
            }
        }
    }

    if let Ok(mut c) = cache.0.lock() {
        c.insert(key.clone(), entries.to_vec());
    }
    write_recent_notes_to_disk(canon.to_string_lossy().as_ref(), entries)
}

#[tauri::command]
pub fn read_recent_notes(
    vault_path: String,
    cache: State<RecentNotesCache>,
) -> Vec<RecentNoteEntry> {
    let entries = read_recent_notes_from_disk(&vault_path);
    if !entries.is_empty() {
        let canon = canonicalize_vault_root(&vault_path);
        let key = canon.to_string_lossy().to_string();
        if let Ok(mut c) = cache.0.lock() {
            c.insert(key, entries.clone());
        }
    }
    entries
}

#[tauri::command]
pub fn save_recent_notes(
    vault_path: String,
    entries: Vec<RecentNoteEntry>,
    cache: State<RecentNotesCache>,
) -> Result<(), String> {
    cache_recent_notes_impl(&vault_path, &entries, &cache)
}

#[tauri::command]
pub fn cache_recent_notes(
    vault_path: String,
    entries: Vec<RecentNoteEntry>,
    cache: State<RecentNotesCache>,
) -> Result<(), String> {
    cache_recent_notes_impl(&vault_path, &entries, &cache)
}

#[tauri::command]
pub fn close_window_now(
    window: WebviewWindow,
    pending: State<PendingClose>,
) -> Result<(), String> {
    pending.0.store(true, Ordering::SeqCst);
    window.close().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_vault_session(app: AppHandle) -> Result<VaultSessionState, String> {
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
    let mut normalized = normalize_vault_session_state(parsed);

    // Hydrate recent-notes state for each open vault so the frontend can restore
    // the Recent notes pane immediately on startup, even before any refresh
    // completes. Use the same canonical key format as RecentNotesCache.
    let mut recent_map = normalized.recent_notes.clone();
    for vault in &normalized.open_vaults {
        let canon = canonicalize_vault_root(vault);
        let key = canon.to_string_lossy().to_string();
        let has_non_empty = recent_map
            .get(&key)
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if has_non_empty {
            continue;
        }
        let entries = read_recent_notes_from_disk(vault);
        if !entries.is_empty() {
            recent_map.insert(key, entries);
        }
    }
    normalized.recent_notes = recent_map;

    persist_vault_session_state(&app, &normalized)?;
    Ok(normalized)
}

#[tauri::command]
pub fn save_vault_session(
    app: AppHandle,
    open_vaults: Vec<String>,
    active_vault: Option<String>,
    active_vault_recent_notes: Option<Vec<RecentNoteEntry>>,
) -> Result<VaultSessionState, String> {
    // Merge with existing session so we never overwrite recent_notes with empty.
    // Otherwise every frontend persist (vault switch, startup) would wipe recent_notes
    // from the session file and we'd rely only on load_vault_session's hydrate step.
    let mut base = None::<VaultSessionState>;
    if let Ok(path) = vault_session_state_path(&app) {
        if path.exists() {
            base = read_vault_session_from_path(&path);
        }
    }
    if base.is_none() {
        if let Ok(path) = vault_session_fallback_path(&app) {
            if path.exists() {
                base = read_vault_session_from_path(&path);
            }
        }
    }
    let normalized = merge_vault_session_state(
        base,
        open_vaults,
        active_vault,
        active_vault_recent_notes,
    );
    persist_vault_session_state(&app, &normalized)?;
    Ok(normalized)
}

pub fn flush_recent_notes_cache(cache: &RecentNotesCache) {
    if let Ok(guard) = cache.0.lock() {
        for (vault_path, entries) in guard.iter() {
            let _ = write_recent_notes_to_disk(vault_path, entries);
        }
    }
}

pub fn is_close_allowed(pending: &PendingClose) -> bool {
    pending.0.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
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
    fn write_and_read_recent_notes_roundtrip() {
        let vault_root = unique_temp_dir("recent-notes-roundtrip");
        fs::create_dir_all(&vault_root).unwrap();
        let vault_path = vault_root.to_string_lossy().to_string();

        let entries = vec![RecentNoteEntry {
            path: "Note.md".to_string(),
            title: "Note.md".to_string(),
            last_opened: 123,
        }];

        write_recent_notes_to_disk(&vault_path, &entries).unwrap();

        let read_back = read_recent_notes_from_disk(&vault_path);
        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].path, "Note.md");
        assert_eq!(read_back[0].title, "Note.md");

        let _ = fs::remove_dir_all(&vault_root);
    }

    #[test]
    fn cache_recent_notes_does_not_overwrite_non_empty_with_empty() {
        let vault_root = unique_temp_dir("recent-notes-no-overwrite");
        fs::create_dir_all(&vault_root).unwrap();
        let vault_path = vault_root.to_string_lossy().to_string();
        let cache = RecentNotesCache::default();

        let initial = vec![RecentNoteEntry {
            path: "Note.md".to_string(),
            title: "Note.md".to_string(),
            last_opened: 123,
        }];

        cache_recent_notes_impl(&vault_path, &initial, &cache).unwrap();

        let first = read_recent_notes_from_disk(&vault_path);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].path, "Note.md");

        let empty: Vec<RecentNoteEntry> = Vec::new();
        cache_recent_notes_impl(&vault_path, &empty, &cache).unwrap();

        let second = read_recent_notes_from_disk(&vault_path);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].path, "Note.md");

        let _ = fs::remove_dir_all(&vault_root);
    }

    #[test]
    fn normalize_vault_session_state_dedups_and_canonicalizes_open_vaults() {
        let root = unique_temp_dir("vault-session-normalize");
        fs::create_dir_all(&root).unwrap();
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();

        let canonical = vault.canonicalize().unwrap();
        let canonical_str = canonical.to_string_lossy().to_string();

        let state = VaultSessionState {
            open_vaults: vec![
                canonical_str.clone(),
                canonical_str.clone(), // duplicate
            ],
            active_vault: Some(canonical_str.clone()),
            recent_notes: HashMap::new(),
        };

        let normalized = normalize_vault_session_state(state);
        assert_eq!(normalized.open_vaults.len(), 1);
        assert_eq!(normalized.open_vaults[0], canonical_str);
        assert_eq!(normalized.active_vault, Some(canonical_str));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_session_preserves_existing_recent_notes_when_frontend_sends_none() {
        let root = unique_temp_dir("vault-session-merge-preserve");
        fs::create_dir_all(&root).unwrap();
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        let canonical = vault.canonicalize().unwrap();
        let canonical_str = canonical.to_string_lossy().to_string();

        let entry = RecentNoteEntry {
            path: "Note.md".to_string(),
            title: "Note".to_string(),
            last_opened: 1,
        };

        let mut recent_map = HashMap::new();
        recent_map.insert(canonical_str.clone(), vec![entry.clone()]);

        let base = Some(VaultSessionState {
            open_vaults: vec![canonical_str.clone()],
            active_vault: Some(canonical_str.clone()),
            recent_notes: recent_map,
        });

        let merged = merge_vault_session_state(
            base,
            vec![canonical_str.clone()],
            Some(canonical_str.clone()),
            None,
        );

        let list = merged
            .recent_notes
            .get(&canonical_str)
            .cloned()
            .unwrap_or_default();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].path, entry.path);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_session_uses_active_recent_notes_when_provided() {
        let root = unique_temp_dir("vault-session-merge-active");
        fs::create_dir_all(&root).unwrap();
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        let canonical = vault.canonicalize().unwrap();
        let canonical_str = canonical.to_string_lossy().to_string();

        let old_entry = RecentNoteEntry {
            path: "Old.md".to_string(),
            title: "Old".to_string(),
            last_opened: 1,
        };
        let mut recent_map = HashMap::new();
        recent_map.insert(canonical_str.clone(), vec![old_entry]);

        let base = Some(VaultSessionState {
            open_vaults: vec![canonical_str.clone()],
            active_vault: Some(canonical_str.clone()),
            recent_notes: recent_map,
        });

        let new_entry = RecentNoteEntry {
            path: "New.md".to_string(),
            title: "New".to_string(),
            last_opened: 2,
        };

        let merged = merge_vault_session_state(
            base,
            vec![canonical_str.clone()],
            Some(canonical_str.clone()),
            Some(vec![new_entry.clone()]),
        );

        let list = merged
            .recent_notes
            .get(&canonical_str)
            .cloned()
            .unwrap_or_default();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].path, new_entry.path);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_session_hydrates_missing_vault_from_disk() {
        let root = unique_temp_dir("vault-session-merge-hydrate");
        fs::create_dir_all(&root).unwrap();
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        let canonical = vault.canonicalize().unwrap();
        let canonical_str = canonical.to_string_lossy().to_string();

        let entry = RecentNoteEntry {
            path: "Persisted.md".to_string(),
            title: "Persisted".to_string(),
            last_opened: 3,
        };

        // Seed disk state for this vault.
        write_recent_notes_to_disk(&canonical_str, &[entry.clone()]).unwrap();

        let merged = merge_vault_session_state(
            None,
            vec![canonical_str.clone()],
            Some(canonical_str.clone()),
            None,
        );

        let list = merged
            .recent_notes
            .get(&canonical_str)
            .cloned()
            .unwrap_or_default();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].path, entry.path);

        let _ = fs::remove_dir_all(&root);
    }
}

