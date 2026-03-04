use crate::path_utils::{collapse_path, normalize_rel_path, normalize_slashes};
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Debug, Default)]
pub struct HeadingCache {
    pub level: u8,
    pub text: String,
    pub line: usize,
}

#[derive(Clone, Debug, Default)]
pub struct FileCache {
    pub headings: Vec<HeadingCache>,
    pub tags: Vec<String>,
    pub links: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct MetadataCacheState {
    pub file_cache: HashMap<String, FileCache>,
    pub resolved_links: HashMap<String, HashMap<String, usize>>,
    pub unresolved_links: HashMap<String, HashMap<String, usize>>,
    pub backlinks: HashMap<String, Vec<String>>,
    pub tags_index: HashMap<String, Vec<String>>,
}

struct InlineMatch {
    start: usize,
    end: usize,
    inner_start: usize,
    inner_end: usize,
    open_len: usize,
    close_len: usize,
    class: &'static str,
    hide_tokens: bool,
    /// Block-level preview shown on-line (caret is on this line).
    preview_html: Option<String>,
    /// Inline-block preview shown off-line (caret is elsewhere).
    preview_html_inline: Option<String>,
    hide_entire_unless_caret: bool,
}

pub struct ImageRenderContext<'a> {
    pub vault_path: &'a str,
    pub current_file: &'a str,
    pub cache: &'a HashMap<String, String>,
}

fn is_escaped_at(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return false;
    }
    let mut cursor = idx;
    let mut slash_count = 0usize;
    while cursor > 0 {
        cursor -= 1;
        if bytes[cursor] == b'\\' {
            slash_count += 1;
        } else {
            break;
        }
    }
    slash_count % 2 == 1
}

fn find_delimiter_positions(text: &str, delimiter: &str) -> Vec<usize> {
    let marker = delimiter.as_bytes();
    if marker.is_empty() {
        return Vec::new();
    }

    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx + marker.len() <= bytes.len() {
        if &bytes[idx..idx + marker.len()] == marker && !is_escaped_at(bytes, idx) {
            out.push(idx);
            idx += marker.len();
        } else {
            idx += 1;
        }
    }
    out
}

fn collect_delimited_matches(
    text: &str,
    delimiter: &str,
    class: &'static str,
    hide_tokens: bool,
) -> Vec<InlineMatch> {
    let token_len = delimiter.len();
    if token_len == 0 {
        return Vec::new();
    }

    let token_positions = find_delimiter_positions(text, delimiter);
    let mut out = Vec::new();
    let mut pending_open: Option<usize> = None;
    for token in token_positions {
        if let Some(open) = pending_open.take() {
            let close = token;
            out.push(InlineMatch {
                start: open,
                end: close + token_len,
                inner_start: open + token_len,
                inner_end: close,
                open_len: token_len,
                close_len: token_len,
                class,
                hide_tokens,
                preview_html: None,
                preview_html_inline: None,
                hide_entire_unless_caret: false,
            });
        } else {
            pending_open = Some(token);
        }
    }

    if let Some(open) = pending_open {
        out.push(InlineMatch {
            start: open,
            end: text.len(),
            // Keep unmatched opening markers visible to avoid caret drift while
            // users are still typing the closing token.
            inner_start: open,
            inner_end: text.len(),
            open_len: 0,
            close_len: 0,
            class,
            hide_tokens,
            preview_html: None,
            preview_html_inline: None,
            hide_entire_unless_caret: false,
        });
    }

    out
}

fn overlaps_existing(matches: &[InlineMatch], start: usize, end: usize) -> bool {
    matches.iter().any(|x| start < x.end && end > x.start)
}

fn push_non_overlapping(matches: &mut Vec<InlineMatch>, candidate: InlineMatch) {
    if !overlaps_existing(matches, candidate.start, candidate.end) {
        matches.push(candidate);
    }
}

fn wrap_line(class: &str, html: String) -> String {
    format!("<span class=\"{class}\">{html}</span>")
}

