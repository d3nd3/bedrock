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
As you type raw markdown syntax (`# Headers`, `**bold**`, `- lists`, etc) the editor dynamically styles markdown in a single editable surface so caret/selection geometry always matches rendered typography.

### Advanced Editing (Bedrock Native)
- **Smart keybindings**:
  - `Cmd/Ctrl + B`: Wrap selection with `**bold**`
  - `Cmd/Ctrl + I`: Wrap selection with `*italic*`
  - `Cmd/Ctrl + K`: Wrap selection with `[[wikilink]]`
  - `Tab` / `Shift+Tab`: Indent and outdent current line or selected block
  - `Enter` on list/quote/task lines continues the structure automatically
- **Auto-pairing** for `()`, `[]`, `{}`, quotes, and backticks.
- **Formatting toolbar** above the editor for quick markdown actions.
- **Syntax visibility toggle** in the toolbar:
  - Default: markdown markers are hidden unless the caret is inside that markdown span.
  - Optional: click **Show Markdown** to reveal marker tokens more explicitly.
- **Debounced safe-save** behavior while typing (shows `Saving...` / `Saved` status).
- **Rename note support** from the note header with automatic wiki-link rewrites across the vault.
- **Core edit kernel**: editor commands are applied through a transaction-based `editor_core`, giving consistent behavior across shortcuts, toolbar, and future plugins.
- **IME + Paste hardened**: composition input is handled safely (no mid-composition re-render jumps), and paste is normalized to plain text through the transaction pipeline.

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

The right-side **Metadata Cache** panel continuously indexes:
- tags
- headings (with line numbers)
- outgoing links
- backlinks
- unresolved links

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

## 6. Verification Scenarios (State-Cameraman)

When capturing for **caret / image-link** verification, the screenshot must show the **editor with a note open**, not the empty "Select a note from the sidebar to start editing." view. Submitting a capture of the initial empty state is invalid; the TESTER will report NOT SOLVED.

**Pre-capture checklist (caret/image goals):** Before calling Capture, confirm: (1) A note is open and the editor pane shows editable text. (2) The note body contains exactly the three lines below. (3) The caret has been moved onto the middle line (e.g. Up → Up → Down → Up). (4) You see a single caret on or beside the image markdown line. Only then take the screenshot and save to `/tmp/cursor_capture.png`.

### Caret on image markdown line
1. **Open a note**: Editor must show editable content. If the vault is empty, create a note (e.g. **New note** or add `test.md`) and open it. **Tip:** **Cmd+1** / **Ctrl+1** opens the first note in the sidebar and focuses the editor; opening any note from the sidebar also focuses the editor.
2. **Set content (exactly)**: The note body must contain **exactly** these three lines, in this order, with no extra characters or brackets:
   - `line above`
   - `![x](https://example.com/a.png)`
   - `line below`

   If the editor auto-inserts extra `[` / `]` / `)` characters or otherwise modifies the text while typing or pasting, **manually correct it** so the three lines match the strings above **visually** before you continue. A capture where the image line is malformed (e.g. missing `![` or the final `)`, or with extra `]` characters) or where "line above" / "line below" appear on the same line will be rejected.
3. **Move caret onto image line**: Focus the editor, then press **Up → Up → Down → Up** so the caret is on the middle (image) line.
4. **Capture**: Take the screenshot **only after** the editor shows these three separate lines and the caret is visible on/near the image markdown line. Save to `/tmp/cursor_capture.png`.

## 7. File-pane regression harness (State-Cameraman)

For the intermittent "**empty file pane on startup**" bug, the camera agent must loop Bedrock launches, detect the failure state, and clearly report the result.

1. **Single session, repeated launches**  
   - Call `session_start` once.  
   - In a loop (e.g. at least 20 iterations):  
     - Use `launch_app` to start Bedrock.  
     - Wait until the main window is focused, a vault is active (status does not say "No active vault"), and the left sidebar **Files** tab is active.
2. **Failure predicate (when to capture)**  
   - Inspect the file-list region under the **Files** tab.  
   - **Failure state** = active vault + **Files** tab selected + file-list region has **no child items** (no files/folders shown).  
   - On the **first** failure hit:  
     - Run `spectacle -ban -o /tmp/cursor_capture.png`.  
     - Stop the loop and report which launch index triggered the bug.
3. **If the bug does not reproduce**  
   - After all iterations without failure, capture the final healthy run with `spectacle -ban -o /tmp/cursor_capture.png`.  
   - In the message, state the launch count, e.g. `20 launches, empty file pane bug not reproduced`.

The TESTER will treat "N launches without failure" plus the final screenshot as evidence that the bug **could not be reproduced** under this harness.

## 8. Recent-notes persistence harness (State-Cameraman)

For the "**Recent notes pane list is not persisting across launches**" goal, the camera agent must prove that a note opened in one run still appears in the **Recent notes** tab after a full app restart.

