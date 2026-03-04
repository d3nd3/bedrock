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