fn sanitize_lang_id(raw: &str) -> Option<String> {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '+' || ch == '#' || ch == '-' {
            // Normalize common suffixes and separators into dashes so
            // language variants like "c++17" or "c#9.0" can still map
            // to a stable base identifier after alias normalization.
            if !out.ends_with('-') {
                out.push('-');
            }
        } else if ch.is_whitespace() {
            break;
        } else {
            // Stop at any other punctuation; info strings rarely need more.
            break;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn normalize_lang(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let mapped: &str = if matches!(lower.as_str(), "sh" | "shell" | "zsh") {
        "bash"
    } else if matches!(lower.as_str(), "js" | "node" | "nodejs") {
        "javascript"
    } else if lower == "ts" {
        "typescript"
    } else if matches!(lower.as_str(), "py" | "python3") {
        "python"
    } else if lower == "cpp" || lower.starts_with("c++") {
        "cpp"
    } else if matches!(lower.as_str(), "c#" | "cs" | "csharp") || lower.starts_with("c#") {
        "csharp"
    } else if lower == "yml" {
        "yaml"
    } else {
        &lower
    };
    sanitize_lang_id(mapped)
}

fn code_fence_open(line: &str) -> Option<(u8, usize, Option<String>)> {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let marker = bytes[0];
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let mut len = 0usize;
    while len < bytes.len() && bytes[len] == marker {
        len += 1;
    }
    if len < 3 {
        return None;
    }

    let rest = trimmed[len..].trim_start();
    let lang = if rest.is_empty() {
        None
    } else {
        // Take the first "word" as the language id.
        let first_word = rest
            .split_whitespace()
            .next()
            .unwrap_or_default();
        normalize_lang(first_word)
    };

    Some((marker, len, lang))
}

fn code_fence_close(line: &str, marker: u8, min_len: usize) -> bool {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() < min_len || bytes.first().copied() != Some(marker) {
        return false;
    }
    let mut len = 0usize;
    while len < bytes.len() && bytes[len] == marker {
        len += 1;
    }
    if len < min_len {
        return false;
    }
    trimmed[len..].trim().is_empty()
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn highlight_strings_segment(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let mut last_plain_start = 0usize;
    let len = s.len();
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut idx = 0usize;

    while idx < chars.len() {
        let (byte_idx, ch) = chars[idx];
        if ch == '"' || ch == '\'' {
            if byte_idx > last_plain_start {
                out.push_str(&escape_html(&s[last_plain_start..byte_idx]));
            }
            let delim = ch;
            let mut end_byte = len;
            idx += 1;
            while idx < chars.len() {
                let (b2, ch2) = chars[idx];
                if ch2 == delim {
                    end_byte = b2 + ch2.len_utf8();
                    idx += 1;
                    break;
                }
                idx += 1;
            }
            out.push_str("<span class=\"hl-code-token-string\">");
            out.push_str(&escape_html(&s[byte_idx..end_byte]));
            out.push_str("</span>");
            last_plain_start = end_byte;
        } else {
            idx += 1;
        }
    }

    if last_plain_start < len {
        out.push_str(&escape_html(&s[last_plain_start..]));
    }

    out
}

fn highlight_bash_code_core(line: &str) -> String {
    if line.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let trimmed_start = line.trim_start();
    let indent_len = line.len() - trimmed_start.len();
    let (indent, _) = line.split_at(indent_len);
    out.push_str(&escape_html(indent));
    let rest = trimmed_start;

    if rest.is_empty() {
        return out;
    }

    if rest.starts_with('#') {
        out.push_str("<span class=\"hl-code-token-comment\">");
        out.push_str(&escape_html(rest));
        out.push_str("</span>");
        return out;
    }

    let mut first_word_end = rest.len();
    for (idx, ch) in rest.char_indices() {
        if ch.is_whitespace() {
            first_word_end = idx;
            break;
        }
    }

    if first_word_end == 0 {
        out.push_str(&highlight_strings_segment(rest));
        return out;
    }

    let (first, after) = rest.split_at(first_word_end);
    out.push_str("<span class=\"hl-code-token-keyword\">");
    out.push_str(&escape_html(first));
    out.push_str("</span>");
    out.push_str(&highlight_strings_segment(after));

    out
}

fn highlight_code_with_singleline_comment_and_first_keyword(
    line: &str,
    comment_prefixes: &[&str],
    keywords: &[&str],
) -> String {
    if line.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let trimmed_start = line.trim_start();
    let indent_len = line.len() - trimmed_start.len();
    let (indent, _) = line.split_at(indent_len);
    out.push_str(&escape_html(indent));
    let rest = trimmed_start;

    if rest.is_empty() {
        return out;
    }

    for prefix in comment_prefixes {
        if rest.starts_with(prefix) {
            out.push_str("<span class=\"hl-code-token-comment\">");
            out.push_str(&escape_html(rest));
            out.push_str("</span>");
            return out;
        }
    }

    let mut first_word_end = rest.len();
    for (idx, ch) in rest.char_indices() {
        if ch.is_whitespace() {
            first_word_end = idx;
            break;
        }
    }

    if first_word_end == 0 {
        out.push_str(&highlight_strings_segment(rest));
        return out;
    }

    let (first, after) = rest.split_at(first_word_end);
    if keywords.iter().any(|kw| kw == &first) {
        out.push_str("<span class=\"hl-code-token-keyword\">");
        out.push_str(&escape_html(first));
        out.push_str("</span>");
    } else {
        out.push_str(&escape_html(first));
    }
    out.push_str(&highlight_strings_segment(after));

    out
}

fn highlight_python_code_core(line: &str) -> String {
    const PY_KEYWORDS: &[&str] = &[
        "def", "class", "if", "elif", "else", "for", "while", "return", "import", "from", "as",
        "try", "except", "finally", "with", "lambda", "pass", "yield", "raise", "True", "False",
        "None",
    ];
    highlight_code_with_singleline_comment_and_first_keyword(line, &["#"], PY_KEYWORDS)
}

fn highlight_c_like_code_core(line: &str, lang: &str) -> String {
    match lang {
        "javascript" | "typescript" => {
            const JS_KW: &[&str] = &[
                "const", "let", "var", "function", "if", "else", "for", "while", "return", "class",
                "extends", "import", "from", "export", "async", "await",
            ];
            highlight_code_with_singleline_comment_and_first_keyword(line, &["//"], JS_KW)
        }
        "c" => {
            const C_KW: &[&str] = &[
                "int", "char", "float", "double", "void", "if", "else", "for", "while", "return",
                "struct", "enum",
            ];
            highlight_code_with_singleline_comment_and_first_keyword(line, &["//"], C_KW)
        }
        "cpp" => {
            const CPP_KW: &[&str] = &[
                "int", "char", "float", "double", "void", "auto", "if", "else", "for", "while",
                "return", "class", "struct", "enum", "template", "typename",
            ];
            highlight_code_with_singleline_comment_and_first_keyword(line, &["//"], CPP_KW)
        }
        "csharp" => {
            const CS_KW: &[&str] = &[
                "int", "string", "bool", "var", "class", "struct", "enum", "namespace", "using",
                "public", "private", "protected", "internal", "return", "if", "else", "for",
                "while",
            ];
            highlight_code_with_singleline_comment_and_first_keyword(line, &["//"], CS_KW)
        }
        "rust" => {
            const RUST_KW: &[&str] = &[
                "fn", "let", "mut", "struct", "enum", "impl", "trait", "pub", "mod", "use",
                "match", "if", "else", "loop", "while", "for", "return",
            ];
            highlight_code_with_singleline_comment_and_first_keyword(line, &["//"], RUST_KW)
        }
        _ => {
            if line.is_empty() {
                return String::new();
            }
            let mut out = String::new();
            let trimmed_start = line.trim_start();
            let indent_len = line.len() - trimmed_start.len();
            let (indent, _) = line.split_at(indent_len);
            out.push_str(&escape_html(indent));
            out.push_str(&highlight_strings_segment(trimmed_start));
            out
        }
    }
}

fn highlight_code_line(line: &str, lang: Option<&str>) -> String {
    let (core, nl) = if let Some(stripped) = line.strip_suffix('\n') {
        (stripped, "\n")
    } else {
        (line, "")
    };

    let highlighted_core = match lang {
        Some("bash") => highlight_bash_code_core(core),
        Some("python") => highlight_python_code_core(core),
        Some("javascript") => highlight_c_like_code_core(core, "javascript"),
        Some("typescript") => highlight_c_like_code_core(core, "typescript"),
        Some("c") => highlight_c_like_code_core(core, "c"),
        Some("cpp") => highlight_c_like_code_core(core, "cpp"),
        Some("csharp") => highlight_c_like_code_core(core, "csharp"),
        Some("rust") => highlight_c_like_code_core(core, "rust"),
        _ => highlight_strings_segment(core),
    };

    format!("{highlighted_core}{nl}")
}

pub fn normalize_pasted_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\u{00A0}', " ")
}

pub fn escape_html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn strip_wiki_target(raw: &str) -> String {
    raw.trim()
        .split('|')
        .next()
        .unwrap_or_default()
        .split('#')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn strip_markdown_image_target(raw: &str) -> String {
    let trimmed = raw.trim();
    let base = if trimmed.starts_with('<') {
        trimmed
            .trim_start_matches('<')
            .split('>')
            .next()
            .unwrap_or_default()
            .trim()
    } else {
        trimmed.split_whitespace().next().unwrap_or_default().trim()
    };
    base.split('#')
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn image_extension(path: &str) -> Option<String> {
    let normalized = normalize_slashes(path);
    let ext = Path::new(&normalized)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    Some(ext)
}

fn is_supported_inline_image_path(path: &str) -> bool {
    matches!(
        image_extension(path).as_deref(),
        Some(
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
    )
}

pub fn image_mime_for_path(path: &str) -> &'static str {
    match image_extension(path).as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        Some("tif") | Some("tiff") => "image/tiff",
        Some("ico") => "image/x-icon",
        Some("avif") => "image/avif",
        Some("heic") => "image/heic",
        Some("heif") => "image/heif",
        _ => "application/octet-stream",
    }
}

pub fn looks_like_external_url(target: &str) -> bool {
    let t = target.trim().to_ascii_lowercase();
    t.contains("://") || t.starts_with("data:")
}

fn current_note_dir(note_path: &str) -> String {
    let normalized = normalize_slashes(note_path);
    normalized
        .rsplit_once('/')
        .map(|(dir, _): (&str, &str)| dir.to_string())
        .unwrap_or_default()
}

fn is_path_within(base: &str, candidate: &str) -> bool {
    let base_norm = collapse_path(base).trim_end_matches('/').to_string();
    let candidate_norm = collapse_path(candidate);
    if base_norm.is_empty() {
        return false;
    }
    candidate_norm == base_norm || candidate_norm.starts_with(&format!("{base_norm}/"))
}

pub fn image_local_candidates(vault_path: &str, note_path: &str, target: &str) -> Vec<String> {
    if target.is_empty() || !is_supported_inline_image_path(target) {
        return Vec::new();
    }
    if looks_like_external_url(target) {
        return Vec::new();
    }

    let mut out = Vec::new();
    let target_norm = normalize_slashes(target.trim());
    let vault_norm = collapse_path(vault_path);

    let is_windows_abs = {
        let b = target_norm.as_bytes();
        b.len() >= 2 && b[1] == b':'
    };
    if is_windows_abs {
        // Keep auto-render strictly within the active vault.
        return Vec::new();
    }
    if target_norm.starts_with('/') {
        out.push(collapse_path(&format!(
            "{vault_norm}/{}",
            target_norm.trim_start_matches('/')
        )));
        return out;
    }

    let note_dir = current_note_dir(note_path);

    if !note_dir.is_empty() {
        out.push(collapse_path(&format!(
            "{vault_norm}/{note_dir}/{target_norm}"
        )));
    }
    out.push(collapse_path(&format!("{vault_norm}/{target_norm}")));
    out.retain(|candidate| is_path_within(&vault_norm, candidate));
    out.sort();
    out.dedup();
    out
}

pub fn collect_image_targets_for_note(text: &str) -> Vec<(String, bool)> {
    static RE_EMBED: OnceLock<Regex> = OnceLock::new();
    static RE_MD_IMAGE: OnceLock<Regex> = OnceLock::new();

    let re_embed = RE_EMBED.get_or_init(|| Regex::new(r"!\[\[([^\]\n]+)\]\]").unwrap());
    let re_md_image =
        RE_MD_IMAGE.get_or_init(|| Regex::new(r"!\[([^\]\n]*)\]\(([^)\n]+)\)").unwrap());

    let mut out = Vec::new();
    for cap in re_embed.captures_iter(text) {
        let raw = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        out.push((strip_wiki_target(raw), true));
    }
    for cap in re_md_image.captures_iter(text) {
        let raw = cap.get(2).map(|m| m.as_str()).unwrap_or_default();
        out.push((strip_markdown_image_target(raw), false));
    }
    out
}

fn resolve_image_preview_html(
    ctx: Option<&ImageRenderContext>,
    target: &str,
    alt: Option<&str>,
    on_line: bool,
) -> Option<String> {
    if target.is_empty() || !is_supported_inline_image_path(target) {
        return None;
    }
    let src = if looks_like_external_url(target) {
        target.to_string()
    } else {
        let ctx = ctx?;
        let candidates = image_local_candidates(ctx.vault_path, ctx.current_file, target);
        let path = candidates
            .into_iter()
            .find(|candidate| ctx.cache.contains_key(candidate))?;
        ctx.cache.get(&path)?.to_string()
    };
    let alt = alt.unwrap_or("");
    let img = format!(
        "<img class=\"md-inline-image\" src=\"{}\" alt=\"{}\"/>",
        escape_html_attr(&src),
        escape_html_attr(alt)
    );
    let extra = if on_line { "" } else { " md-inline-image-inline" };
    Some(format!("<span class=\"md-inline-image-wrap{extra}\" contenteditable=\"false\" style=\"pointer-events:none;user-select:none;caret-color:transparent;\">{img}</span>"))
}

pub fn line_index_at(text: &str, byte_offset: usize) -> usize {
    let end = byte_offset.min(text.len());
    text[..end].lines().count().saturating_sub(1).max(0)
}

pub fn line_start(text: &str, line_index: usize) -> usize {
    if line_index == 0 {
        return 0;
    }
    text.match_indices('\n')
        .nth(line_index.saturating_sub(1))
        .map(|(i, _)| i + 1)
        .unwrap_or(text.len())
}

pub fn highlight_inline(
    text: &str,
    caret: Option<usize>,
    image_ctx: Option<&ImageRenderContext>,
) -> String {
    static RE_EMBED: OnceLock<Regex> = OnceLock::new();
    static RE_WIKI: OnceLock<Regex> = OnceLock::new();
    static RE_MD_LINK: OnceLock<Regex> = OnceLock::new();
    static RE_MD_IMAGE: OnceLock<Regex> = OnceLock::new();
    static RE_CODE: OnceLock<Regex> = OnceLock::new();
    static RE_INLINE_MATH: OnceLock<Regex> = OnceLock::new();
    static RE_FOOTNOTE_REF: OnceLock<Regex> = OnceLock::new();
    static RE_INLINE_FOOTNOTE: OnceLock<Regex> = OnceLock::new();
    static RE_BLOCK_ID: OnceLock<Regex> = OnceLock::new();
    static RE_TAG: OnceLock<Regex> = OnceLock::new();

    let re_embed = RE_EMBED.get_or_init(|| Regex::new(r"!\[\[([^\]\n]+)\]\]").unwrap());
    let re_wiki = RE_WIKI.get_or_init(|| Regex::new(r"\[\[([^\]]+)\]\]").unwrap());
    let re_md_link = RE_MD_LINK.get_or_init(|| Regex::new(r"\[([^\]\n]+)\]\(([^)\n]+)\)").unwrap());
    let re_md_image =
        RE_MD_IMAGE.get_or_init(|| Regex::new(r"!\[([^\]\n]*)\]\(([^)\n]+)\)").unwrap());
    let re_code = RE_CODE.get_or_init(|| Regex::new(r"`([^`\n]+)`").unwrap());
    let re_inline_math = RE_INLINE_MATH.get_or_init(|| Regex::new(r"\$([^$\n]+)\$").unwrap());
    let re_footnote_ref = RE_FOOTNOTE_REF.get_or_init(|| Regex::new(r"\[\^[^\]\n]+\]").unwrap());
    let re_inline_footnote =
        RE_INLINE_FOOTNOTE.get_or_init(|| Regex::new(r"\^\[[^\]\n]+\]").unwrap());
    let re_block_id =
        RE_BLOCK_ID.get_or_init(|| Regex::new(r"\^[A-Za-z0-9][A-Za-z0-9-]*").unwrap());
    let re_tag = RE_TAG.get_or_init(|| Regex::new(r"#[A-Za-z][A-Za-z0-9_/-]*").unwrap());

    let mut matches: Vec<InlineMatch> = Vec::new();

    // Inline code has the highest precedence.
    for cap in re_code.captures_iter(text) {
        let m = cap.get(0).unwrap();
        let inner = cap.get(1).unwrap();
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: inner.start(),
                inner_end: inner.end(),
                open_len: 1,
                close_len: 1,
                class: "hl-code",
                hide_tokens: true,
                preview_html: None,
                preview_html_inline: None,
                hide_entire_unless_caret: false,
            },
        );
    }

    // Obsidian comments: %% comment %% (including unmatched opener while typing).
    for m in collect_delimited_matches(text, "%%", "hl-comment", false) {
        push_non_overlapping(&mut matches, m);
    }

    for cap in re_embed.captures_iter(text) {
        let m = cap.get(0).unwrap();
        let inner = cap.get(1).unwrap();
        let target = strip_wiki_target(inner.as_str());
        let preview_html = resolve_image_preview_html(image_ctx, &target, None, true);
        let preview_html_inline = resolve_image_preview_html(image_ctx, &target, None, false);
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: inner.start(),
                inner_end: inner.end(),
                open_len: 3,
                close_len: 2,
                class: "hl-embed",
                hide_tokens: true,
                preview_html,
                preview_html_inline,
                hide_entire_unless_caret: true,
            },
        );
    }

    for cap in re_wiki.captures_iter(text) {
        let m = cap.get(0).unwrap();
        let inner = cap.get(1).unwrap();
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: inner.start(),
                inner_end: inner.end(),
                open_len: 2,
                close_len: 2,
                class: "hl-link",
                hide_tokens: true,
                preview_html: None,
                preview_html_inline: None,
                hide_entire_unless_caret: false,
            },
        );
    }

    for cap in re_md_image.captures_iter(text) {
        let m = cap.get(0).unwrap();
        let alt = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let raw_target = cap.get(2).map(|m| m.as_str()).unwrap_or_default();
        let target = strip_markdown_image_target(raw_target);
        let preview_html = resolve_image_preview_html(image_ctx, &target, Some(alt), true);
        let preview_html_inline = resolve_image_preview_html(image_ctx, &target, Some(alt), false);
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-embed",
                hide_tokens: false,
                preview_html,
                preview_html_inline,
                hide_entire_unless_caret: true,
            },
        );
    }

    for cap in re_md_link.captures_iter(text) {
        let m = cap.get(0).unwrap();
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-link",
                hide_tokens: false,
                preview_html: None,
                preview_html_inline: None,
                hide_entire_unless_caret: false,
            },
        );
    }

    for m in collect_delimited_matches(text, "***", "hl-bold hl-italic", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "___", "hl-bold hl-italic", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "**", "hl-bold", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "__", "hl-bold", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "~~", "hl-strike", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "==", "hl-mark", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "*", "hl-italic", true) {
        push_non_overlapping(&mut matches, m);
    }
    for m in collect_delimited_matches(text, "_", "hl-italic", true) {
        push_non_overlapping(&mut matches, m);
    }

    for cap in re_inline_math.captures_iter(text) {
        let m = cap.get(0).unwrap();
        let inner = cap.get(1).unwrap();
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: inner.start(),
                inner_end: inner.end(),
                open_len: 1,
                close_len: 1,
                class: "hl-math-inline",
                hide_tokens: true,
                preview_html: None,
                preview_html_inline: None,
                hide_entire_unless_caret: false,
            },
        );
    }

    for m in re_footnote_ref.find_iter(text) {
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-footnote",
                hide_tokens: false,
                preview_html: None,
                preview_html_inline: None,
                hide_entire_unless_caret: false,
            },
        );
    }

    for m in re_inline_footnote.find_iter(text) {
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-footnote",
                hide_tokens: false,
                preview_html: None,
                preview_html_inline: None,
                hide_entire_unless_caret: false,
            },
        );
    }

    for m in re_tag.find_iter(text) {
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-tag",
                hide_tokens: false,
                preview_html: None,
                preview_html_inline: None,
                hide_entire_unless_caret: false,
            },
        );
    }

    for m in re_block_id.find_iter(text) {
        push_non_overlapping(
            &mut matches,
            InlineMatch {
                start: m.start(),
                end: m.end(),
                inner_start: m.start(),
                inner_end: m.end(),
                open_len: 0,
                close_len: 0,
                class: "hl-block-id",
                hide_tokens: false,
                preview_html: None,
                preview_html_inline: None,
                hide_entire_unless_caret: false,
            },
        );
    }

    matches.sort_by_key(|m| m.start);
    let mut disjoint: Vec<&InlineMatch> = Vec::new();
    let mut last_end = 0usize;
    for m in &matches {
        if m.start >= last_end {
            disjoint.push(m);
            last_end = m.end;
        }
    }

    let mut out = String::new();
    let mut pos = 0usize;
    let caret_line = caret.map(|c| line_index_at(text, c));
    for m in disjoint {
        out.push_str(&escape_html(&text[pos..m.start]));
        let caret_inside = caret.map(|c| c >= m.start && c <= m.end).unwrap_or(false);
        let match_line = line_index_at(text, m.start);
        let caret_in_image_line = caret_line.map(|cl| cl == match_line).unwrap_or(false);

        if m.hide_entire_unless_caret {
            out.push_str("<span class=\"md-embed");
            if caret_in_image_line {
                out.push_str(" md-embed-caret-on-line");
            }
            out.push_str("\">");
            out.push_str("<span class=\"md-embed-source\">");
            out.push_str(&escape_html(&text[m.start..m.end]));
            out.push_str("</span>");
            if caret_in_image_line {
                out.push_str("<span class=\"md-embed-line-end\" contenteditable=\"false\"></span>");
            }
            let preview = if caret_in_image_line {
                &m.preview_html
            } else {
                &m.preview_html_inline
            };
            if let Some(p) = preview {
                out.push_str(p);
            }
            out.push_str("</span>");
        } else if caret_inside && m.hide_tokens {
            // Keep live formatting active while caret is inside the markdown span,
            // but reveal the wrapper tokens for accurate editing context.
            out.push_str("<span class=\"md-token md-token-visible\">");
            out.push_str(&escape_html(&text[m.start..m.start + m.open_len]));
            out.push_str("</span><span class=\"");
            out.push_str(m.class);
            out.push_str("\">");
            out.push_str(&escape_html(&text[m.inner_start..m.inner_end]));
            out.push_str("</span><span class=\"md-token md-token-visible\">");
            out.push_str(&escape_html(&text[m.end - m.close_len..m.end]));
            out.push_str("</span>");
        } else if caret_inside {
            out.push_str(&escape_html(&text[m.start..m.end]));
        } else if m.hide_tokens {
            out.push_str("<span class=\"md-token md-token-hidden\">");
            out.push_str(&escape_html(&text[m.start..m.start + m.open_len]));
            out.push_str("</span><span class=\"");
            out.push_str(m.class);
            out.push_str("\">");
            out.push_str(&escape_html(&text[m.inner_start..m.inner_end]));
            out.push_str("</span><span class=\"md-token md-token-hidden\">");
            out.push_str(&escape_html(&text[m.end - m.close_len..m.end]));
            out.push_str("</span>");
        } else {
            out.push_str("<span class=\"");
            out.push_str(m.class);
            out.push_str("\">");
            out.push_str(&escape_html(&text[m.start..m.end]));
            out.push_str("</span>");
        }
        if let Some(preview) = &m.preview_html {
            if !m.hide_entire_unless_caret {
                out.push_str(preview);
            }
        }
        pos = m.end;
    }
    out.push_str(&escape_html(&text[pos..]));
    out
}

