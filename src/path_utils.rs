pub fn normalize_slashes(path: &str) -> String {
    path.replace('\\', "/")
}

pub fn collapse_path(path: &str) -> String {
    let normalized = normalize_slashes(path);
    let bytes = normalized.as_bytes();
    let (prefix, rest) = if normalized.starts_with('/') {
        (
            "/".to_string(),
            normalized.trim_start_matches('/').to_string(),
        )
    } else if bytes.len() >= 2 && bytes[1] == b':' {
        let drive = normalized[..2].to_string();
        let tail = normalized[2..].trim_start_matches('/').to_string();
        (format!("{drive}/"), tail)
    } else {
        (String::new(), normalized)
    };

    let mut parts: Vec<&str> = Vec::new();
    for segment in rest.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            if !parts.is_empty() {
                parts.pop();
            }
            continue;
        }
        parts.push(segment);
    }

    let joined = parts.join("/");
    if prefix.is_empty() {
        joined
    } else {
        format!("{prefix}{joined}")
    }
}

pub fn normalize_rel_path(path: &str) -> String {
    path.trim().replace('\\', "/").trim_matches('/').to_string()
}

/// Canonical key for note-colors map so that "MentalHealth" (Obsidian) and "Mental-health" (filesystem) match.
/// Each segment is lowercased and has hyphens/underscores removed.
pub fn canonical_note_color_key(path: &str) -> String {
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

/// Look up label color for a path, cascading from file to parent folders so that when only a folder
/// has a color in the map (e.g. from obsidian-file-color data.json), files under that folder get
/// the same color. Tries the path's canonical key, then parent, then grandparent, etc.
pub fn resolve_note_color(colors: &std::collections::HashMap<String, String>, path: &str) -> Option<String> {
    let mut current = normalize_rel_path(path);
    loop {
        let key = canonical_note_color_key(&current);
        if let Some(hex) = colors.get(&key) {
            return Some(hex.clone());
        }
        if current.is_empty() {
            return None;
        }
        let parts: Vec<&str> = current.split('/').collect();
        if parts.len() <= 1 {
            return None;
        }
        current = parts[..parts.len() - 1].join("/");
    }
}

pub fn vault_display_name(path: &str) -> String {
    let normalized = normalize_slashes(path.trim().trim_end_matches('/'));
    normalized
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("Vault")
        .to_string()
}

pub fn file_display_name(path: &str) -> String {
    let normalized = normalize_slashes(path);
    normalized
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