**Harness run options (choose one that shows a window after “restart”):**

- **Option A — Outside isolated session (preferred for full restart):**  
  Run the steps below in an environment where a real second process can start (e.g. normal desktop, or two separate runs). Do **not** rely on “close window then `launch_app` again” inside a single isolated KWin session with `cargo tauri dev`, because the first dev process may still be running and the second launch may never show a window.  
  - Manual variant: (1) Start Bedrock (`cargo tauri dev` or the built binary), open a note, close the app. (2) Start Bedrock again. (3) Open the **Recent notes** tab and capture.  
  - If using the built binary: first launch can be `cargo tauri dev` (or the binary); after close the process exits. Second launch: run the **binary** from `src-tauri/target/release/` (or `cargo tauri dev` in a new terminal after stopping the first) so the second window appears.

- **Option B — Single-session fallback (when Option A is not possible):**  
  In a single isolated session: one launch, **open a note from the Files tab so that its contents are visible in the center editor and the Metadata Cache panel refers to that note**, then switch to the **Recent notes** tab and **confirm that at least one entry appears in the list** (not “No recent notes.”) before capturing. **Document clearly** in the handoff: “Single-session run; full app restart was not performed. This capture only shows that the Recent notes list is populated in-session; persistence across process restarts was not verified.”

**Steps (when doing a full restart, e.g. Option A):**

1. **Prepare a vault with notes**  
   - Launch Bedrock and ensure a vault is active.  
   - If the vault has no notes, create a simple markdown note (for example `test-persistence.md`) so that it appears under the **Files** tab.
2. **Populate the Recent notes list**  
   - In the left sidebar, ensure the **Files** tab is selected.  
   - Click a note (e.g. `test-persistence.md`) so it opens in the editor.  
   - Confirm that switching to the **Recent notes** tab shows this note in the recent list.
3. **Restart the application**  
   - Close the Bedrock window so the app fully exits.  
   - Relaunch Bedrock in the same environment (same user home and vault location).  
   - Wait until the main window is focused and a vault is active.
4. **Verify Recent notes persistence**  
   - In the left sidebar, explicitly click the **Recent notes** tab so that it is selected (not **Files**).  
   - Confirm that the note you opened before restart is visible in this **Recent notes** list.
5. **Capture proof for the TESTER**  
   - With the **Recent notes** tab selected and the previously opened note visible in the list, take a screenshot with `spectacle -ban -o /tmp/cursor_capture.png`.  
   - **Capture validity**: The screenshot must show the Bedrock window and UI (not a black or blank frame). When using kwin-mcp: call `focus_window` with app name `"tauri-app"` so the Bedrock window is active, wait 1–2 seconds, then take the screenshot (use `active_window_only: true` if available so only the Bedrock window is captured).  
   - This screenshot must clearly show:
     - The **Recent notes** tab highlighted.  
     - The note you opened before restart listed under Recent notes.  
     - For Option B runs, the capture should additionally demonstrate that a note was actually opened in-session (center editor not showing “Select a note from the sidebar”, and the Metadata Cache referring to the opened note) so that in-session population of the Recent list is clearly evidenced.

> **Invalid capture example (will be treated as NOT SOLVED):** The **Recent notes** tab is selected, but the list shows **“No recent notes.”**, and the main editor still displays **“Select a note from the sidebar to start editing.”** with no note content visible. This screenshot does **not** prove that the Recent list populated in-session or that it persisted across restarts and must not be submitted as evidence for this goal.

The TESTER will only treat this goal as **SOLVED** if the final screenshot satisfies these conditions (and, when Option B was used, the limitation is clearly stated).

## 9. Fuzzy search over note contents

Bedrock includes a dedicated **Search** tab in the left sidebar (next to **Files** and **Recent notes**) that provides a fast, fuzzy search over your vault:

- Type at least **2 characters** into the search box to activate search.
- The engine searches **note titles, paths, and full contents**, all **case-insensitively**.
- Matching is **token-based and fuzzy**:
  - Multi-word queries are split into tokens; a note must match **all** tokens to appear in results.
  - Each token allows small typos (single-character deletions) so slightly misspelled words can still match.
- Results are scored and ordered so that **exact matches** on titles/paths rank highest, followed by fuzzy content matches.

To verify fuzzy search interactively:

1. Open a vault with several markdown notes whose bodies contain a distinctive phrase (for example, `fuzzy search algorithm over note contents`).
2. Click the **Search** tab in the sidebar so the search pane is visible.
3. In the search box, type a **multi-token query** (e.g. `fuzzy search algorithm`) or a slightly misspelled variant (e.g. `fuzy searh algoritm`).
4. Confirm that:
   - The results list shows multiple matching notes whose bodies or paths contain those tokens (or close fuzzy variants).
   - Selecting a result opens that note in the editor and the query text is clearly visible in the note body.