pub fn highlight_markdown(
    text: &str,
    caret: Option<usize>,
    image_ctx: Option<&ImageRenderContext>,
) -> String {
    static RE_HEADING: OnceLock<Regex> = OnceLock::new();
    static RE_CALLOUT: OnceLock<Regex> = OnceLock::new();
    static RE_QUOTE: OnceLock<Regex> = OnceLock::new();
    static RE_TASK: OnceLock<Regex> = OnceLock::new();
    static RE_LIST: OnceLock<Regex> = OnceLock::new();
    static RE_ORDERED: OnceLock<Regex> = OnceLock::new();
    static RE_HR: OnceLock<Regex> = OnceLock::new();
    static RE_TABLE_ROW: OnceLock<Regex> = OnceLock::new();
    static RE_TABLE_SEPARATOR: OnceLock<Regex> = OnceLock::new();
    static RE_FOOTNOTE_DEF: OnceLock<Regex> = OnceLock::new();

    let re_heading = RE_HEADING.get_or_init(|| Regex::new(r"^(#{1,6})[^\S\n]+.*$").unwrap());
    let re_callout =
        RE_CALLOUT.get_or_init(|| Regex::new(r"^\s*>\s*\[![A-Za-z0-9-]+\][+-]?\s*.*$").unwrap());
    let re_quote = RE_QUOTE.get_or_init(|| Regex::new(r"^\s*>\s+.*$").unwrap());
    let re_task = RE_TASK.get_or_init(|| Regex::new(r"^\s*[-*+]\s+\[(?: |x|X)\]\s+.*$").unwrap());
    let re_list = RE_LIST.get_or_init(|| Regex::new(r"^\s*[-*+]\s+.*$").unwrap());
    let re_ordered = RE_ORDERED.get_or_init(|| Regex::new(r"^\s*\d+[.)]\s+.*$").unwrap());
    let re_hr = RE_HR.get_or_init(|| {
        Regex::new(r"^\s{0,3}(?:(?:\*[\t ]*){3,}|(?:-[\t ]*){3,}|(?:_[\t ]*){3,})\s*$").unwrap()
    });
    let re_table_row = RE_TABLE_ROW.get_or_init(|| Regex::new(r"^\s*\|.*\|\s*$").unwrap());
    let re_table_separator = RE_TABLE_SEPARATOR.get_or_init(|| {
        Regex::new(r"^\s*\|?(?:\s*:?-{3,}:?\s*\|)+\s*:?-{3,}:?\s*\|?\s*$").unwrap()
    });
    let re_footnote_def =
        RE_FOOTNOTE_DEF.get_or_init(|| Regex::new(r"^\s*\[\^[^\]]+\]:\s+.*$").unwrap());

    let mut out = String::new();
    let mut offset = 0usize;
    let mut in_frontmatter = false;
    let mut frontmatter_possible = true;
    let mut in_math_block = false;
    let mut in_comment_block = false;
    let mut code_fence: Option<(u8, usize, Option<String>)> = None;

    for line in text.split_inclusive('\n') {
        let line_len = line.len();
        let line_without_nl = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = line_without_nl.trim();

        if let Some((marker, min_len, ref lang)) = code_fence {
            let is_close = code_fence_close(line_without_nl, marker, min_len);
            if is_close {
                let base = "hl-codeblock hl-code-fence";
                let class = if let Some(ref lang_id) = lang {
                    format!("{base} hl-code-lang-{lang_id}")
                } else {
                    base.to_string()
                };
                out.push_str(&wrap_line(&class, escape_html(line)));
                code_fence = None;
            } else {
                let class = if let Some(ref lang_id) = lang {
                    format!("hl-codeblock hl-code-lang-{lang_id}")
                } else {
                    "hl-codeblock".to_string()
                };
                let inner = highlight_code_line(line, lang.as_deref());
                out.push_str(&wrap_line(&class, inner));
            }
            offset += line_len;
            continue;
        }

        if in_math_block {
            out.push_str(&wrap_line("hl-math-block", escape_html(line)));
            if trimmed == "$$" {
                in_math_block = false;
            }
            offset += line_len;
            continue;
        }

        if in_frontmatter {
            out.push_str(&wrap_line("hl-frontmatter", escape_html(line)));
            if trimmed == "---" || trimmed == "..." {
                in_frontmatter = false;
                frontmatter_possible = false;
            }
            offset += line_len;
            continue;
        }

        if in_comment_block {
            out.push_str(&wrap_line("hl-comment", escape_html(line)));
            if line_without_nl.matches("%%").count() % 2 == 1 {
                in_comment_block = false;
            }
            offset += line_len;
            continue;
        }

        if frontmatter_possible {
            if trimmed == "---" {
                out.push_str(&wrap_line("hl-frontmatter", escape_html(line)));
                in_frontmatter = true;
                offset += line_len;
                continue;
            }
            if !trimmed.is_empty() {
                frontmatter_possible = false;
            }
        }

        if let Some((marker, len, lang)) = code_fence_open(line_without_nl) {
            let base = "hl-codeblock hl-code-fence";
            let class = if let Some(ref lang_id) = lang {
                format!("{base} hl-code-lang-{lang_id}")
            } else {
                base.to_string()
            };
            out.push_str(&wrap_line(&class, escape_html(line)));
            code_fence = Some((marker, len, lang));
            offset += line_len;
            continue;
        }

        if trimmed == "$$" {
            out.push_str(&wrap_line("hl-math-block", escape_html(line)));
            in_math_block = true;
            offset += line_len;
            continue;
        }

        let line_end = offset + line_len;
        let caret_rel = caret
            .filter(|c| *c >= offset && (*c < line_end || (*c == line_end && line_end == text.len())))
            .map(|c| c - offset);
        let line_html = highlight_inline(line, caret_rel, image_ctx);

        let wrapped = if re_callout.is_match(line_without_nl) {
            wrap_line("hl-callout", line_html)
        } else if let Some(cap) = re_heading.captures(line_without_nl) {
            let level = cap.get(1).map(|m| m.as_str().len()).unwrap_or(1);
            match level {
                1 => wrap_line("hl-h1", line_html),
                2 => wrap_line("hl-h2", line_html),
                3 => wrap_line("hl-h3", line_html),
                4 => wrap_line("hl-h4", line_html),
                5 => wrap_line("hl-h5", line_html),
                _ => wrap_line("hl-h6", line_html),
            }
        } else if re_hr.is_match(line_without_nl) {
            wrap_line("hl-hr", line_html)
        } else if re_footnote_def.is_match(line_without_nl) {
            wrap_line("hl-footnote-def", line_html)
        } else if re_quote.is_match(line_without_nl) {
            wrap_line("hl-quote", line_html)
        } else if re_task.is_match(line_without_nl) {
            wrap_line("hl-task", line_html)
        } else if re_ordered.is_match(line_without_nl) || re_list.is_match(line_without_nl) {
            wrap_line("hl-list", line_html)
        } else if re_table_separator.is_match(line_without_nl)
            || re_table_row.is_match(line_without_nl)
        {
            wrap_line("hl-table", line_html)
        } else {
            line_html
        };
        out.push_str(&wrapped);

        if line_without_nl.matches("%%").count() % 2 == 1 {
            in_comment_block = true;
        }

        offset += line_len;
    }

    out
}

