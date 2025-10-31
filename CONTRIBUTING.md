# Contributing to Zed Mermaid Preview

Thank you for your interest in contributing! This guide will help you set up your development environment and understand the workflow.

## Development Setup

### Prerequisites
- Rust (latest stable)
- Node.js and npm
- Mermaid CLI: `npm install -g @mermaid-js/mermaid-cli`
- Zed editor

### Initial Setup

1. **Clone the repository**
   ```bash
   git clone https://github.com/dawsh2/zed-mermaid-preview.git
   cd zed-mermaid-preview
   ```

2. **Run the development setup script**
   ```bash
   ./scripts/dev-setup.sh
   ```

   This configures `MERMAID_LSP_PATH` in your shell so Zed uses your local builds instead of downloading from GitHub.

3. **Restart your terminal** to pick up the environment variable

4. **Build the extension**
   ```bash
   ./scripts/build.sh
   ```

5. **Install as development extension in Zed**
   - Open Zed
   - Press `Cmd+Shift+P` (macOS) or `Ctrl+Shift+P` (Linux/Windows)
   - Type "Extensions: Install Development Extension"
   - Navigate to your cloned `zed-mermaid-preview` directory
   - Select it

6. **Restart Zed**

## Development Workflow

### Making Changes

**To the LSP server (Rust code in `lsp/`):**
```bash
cd lsp
# Make your changes to src/main.rs, src/render.rs, etc.

# Rebuild
cargo build --release

# Test
cargo test

# Restart Zed to load the new binary
```

**To the extension (Rust code in `src/`):**
```bash
# Make your changes to src/lib.rs

# Rebuild
cargo build --lib --target wasm32-wasip2 --release

# Zed will detect the change and recompile automatically
# Or restart Zed to force reload
```

**To the Mermaid config:**
```bash
# Edit lsp/src/mermaid-config.json

# Rebuild LSP to embed new config
cd lsp && cargo build --release

# Restart Zed
```

### Testing Your Changes

1. Open `example.md` in Zed
2. Try rendering various diagram types
3. Check the logs: `tail -f ~/Library/Logs/Zed/Zed.log` (macOS)
4. Run unit tests: `cd lsp && cargo test`

### Understanding the Architecture

```
┌─────────────────┐
│  Markdown File  │
└────────┬────────┘
         │ Opens in Zed
         ▼
┌─────────────────┐
│ Extension WASM  │  (src/lib.rs)
│  - Detects code │  Runs in Zed's WASM runtime
│  - Provides LSP │
└────────┬────────┘
         │ Spawns
         ▼
┌─────────────────┐
│  LSP Server     │  (lsp/src/main.rs)
│  - Code actions │  Native Rust binary
│  - Renders SVG  │
└────────┬────────┘
         │ Calls
         ▼
┌─────────────────┐
│  mmdc (Node)    │  @mermaid-js/mermaid-cli
│  - Generates    │  User must install separately
│    raw SVG      │
└────────┬────────┘
         │ Returns SVG
         ▼
┌─────────────────┐
│ SVG Sanitizer   │  (lsp/src/render.rs)
│  - Strips JS    │  Security layer
│  - Fixes labels │
└────────┬────────┘
         │
         ▼
    .mermaid/diagram.svg (saved to disk)
```

### Why MERMAID_LSP_PATH?

Without the env var, Zed uses the cached binary downloaded from GitHub releases:
```
~/Library/Application Support/Zed/extensions/work/mermaid-preview/mermaid-lsp-cache/v0.1.24/mermaid-lsp
```

With `MERMAID_LSP_PATH` set, Zed uses your local build:
```
/path/to/your/repo/target/release/mermaid-lsp
```

This means your changes take effect immediately after rebuilding!

## Common Issues

### Code actions not appearing
- Ensure LSP is running: `ps aux | grep mermaid-lsp`
- Check Zed logs for errors: `~/Library/Logs/Zed/Zed.log`
- Verify `mmdc` is installed: `mmdc --version`

### Changes not taking effect
- Make sure you rebuilt: `cd lsp && cargo build --release`
- Verify `MERMAID_LSP_PATH` is set: `echo $MERMAID_LSP_PATH`
- Restart Zed completely

### Class diagrams rendering poorly
- This was fixed in commit `[hash]` - make sure you're on latest `main`
- The fix adds diagram-specific `htmlLabels: false` to mermaid-config.json

## Submitting Changes

1. Create a new branch: `git checkout -b feature/my-feature`
2. Make your changes
3. Test thoroughly with `example.md`
4. Run tests: `cd lsp && cargo test`
5. Commit with clear messages
6. Push and create a pull request

## Release Process (Maintainers)

1. Update version in `extension.toml`
2. Update version in `lsp/Cargo.toml`
3. Build release binaries: `./scripts/package-mermaid-lsp.sh`
4. Create GitHub release with binaries
5. Users get auto-update through Zed's extension system

## Questions?

Open an issue or start a discussion on GitHub!
