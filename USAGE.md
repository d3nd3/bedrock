# Bedrock Usage Guide

Welcome to Bedrock! Bedrock is built to be fast, intuitive, and distraction-free while allowing power users incredible flexibility to customize their environment.

## 1. The BedrockVault
When you run Bedrock for the first time, it automatically creates a `BedrockVault` directory inside your operating system's standard `Documents` folder.
- **Mac**: `~/Documents/BedrockVault/`
- **Linux**: `~/Documents/BedrockVault/`
- **Windows**: `C:\Users\<User>\Documents\BedrockVault\`

Every note you write in Bedrock is saved as a plain `.md` (Markdown) file entirely locally inside this vault. You own your data.

## 2. Navigating the UI
The interface is split into two primary areas:

### The Sidebar (Left)
- Displays all the markdown files currently residing in your `BedrockVault`.
- Click on any file name to load its contents into the Editor pane.
- Click the **⚙ (Settings)** icon in the header to configure the application's Global Theme!

### The Editor Pane (Right)
This is the core of Bedrock. It features a lightning-fast *In-Place* Markdown Editor. 
As you type raw markdown syntax (`# Headers`, `**bold**`, `- lists`, etc) the editor will dynamically identify and style the markdown elements in-place with beautiful typography using overlapping transparent highlight layers.

## 3. Customizing the Theme (Settings UI & Plugins)
Bedrock gives you absolute control over your visual environment. 

### Built-in Settings UI
Click the `⚙` icon in the sidebar to access the built-in Settings UI. Here you can configure 14 distinct variables that instantly update the UI:
*   **Window Frame**: Editor Font Size, Accent Color, Background Primary, Background Secondary, Text Primary
*   **Markdown Core**: H1 Color, H2 Color, H3 Color, H4 Color
*   **Markdown Effects**: Bold Text Color, Italic Text Color, Code Background, Code Text Color, Blockquote Color
These variables are saved to `BedrockVault/settings.json`, and are natively hidden from your sidebar.

### Developer Plugin API
If you need deeper customization (CSS overrides for sizing, animations, or advanced selectors), Bedrock exposes a pure CSS Plugin API.
1. Navigate to your `BedrockVault`.
2. Inside, open the hidden `.plugins` directory (`BedrockVault/.plugins/`).
3. You will see a `theme.css` file. Edit this file to override *any* CSS variables or classes.
*Reload Bedrock to see your custom CSS take effect instantly.*

## 4. Keyboard Shortcuts & Markdown
Bedrock supports all standard, fast markdown entry formats.
- `# ` for H1 headings
- `## ` for H2 headings
- `*italic*` or `_italic_`
- `**bold**`
- `> ` for blockquotes
- ` ``` ` for code blocks

Writing is saved instantly to the local disk as soon as you type.

## 5. Developer Usage & Compilation

Bedrock is built entirely in Rust, using Tauri v2 for the cross-platform native wrapper and Leptos for the WebAssembly (WASM) frontend.

### Prerequisites

You will need the following installed:
1. **Rust**: ([rustup.rs](https://rustup.rs/))
   - After installing Rust, add the WASM target: `rustup target add wasm32-unknown-unknown`
2. **Trunk**: The WASM bundler used by Leptos.
   - `cargo install trunk`
3. **Tauri CLI**: The command line utility for building Tauri apps.
   - `cargo install tauri-cli`
4. **OS specific build dependencies**:
   - **Linux**: `sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`
   - **Mac/Windows**: Tauri will build out-of-the-box assuming XCode/Visual Studio build tools are present. See the [Tauri Prerequisites](https://tauri.app/start/prerequisites/) for exact details.

### Building & Running from Source

To start the local development server (with hot-reloading for UI changes):
```bash
cargo tauri dev
```

To build the optimized, static native application for your operating system:
```bash
cargo tauri build
```
The compiled binaries will be placed in `src-tauri/target/release/bundle/`.
