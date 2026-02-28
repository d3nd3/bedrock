# Bedrock Markdown Syntax

This document lists the markdown syntax currently supported by Bedrock's live editor/highlighter and metadata indexer.

## Inline Syntax

- Bold: `**bold**`, `__bold__`
- Italic: `*italic*`, `_italic_`
- Bold + Italic: `***both***`, `___both___`
- Strikethrough: `~~struck~~`
- Highlight: `==highlight==`
- Inline code: `` `code` ``
- Inline math: `$a^2 + b^2 = c^2$`
- Wiki links: `[[Note Name]]`
- Embeds: `![[image.png]]`
- Markdown links: `[label](Note.md)`
- Markdown images: `![alt](image.png)`
- Tags: `#tag`
- Footnote references: `[^note]`
- Inline footnotes: `^[inline note text]`
- Block IDs: `^block-id`
- Obsidian comments: `%%comment%%`
- Escaped delimiters: `\*literal asterisks\*` (escape-aware parsing)

## Block Syntax

- YAML frontmatter (top-of-file): 
  - Start: `---`
  - End: `---` or `...`
- Headings: `#` through `######`
- Blockquotes: `> quote`
- Callouts: `> [!note] Title` (supports `+` / `-` fold marker style)
- Task items: `- [ ] task`, `- [x] done`
- Unordered lists: `- item`, `* item`, `+ item`
- Ordered lists: `1. item`, `1) item`
- Horizontal rules: `---`, `***`, `___`
- Pipe tables:
  - `| col | col |`
  - `| --- | --- |`
- Fenced code blocks:
  - Backticks: ```` ```lang ... ``` ````
  - Tildes: `~~~lang ... ~~~`
- Math blocks:
  - Start/end fence: `$$`
- Footnote definitions: `[^id]: definition`
- Multi-line Obsidian comments:
  - `%%`
  - `comment lines`
  - `%%`

## Metadata Indexing

Bedrock's metadata cache currently indexes:

- Headings (with level and line number)
- Tags
- Wiki links (`[[...]]`)
- Markdown links/images (`[...](...)`, `![...](...)`) for local targets
- Resolved links, unresolved links, and backlinks

External URL markdown links (for example `https://...` and `mailto:...`) are ignored by local note resolution.

## Editing Behavior Notes

- Markdown markers are hidden by default outside the active formatting span.
- Markers become visible when the caret is inside that span.
- Toolbar toggle:
  - `Show Markdown` reveals marker tokens globally.
  - `Hide Markdown` returns to live-preview behavior.
