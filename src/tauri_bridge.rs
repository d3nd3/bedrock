use crate::app_state::{
    AppSettings, RecentNoteEntry, VaultImportReport, VaultNote, VaultSessionState,
};
use js_sys::{Object, Reflect};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

#[derive(Serialize)]
struct ReadDirArgs<'a> {
    path: &'a str,
}

#[derive(Deserialize)]
pub struct ReadDirResult {
    pub notes: Vec<String>,
    pub empty_dirs: Vec<String>,
}

#[derive(Serialize)]
struct ReadFileArgs<'a> {
    path: &'a str,
}

#[derive(Serialize)]
struct WriteFileArgs<'a> {
    path: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct SaveSettingsArgs<'a> {
    vault_path: &'a str,
    settings: &'a str,
}

#[derive(Serialize)]
struct VaultPathArgs<'a> {
    vault_path: &'a str,
}

#[derive(Serialize)]
struct RenameNoteArgs<'a> {
    vault_path: &'a str,
    old_path: &'a str,
    new_path: &'a str,
}

#[derive(Serialize)]
struct SaveRecentNotesArgs<'a> {
    vault_path: &'a str,
    entries: &'a [RecentNoteEntry],
}

pub async fn read_dir(path: &str) -> ReadDirResult {
    let args = serde_wasm_bindgen::to_value(&ReadDirArgs { path }).unwrap();
    let dir_val = invoke("read_dir", args).await;
    serde_wasm_bindgen::from_value::<ReadDirResult>(dir_val).unwrap_or_else(|_| ReadDirResult {
        notes: Vec::new(),
        empty_dirs: Vec::new(),
    })
}

pub async fn read_vault_notes(vault_path: &str) -> Vec<VaultNote> {
    let vault_args =
        serde_wasm_bindgen::to_value(&VaultPathArgs { vault_path }).unwrap();
    let notes_val = invoke("read_vault_notes", vault_args).await;
    serde_wasm_bindgen::from_value::<Vec<VaultNote>>(notes_val).unwrap_or_default()
}

pub async fn read_recent_notes(vault_path: &str) -> Vec<RecentNoteEntry> {
    let args =
        serde_wasm_bindgen::to_value(&VaultPathArgs { vault_path }).unwrap_or(JsValue::NULL);
    let val = invoke("read_recent_notes", args).await;
    serde_wasm_bindgen::from_value::<Vec<RecentNoteEntry>>(val).unwrap_or_default()
}

pub async fn cache_recent_notes(vault_path: &str, entries: &[RecentNoteEntry]) {
    let args = serde_wasm_bindgen::to_value(&SaveRecentNotesArgs { vault_path, entries }).unwrap();
    let _ = invoke("cache_recent_notes", args).await;
}

pub async fn load_plugins_css(vault_path: &str) -> Option<String> {
    let vault_args =
        serde_wasm_bindgen::to_value(&VaultPathArgs { vault_path }).unwrap();
    let css_val = invoke("load_plugins_css", vault_args).await;
    css_val.as_string()
}

pub async fn load_settings(vault_path: &str) -> Option<String> {
    let vault_args =
        serde_wasm_bindgen::to_value(&VaultPathArgs { vault_path }).unwrap();
    let s_val = invoke("load_settings", vault_args).await;
    s_val.as_string()
}

pub async fn save_settings(vault_path: &str, settings: &AppSettings) {
    if let Ok(s_json) = serde_json::to_string(settings) {
        let args = serde_wasm_bindgen::to_value(&SaveSettingsArgs {
            vault_path,
            settings: &s_json,
        })
        .unwrap();
        invoke("save_settings", args).await;
    }
}

pub async fn read_file(path: &str) -> Option<String> {
    let args = serde_wasm_bindgen::to_value(&ReadFileArgs { path }).unwrap();
    let text_val = invoke("read_file", args).await;
    text_val.as_string()
}

pub async fn read_file_base64(path: &str) -> Option<String> {
    let args = serde_wasm_bindgen::to_value(&ReadFileArgs { path }).unwrap();
    let value = invoke("read_file_base64", args).await;
    value.as_string()
}

pub async fn write_file(path: &str, content: &str) {
    let args = serde_wasm_bindgen::to_value(&WriteFileArgs { path, content }).unwrap();
    invoke("write_file", args).await;
}

pub async fn create_dir(path: &str) {
    let args = serde_wasm_bindgen::to_value(&ReadFileArgs { path }).unwrap();
    let _ = invoke("create_dir", args).await;
}

pub async fn delete_file(path: &str) {
    let args = serde_wasm_bindgen::to_value(&ReadFileArgs { path }).unwrap();
    let _ = invoke("delete_file", args).await;
}

pub async fn delete_dir(path: &str) {
    let args = serde_wasm_bindgen::to_value(&ReadFileArgs { path }).unwrap();
    let _ = invoke("delete_dir", args).await;
}

pub async fn rename_note(vault_path: &str, old_path: &str, new_path: &str) -> String {
    let args = serde_wasm_bindgen::to_value(&RenameNoteArgs {
        vault_path,
        old_path,
        new_path,
    })
    .unwrap();
    let result = invoke("rename_note", args).await;
    result.as_string().unwrap_or_else(|| new_path.to_string())
}

pub async fn import_obsidian_vault_with_picker() -> Option<VaultImportReport> {
    let result = invoke("import_obsidian_vault_with_picker", JsValue::NULL).await;
    serde_wasm_bindgen::from_value::<VaultImportReport>(result).ok()
}

pub async fn pick_bedrock_vault() -> Option<String> {
    let result = invoke("pick_bedrock_vault", JsValue::NULL).await;
    serde_wasm_bindgen::from_value::<Option<String>>(result)
        .ok()
        .flatten()
}

pub async fn init_vault() -> Option<String> {
    invoke("init_vault", JsValue::NULL).await.as_string()
}

pub async fn load_vault_session() -> VaultSessionState {
    let session_val = invoke("load_vault_session", JsValue::NULL).await;
    serde_wasm_bindgen::from_value::<VaultSessionState>(session_val).unwrap_or_default()
}

pub async fn save_vault_session(
    open_vaults: &[String],
    active_vault: Option<&str>,
    active_vault_recent: Option<&[RecentNoteEntry]>,
) -> VaultSessionState {
    let payload = Object::new();
    let open_value = serde_wasm_bindgen::to_value(open_vaults).unwrap_or(JsValue::NULL);
    let active_value = active_vault
        .map(JsValue::from_str)
        .unwrap_or(JsValue::NULL);

    let _ = Reflect::set(&payload, &JsValue::from_str("open_vaults"), &open_value);
    let _ = Reflect::set(&payload, &JsValue::from_str("openVaults"), &open_value);
    let _ = Reflect::set(&payload, &JsValue::from_str("active_vault"), &active_value);
    let _ = Reflect::set(&payload, &JsValue::from_str("activeVault"), &active_value);

    if let Some(entries) = active_vault_recent {
        let entries_value = serde_wasm_bindgen::to_value(entries).unwrap_or(JsValue::NULL);
        let _ = Reflect::set(
            &payload,
            &JsValue::from_str("active_vault_recent_notes"),
            &entries_value,
        );
        let _ = Reflect::set(
            &payload,
            &JsValue::from_str("activeVaultRecentNotes"),
            &entries_value,
        );
    }

    let result = invoke("save_vault_session", payload.into()).await;
    serde_wasm_bindgen::from_value::<VaultSessionState>(result).unwrap_or_default()
}

pub async fn open_settings_window() {
    let _ = invoke("open_settings_window", JsValue::NULL).await;
}

