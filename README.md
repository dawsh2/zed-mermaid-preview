# Mermaid Preview Extension for Zed

A Zed extension that renders Mermaid diagrams as PNG images directly in your markdown files.

## Features

- Render Mermaid diagrams to PNG images using the Mermaid CLI (mmdc)
- Works with both fenced code blocks in Markdown files (````mermaid`) and standalone `.mmd` files
- Provides "Render Mermaid Diagram" and "Edit Mermaid Source" code actions
- Keeps the original Mermaid source code in HTML comments for easy editing
- PNG images render inline in Zed's markdown preview

## Requirements

- [Mermaid CLI](https://github.com/mermaid-js/mermaid-cli) (`mmdc`) must be installed
- The extension looks for `mmdc` in your PATH

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

1. Clone this repository to your Zed extensions directory:
   ```bash
   git clone https://github.com/your-username/mermaid-preview.git ~/.config/zed/extensions/mermaid-preview
   ```

2. Build the extension:
   ```bash
   cd ~/.config/zed/extensions/mermaid-preview
   cargo build --release
   ```

3. Restart Zed

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

3. Right-click and choose "Render Mermaid Diagram" from the context menu

4. The code block will be replaced with a rendered PNG image:
   ```markdown
   ![Mermaid Diagram](/path/to/your/file_diagram.png)

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

4. A PNG image will be added at the top of the file

### Editing Rendered Diagrams

To edit a previously rendered diagram:
1. Select the rendered image or comment block
2. Right-click and choose "Edit Mermaid Source"
3. The PNG will be replaced with the original Mermaid code block

## Development

This extension consists of:
- A WebAssembly extension that manages the LSP server
- A Rust LSP server that handles the Mermaid rendering

### Building from Source

```bash
# Clone the repository
git clone https://github.com/your-username/mermaid-preview.git
cd mermaid-preview

# Build the extension and LSP
cargo build --release

# The extension.wasm and mermaid-lsp binary will be created in the target/release directory
```

### Testing

The extension includes tests for the LSP server:
```bash
cargo test
```

## Architecture

The extension uses a Language Server Protocol (LSP) approach:

1. The WebAssembly extension starts a Rust LSP server
2. When you select Mermaid code and use "Render Mermaid Diagram", the LSP:
   - Extracts the Mermaid code
   - Calls `mmdc` to convert it to PNG
   - Writes the PNG file to disk
   - Replaces the code block with markdown image syntax
3. Zed's markdown preview renders the PNG image inline

## Security

- The LSP server runs in a sandboxed environment
- All file operations are restricted to the project directory
- PNG generation uses the trusted Mermaid CLI tool

## License

MIT License

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.