pub fn highlight_markdown_for_editor(
    text: &str,
    caret: Option<usize>,
    vault_path: &str,
    current_file: &str,
    image_cache: &HashMap<String, String>,
) -> String {
    if vault_path.is_empty() || current_file.is_empty() {
        return highlight_markdown(text, caret, None);
    }
    let ctx = ImageRenderContext {
        vault_path,
        current_file,
        cache: image_cache,
    };
    highlight_markdown(text, caret, Some(&ctx))
}

pub fn extract_file_cache(text: &str) -> FileCache {
    static RE_HEADING: OnceLock<Regex> = OnceLock::new();
    static RE_WIKI: OnceLock<Regex> = OnceLock::new();
    static RE_MD_LINK: OnceLock<Regex> = OnceLock::new();
    static RE_TAG: OnceLock<Regex> = OnceLock::new();

    let re_heading = RE_HEADING.get_or_init(|| Regex::new(r"^(#{1,6})[ \t]+(.+?)\s*$").unwrap());
    let re_wiki = RE_WIKI.get_or_init(|| Regex::new(r"\[\[([^\]]+)\]\]").unwrap());
    let re_md_link = RE_MD_LINK.get_or_init(|| Regex::new(r"!?\[[^\]\n]*\]\(([^)\n]+)\)").unwrap());
    let re_tag = RE_TAG.get_or_init(|| Regex::new(r"#[A-Za-z][A-Za-z0-9_/-]*").unwrap());

    let mut headings = Vec::new();
    let mut tags = Vec::new();
    let mut links = Vec::new();

    for (idx, line) in text.lines().enumerate() {
        if let Some(cap) = re_heading.captures(line) {
            let level = cap.get(1).map(|m| m.as_str().len()).unwrap_or(1) as u8;
            let text = cap
                .get(2)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default();
            headings.push(HeadingCache {
                level,
                text,
                line: idx + 1,
            });
        }
        for tag in re_tag.find_iter(line) {
            tags.push(tag.as_str().trim_start_matches('#').to_ascii_lowercase());
        }
    }

    for cap in re_wiki.captures_iter(text) {
        let raw_inner = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        let left = raw_inner.split('|').next().unwrap_or_default();
        let link = left.split('#').next().unwrap_or_default().trim();
        if !link.is_empty() {
            links.push(normalize_rel_path(link));
        }
    }

    for cap in re_md_link.captures_iter(text) {
        let raw_target = cap.get(1).map(|m| m.as_str()).unwrap_or_default().trim();
        let target = raw_target.trim_matches('<').trim_matches('>');
        if target.is_empty() || target.starts_with('#') {
            continue;
        }
        let lowered = target.to_ascii_lowercase();
        if lowered.contains("://") || lowered.starts_with("mailto:") {
            continue;
        }
        let cleaned = target
            .split('#')
            .next()
            .unwrap_or_default()
            .split('?')
            .next()
            .unwrap_or_default()
            .trim();
        if !cleaned.is_empty() {
            links.push(normalize_rel_path(cleaned));
        }
    }

    tags.sort();
    tags.dedup();
    links.sort();
    links.dedup();

    FileCache {
        headings,
        tags,
        links,
    }
}

