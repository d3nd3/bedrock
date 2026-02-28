# ⯁ Bedrock

**Bedrock** is a lightning-fast, premium, cross-platform markup note-taking tool. Built entirely in Rust from the ground up, it ensures maximum performance while offering incredible extensibility for aesthetic and behavioral plugins.

## Features
- **Pure Rust Architecture**: Powered by Tauri v2 and Leptos (WASM), delivering native-level speeds everywhere.
- **Cross-Platform**: Designed for macOS, Windows, Linux, iOS, and Android.
- **Local-First**: Notes are stored purely as local `.md` files in your BedrockVault. No databases, no sync servers required.
- **Advanced Markdown Editor**: Smart keybindings, auto-pairing, list continuation, toolbar actions, and live syntax stylizing.
- **Markdown Syntax Reference**: See [`MARKDOWN_SYNTAX.md`](./MARKDOWN_SYNTAX.md) for the current supported syntax list.
- **Transaction Core Engine**: Bedrock applies edits through a CM6-style state+transaction kernel (`editor_core`) for predictable, composable behavior.
- **Metadata Cache**: In-memory indexing of headings, tags, resolved links, unresolved links, and backlinks.
- **File Intelligence**: Recursive vault indexing plus rename with automatic wiki-link updates across notes.
- **Dynamic Plugin API**: Complete CSS variable system allows you to build deeply integrated themes and plugins just by dropping a `.css` file in your `.plugins` folder.

## License

Bedrock operates under a **Dual-License** model:
- **AGPL v3**: Free for open source and personal use. Any modifications or network instances must also be fully open-sourced under the AGPL.
- **Commercial License**: Required for companies or individuals who want to sell Bedrock, integrate it into commercial products, or keep code modifications private.
- **Premium UI**: Designed with clean typography, smooth transitions, and a focus on distraction-free writing.

## Development

### Prerequisites
- [Rust](https://www.rust-lang.org/)
- [Tauri v2 CLI Prerequisites](https://tauri.app/start/prerequisites/)

### Running Locally
```bash
cargo tauri dev
```
