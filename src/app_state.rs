use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RecentNoteEntry {
    pub path: String,
    pub title: String,
    pub last_opened: i64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct VaultNote {
    pub path: String,
    pub content: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct VaultImportReport {
    pub success: bool,
    pub cancelled: bool,
    pub message: String,
    pub source_vault: Option<String>,
    pub destination_vault: Option<String>,
    pub scanned_notes: usize,
    pub imported_notes: usize,
    pub scanned_images: usize,
    pub imported_images: usize,
    pub renamed_notes: usize,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct VaultSessionState {
    pub open_vaults: Vec<String>,
    pub active_vault: Option<String>,
    #[serde(default)]
    pub recent_notes: HashMap<String, Vec<RecentNoteEntry>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppSettings {
    pub font_size: u32,
    pub accent_color: String,
    pub bg_primary: String,
    pub bg_secondary: String,
    pub text_primary: String,
    pub md_h1_color: String,
    pub md_h2_color: String,
    pub md_h3_color: String,
    pub md_h4_color: String,
    pub md_bold_color: String,
    pub md_italic_color: String,
    pub md_code_bg: String,
    pub md_code_text: String,
    pub md_quote_color: String,
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