fn resolve_linkpath(
    linkpath: &str,
    source_path: &str,
    file_lookup: &HashMap<String, String>,
    stem_lookup: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let raw = normalize_rel_path(linkpath);
    if raw.is_empty() {
        return None;
    }

    let raw_has_ext = raw.to_ascii_lowercase().ends_with(".md");
    let mut candidates: Vec<String> = Vec::new();
    candidates.push(if raw_has_ext {
        raw.clone()
    } else {
        format!("{raw}.md")
    });

    if raw.contains('/') {
        let source_dir = source_path
            .rsplit_once('/')
            .map(|(dir, _)| dir)
            .unwrap_or("");
        if !source_dir.is_empty() {
            let joined = normalize_rel_path(&format!("{source_dir}/{raw}"));
            candidates.push(if raw_has_ext {
                joined.clone()
            } else {
                format!("{joined}.md")
            });
        }
    }

    for candidate in candidates {
        let key = candidate.to_ascii_lowercase();
        if let Some(found) = file_lookup.get(&key) {
            return Some(found.clone());
        }
    }

    let stem = Path::new(&raw)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&raw)
        .to_ascii_lowercase();

    if let Some(candidates) = stem_lookup.get(&stem) {
        if candidates.len() == 1 {
            return candidates.first().cloned();
        }
    }

    None
}

