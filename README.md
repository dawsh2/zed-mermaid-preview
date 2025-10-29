# Mermaid Preview Extension for Zed

A Zed extension that renders Mermaid diagrams as SVG images directly in your Markdown files and dedicated `.mmd` documents.

## Features

- Render Mermaid diagrams to SVG images using the Mermaid CLI (mmdc)
- Works with both fenced code blocks in Markdown files (````mermaid`) and standalone `.mmd` files
- Provides "Render Mermaid Diagram" and "Edit Mermaid Source" code actions
- Keeps the original Mermaid source code in HTML comments for easy editing
- SVG images render inline in Zed's markdown preview with perfect scaling

## Requirements

- [Mermaid CLI](https://github.com/mermaid-js/mermaid-cli) (`mmdc`) must be installed and discoverable either via `MERMAID_CLI_PATH` or your `PATH`
- Rust with the `wasm32-wasip2` target (the scripts below add it automatically if needed)

### Installing Mermaid CLI

```bash
# Using npm
npm install -g @mermaid-js/mermaid-cli

# Using yarn
yarn global add @mermaid-js/mermaid-cli

# Using pnpm
pnpm add -g @mermaid-js/mermaid-cli
```

## Installation

The included helper scripts support a configurable extensions directory and Mermaid CLI path:

- `ZED_EXTENSIONS_DIR`: overrides the default destination (`$HOME/.config/zed/extensions`).
- `MERMAID_CLI_PATH`: absolute path to the `mmdc` binary when it is not in `PATH`.

```bash
# Clone the repository
git clone https://github.com/daws/mermaid-preview.git
cd mermaid-preview

# Build artifacts locally (optional but useful when iterating)
./build.sh

# Install into your Zed extensions directory
./install.sh

# Restart Zed to load the extension
```

## Usage

### In Markdown Files

1. Create a Mermaid code block in your markdown:
   ```markdown
   ```mermaid
   flowchart TD
       A[Start] --> B[Process Data]
       B --> C{Decision}
       C -->|Yes| D[Execute]
       C -->|No| E[Retry]
       E --> B
   ```
   ```

2. Select the code block (or place your cursor inside it)

3. Right-click and choose "Render Mermaid Diagram" from the context menu (or run the same command from the palette)

4. The code block will be replaced with a rendered SVG image:
   ```markdown
   ![Mermaid Diagram](/path/to/your/file_diagram.svg)

   <!-- mermaid-source
   ```mermaid
   flowchart TD
       A[Start] --> B[Process Data]
       B --> C{Decision}
       C -->|Yes| D[Execute]
       C -->|No| E[Retry]
       E --> B
   ```
   -->
   ```

### In Mermaid Files (`.mmd`)

1. Create or open a `.mmd` file with Mermaid code

2. Select the code you want to render

3. Right-click and choose "Render Mermaid Diagram"

4. An SVG image will be added at the top of the file alongside an embedded copy of the source

### Editing Rendered Diagrams

To edit a previously rendered diagram:
1. Select the rendered image or comment block
2. Right-click and choose "Edit Mermaid Source"
3. The SVG will be replaced with the original Mermaid code block

## Development

This extension consists of:
- A WebAssembly extension that manages the LSP server
- A Rust LSP server that handles the Mermaid rendering

### Building from Source

```bash
# Clone the repository
git clone https://github.com/daws/mermaid-preview.git
cd mermaid-preview

# Build the extension and LSP (mirrors what install.sh runs)
./build.sh
```

Artifacts are written to `extension.wasm` and `mermaid-lsp` in the repository root for packaging.

### Testing

The extension includes tests for the LSP server:
```bash
cargo test
```

## Architecture

The extension uses a Language Server Protocol (LSP) approach:

1. The WebAssembly extension starts a Rust LSP server.
2. When you select Mermaid code and use "Render Mermaid Diagram", the LSP:
   - Extracts the Mermaid code
   - Calls the Mermaid CLI (`mmdc`) to convert it to SVG (respecting `MERMAID_CLI_PATH` when set)
   - Writes the SVG file to disk
   - Replaces the code block with markdown image syntax
3. Zed's markdown preview renders the SVG image inline with perfect scaling.

### Shipping the language server

The WASM extension now downloads the native `mermaid-lsp` binary on demand from the latest GitHub release.

- When the language server starts, it checks for a cached binary that matches the current platform (macOS, Linux, Windows across `aarch64`, `x86_64`, and `x86`).
- If the binary is missing, the extension downloads `mermaid-lsp-<target>.zip` from the newest release of `daws/mermaid-preview`, unpacks it into `mermaid-lsp-cache/<version>/`, marks it executable, and launches it.
- Asset names follow the Rust triple for each target (for example `mermaid-lsp-apple-darwin-aarch64.zip`, `mermaid-lsp-unknown-linux-gnu-x86_64.zip`, `mermaid-lsp-pc-windows-msvc-x86.zip`). Ensure each archive contains the compiled binary at the root.

The Mermaid language definition now reuses Zed's built-in Markdown grammar directly, so installs no longer need to fetch or compile `tree-sitter-markdown`. That keeps dev rebuilds fast and removes the long "compiling markdown parser" step.

Developers can still run `./build.sh` locally when iterating—it produces the wasm artifact and a fresh `mermaid-lsp` binary for testing or for creating release assets. Setting `MERMAID_LSP_PATH` (or having `mermaid-lsp` on `PATH`) continues to override the download flow, which is handy for local builds.

#### Cutting a release

1. Push a tag or draft a GitHub Release in this repository.
2. The `Build mermaid-lsp binaries` workflow runs for macOS (arm64 and x86_64), Linux (x86_64), and Windows (x86_64) targets.
3. Each job invokes `scripts/package-mermaid-lsp.sh <target>` to compile `mermaid-lsp`, zip the binary as `mermaid-lsp-<target>.zip`, and upload it as a build artifact.
4. When the release is published, the workflow automatically attaches those zip files to the GitHub Release so the extension can download them on demand.

No extra steps are required for the Markdown grammar; it's bundled with Zed itself and is reused directly.

You can also trigger the workflow manually via the **Run workflow** button to produce fresh artifacts without publishing a release.

### Dumb pipe language strategy

- Markdown buffers keep using Zed's built-in Markdown language; the extension only reacts when it detects a ` ```mermaid` code fence.
- Standalone `.mmd`/`.mermaid` files are registered as a `Mermaid` language that reuses the Markdown tree-sitter grammar bundled via this extension. The grammar is fetched and compiled on first install (similar to official Markdown-based extensions) and cached afterwards.
- Because we're not injecting our own grammar, the extension behaves like a "dumb pipe": it shuttles text to the CLI, updates the document, and leaves syntax highlighting untouched.

### Future grammar options

If you decide to ship richer editor features later, you can:

- **Reuse Markdown's tree-sitter grammar** – enables syntax-aware injections, highlighting, and smarter selections inside Mermaid blocks, but requires fetching/compiling the grammar during install and tracking upstream revisions.
- **Author a minimal Mermaid grammar** – unlocks folding/indentation in `.mmd` files without pulling in the full Markdown parser, at the cost of maintaining your own tree-sitter grammar.
- **Stay with `plain_text`** – if you fork the extension and swap the grammar back to `plain_text`, the install remains lightweight but `.mmd` files won't get syntax analysis. The current default favors the Markdown grammar for a better editing experience.

## Security

- The LSP server runs in a sandboxed environment
- All file operations are restricted to the project directory
- SVG generation uses the trusted Mermaid CLI tool

## License

MIT License

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