pub fn build_metadata_cache(
    notes: &HashMap<String, String>,
    files: &[String],
) -> MetadataCacheState {
    let mut state = MetadataCacheState::default();
    let mut file_lookup = HashMap::new();
    let mut stem_lookup: HashMap<String, Vec<String>> = HashMap::new();

    for path in files {
        file_lookup.insert(path.to_ascii_lowercase(), path.clone());
        let stem = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
            .to_ascii_lowercase();
        stem_lookup.entry(stem).or_default().push(path.clone());

        let text = notes.get(path).cloned().unwrap_or_default();
        let cache = extract_file_cache(&text);
        for tag in &cache.tags {
            state
                .tags_index
                .entry(tag.clone())
                .or_default()
                .push(path.clone());
        }
        state.file_cache.insert(path.clone(), cache);
    }

    for path in files {
        let cache = state.file_cache.get(path).cloned().unwrap_or_default();
        for link in cache.links {
            if let Some(target) = resolve_linkpath(&link, path, &file_lookup, &stem_lookup) {
                let by_source = state.resolved_links.entry(path.clone()).or_default();
                *by_source.entry(target.clone()).or_insert(0) += 1;
                state
                    .backlinks
                    .entry(target)
                    .or_default()
                    .push(path.clone());
            } else {
                let by_source = state.unresolved_links.entry(path.clone()).or_default();
                *by_source.entry(link).or_insert(0) += 1;
            }
        }
    }

    for files_for_tag in state.tags_index.values_mut() {
        files_for_tag.sort();
        files_for_tag.dedup();
    }
    for backlink_sources in state.backlinks.values_mut() {
        backlink_sources.sort();
        backlink_sources.dedup();
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_has(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected substring `{needle}` in:\n{haystack}"
        );
    }

    #[test]
    fn highlights_obsidian_inline_emphasis_variants() {
        let html = highlight_inline(
            "**bold** __bold2__ *it* _it2_ ~~gone~~ ==mark== ***both*** ___both2___",
            None,
            None,
        );
        assert_has(&html, "hl-bold");
        assert_has(&html, "hl-italic");
        assert_has(&html, "hl-strike");
        assert_has(&html, "hl-mark");
        assert_has(&html, "hl-bold hl-italic");
    }

    #[test]
    fn highlights_obsidian_links_embeds_tags_and_blocks() {
        let html = highlight_inline(
            "[[Note]] ![[Asset.png]] [Label](Note.md) ![img](Img.png) #tag ^block-id",
            None,
            None,
        );
        assert_has(&html, "hl-link");
        assert_has(&html, "md-embed");
        assert_has(&html, "hl-tag");
        assert_has(&html, "hl-block-id");
    }

    #[test]
    fn highlights_obsidian_comments_footnotes_and_math() {
        let html = highlight_inline(
            "%%comment%% Ref[^1] inline ^[note] and $e^{i\\pi}+1=0$",
            None,
            None,
        );
        assert_has(&html, "hl-comment");
        assert_has(&html, "hl-footnote");
        assert_has(&html, "hl-math-inline");
    }

    #[test]
    fn highlights_headings_h1_to_h6() {
        let html = highlight_markdown(
            "# h1\n## h2\n### h3\n#### h4\n##### h5\n###### h6\n".trim_end_matches('\n'),
            None,
            None,
        );
        assert_has(&html, "hl-h1");
        assert_has(&html, "hl-h2");
        assert_has(&html, "hl-h3");
        assert_has(&html, "hl-h4");
        assert_has(&html, "hl-h5");
        assert_has(&html, "hl-h6");
    }

    #[test]
    fn highlights_callouts_lists_quotes_and_hr() {
        let html = highlight_markdown(
            "> [!note] Title\n> quote\n- [ ] task\n1) one\n- item\n---\n",
            None,
            None,
        );
        assert_has(&html, "hl-callout");
        assert_has(&html, "hl-quote");
        assert_has(&html, "hl-task");
        assert_has(&html, "hl-list");
        assert_has(&html, "hl-hr");
    }

    #[test]
    fn highlights_tables_code_fences_math_blocks_frontmatter_and_comments() {
        let html = highlight_markdown(
            "---\ntitle: Bedrock\n---\n| a | b |\n| --- | --- |\n```rust\nlet x = 1;\n```\n$$\na+b\n$$\n%%\ncomment block\n%%\n",
            None,
            None,
        );
        assert_has(&html, "hl-frontmatter");
        assert_has(&html, "hl-table");
        assert_has(&html, "hl-codeblock");
        assert_has(&html, "hl-code-fence");
        assert_has(&html, "hl-math-block");
        assert_has(&html, "hl-comment");
    }

    #[test]
    fn highlights_code_fence_language_class() {
        let html = highlight_markdown("```bash\necho hi\n```\n", None, None);
        assert_has(&html, "hl-code-lang-bash");
    }

    #[test]
    fn normalizes_additional_code_fence_languages() {
        let html_py = highlight_markdown("```python\nprint('hi')\n```\n", None, None);
        assert_has(&html_py, "hl-code-lang-python");

        let html_js = highlight_markdown("```javascript\nconsole.log('hi');\n```\n", None, None);
        assert_has(&html_js, "hl-code-lang-javascript");

        let html_c = highlight_markdown("```c\nint x = 1;\n```\n", None, None);
        assert_has(&html_c, "hl-code-lang-c");

        let html_cpp = highlight_markdown("```c++\nint x = 1;\n```\n", None, None);
        assert_has(&html_cpp, "hl-code-lang-cpp");

        let html_cs = highlight_markdown("```c#\nint x = 1;\n```\n", None, None);
        assert_has(&html_cs, "hl-code-lang-csharp");
    }

    #[test]
    fn highlights_footnote_definitions() {
        let html = highlight_markdown("[^note]: footnote text\n", None, None);
        assert_has(&html, "hl-footnote-def");
    }

    #[test]
    fn metadata_extracts_wikilinks_and_markdown_links() {
        let cache = extract_file_cache(
            "[[Wiki Note]]\n[md](Folder/Note.md)\n![img](Image.png)\n[ext](https://example.com)\n",
        );
        assert!(cache.links.iter().any(|link| link == "Wiki Note"));
        assert!(cache.links.iter().any(|link| link == "Folder/Note.md"));
        assert!(cache.links.iter().any(|link| link == "Image.png"));
        assert!(!cache.links.iter().any(|link| link.contains("https://")));
    }

    #[test]
    fn inline_delimiters_respect_escaping() {
        let html = highlight_inline(r"\*\*literal\*\* and **bold**", None, None);
        assert_has(&html, "hl-bold");
        assert_has(&html, r"\*\*literal\*\*");
    }

    #[test]
    fn fenced_code_blocks_highlight_tokens_for_common_languages() {
        // Python: strings inside fenced blocks should be highlighted.
        let html_py = highlight_markdown("```python\nprint(\"hi\")\n```\n", None, None);
        assert_has(&html_py, "hl-code-lang-python");
        assert_has(&html_py, "hl-code-token-string");

        // JavaScript: keywords and strings should both be tokenized.
        let html_js =
            highlight_markdown("```javascript\nconst msg = \"hi\";\n```\n", None, None);
        assert_has(&html_js, "hl-code-lang-javascript");
        assert_has(&html_js, "hl-code-token-keyword");
        assert_has(&html_js, "hl-code-token-string");

        // C: basic keywords like `int` should be recognized.
        let html_c = highlight_markdown("```c\nint x = 1;\n```\n", None, None);
        assert_has(&html_c, "hl-code-lang-c");
        assert_has(&html_c, "hl-code-token-keyword");

        // C++: the same keyword machinery should apply.
        let html_cpp = highlight_markdown("```c++\nint x = 1;\n```\n", None, None);
        assert_has(&html_cpp, "hl-code-lang-cpp");
        assert_has(&html_cpp, "hl-code-token-keyword");

        // Rust: `fn` should be treated as a keyword.
        let html_rust = highlight_markdown("```rust\nfn main() {}\n```\n", None, None);
        assert_has(&html_rust, "hl-code-lang-rust");
        assert_has(&html_rust, "hl-code-token-keyword");
    }
}

