# 🧭 Mermaid Renderer Migration Plan (Zed + LSP)

## Overview

This document describes how to migrate from the **Mermaid CLI (`mmdc`)** to a **Node-based Mermaid rendering service** that integrates cleanly with a **Rust LSP server** for use inside the **Zed editor**.

---

## 1. Motivation

### Current Situation
- `mmdc` v11+ often ignores `htmlLabels: false`, causing missing or broken labels.
- Behavior differs between systems due to inconsistent bundling of Mermaid + Chromium versions.
- Fallbacks (e.g., foreignObject → text conversion) are complex and unreliable.
- Requires external system binaries (`mmdc`, Node, Chromium) that are hard to pin.

### Observed Issue
Zed’s built-in SVG viewer does **not** support `<foreignObject>` tags, which `mmdc` often emits despite configuration.
As a result, diagrams may render as blank or malformed.

---

## 2. Proposed Solution

### Plan: Migrate from `mmdc` CLI to the Mermaid Node.js API

We will create a small Node.js **renderer service** that exposes Mermaid’s API over a simple **JSON-lines IPC protocol**, managed by the Rust LSP process.

#### Goals
- ✅ Deterministic `htmlLabels: false` behavior
- ✅ Single Mermaid version control
- ✅ Better performance (persistent renderer instance)
- ✅ No external binaries or CLI dependencies
- ✅ Cleaner, more maintainable architecture

---

## 3. Architecture

### Components
| Component | Language | Role |
|------------|-----------|------|
| **Zed extension (LSP client)** | Rust | Communicates with the LSP server via the Language Server Protocol. |
| **Custom LSP server** | Rust | Manages file changes, user requests, and spawns the renderer process. |
| **`mermaid-renderer.js`** | Node.js | Uses Mermaid + Puppeteer to render SVG, communicating with the LSP via stdin/stdout. |
| **Chromium (headless)** | Managed by Puppeteer | Provides a DOM and layout engine for accurate diagram rendering. |

### Data Flow

```
Zed Editor
   │
   ▼
Rust LSP Server
   │  JSON lines (IPC)
   ▼
Node.js Renderer
   │  (Mermaid + Puppeteer)
   ▼
Headless Chromium
   │
   ▼
SVG Output → Returned to Zed
```

---

## 4. Node.js Renderer Implementation

### Directory Layout
```
mermaid-renderer/
├─ package.json
├─ mermaid-renderer.js
└─ README.md
```

### package.json
```json
{
  "name": "mermaid-renderer",
  "version": "1.0.0",
  "description": "Persistent Mermaid renderer using Puppeteer",
  "main": "mermaid-renderer.js",
  "scripts": {
    "start": "node mermaid-renderer.js"
  },
  "dependencies": {
    "mermaid": "11.4.1",
    "puppeteer": "23.1.0"
  },
  "engines": {
    "node": ">=18"
  }
}
```

> **Note:**
> - Pin `mermaid` to the exact version verified to respect `htmlLabels: false`.
> - `puppeteer` will auto-download a compatible Chromium.
> - For smaller bundles, use `puppeteer-core` + a shared browser installation.

---

### mermaid-renderer.js
```js
#!/usr/bin/env node
// JSON-lines stdin -> JSON-lines stdout Mermaid renderer
// Requires: npm i puppeteer mermaid

const puppeteer = require('puppeteer');
const path = require('path');
const readline = require('readline');

const TIMEOUT_MS = 8000; // per-render timeout

(async () => {
  const browser = await puppeteer.launch({
    args: ['--no-sandbox', '--disable-dev-shm-usage']
  });

  const page = await browser.newPage();
  await page.setContent('<!doctype html><html><body><div id="container"></div></body></html>', { waitUntil: 'networkidle0' });

  // Inject pinned Mermaid version
  const mermaidPath = path.resolve(require.resolve('mermaid/dist/mermaid.min.js'));
  await page.addScriptTag({ path: mermaidPath });

  async function renderMermaid(src, cfg) {
    return page.evaluate(async (src, cfg) => {
      const baseCfg = { startOnLoad: false };
      const m = window.mermaid || window.mermaidAPI || {};
      if (m.initialize) m.initialize(Object.assign(baseCfg, cfg || {}));
      else if (m.mermaidAPI && m.mermaidAPI.initialize)
        m.mermaidAPI.initialize(Object.assign(baseCfg, cfg || {}));

      const id = 'mmd-' + Math.floor(Math.random() * 1e9);
      if (window.mermaid && window.mermaid.render) {
        const maybe = window.mermaid.render(id, src);
        if (maybe && typeof maybe.then === 'function') {
          const result = await maybe;
          return { svg: typeof result === 'string' ? result : result.svg };
        }
      }
      if (window.mermaid.mermaidAPI?.render) {
        return new Promise((resolve, reject) => {
          window.mermaid.mermaidAPI.render(id, src, (svg) => resolve({ svg }), {}, document.getElementById('container'));
        });
      }
      return { error: 'no compatible mermaid render function found' };
    }, src, cfg);
  }

  const rl = readline.createInterface({ input: process.stdin, output: process.stdout, terminal: false });

  rl.on('line', async (line) => {
    if (!line.trim()) return;
    let req;
    try { req = JSON.parse(line); }
    catch { return process.stdout.write(JSON.stringify({ ok: false, error: 'invalid_json' }) + '\n'); }

    const { id, diagram, config } = req;
    let done = false;
    const timer = setTimeout(() => {
      if (!done) {
        done = true;
        process.stdout.write(JSON.stringify({ id, ok: false, error: 'timeout' }) + '\n');
      }
    }, TIMEOUT_MS);

    try {
      const result = await renderMermaid(diagram, config);
      if (done) return;
      done = true;
      clearTimeout(timer);
      if (result.svg)
        process.stdout.write(JSON.stringify({ id, ok: true, svg: result.svg }) + '\n');
      else
        process.stdout.write(JSON.stringify({ id, ok: false, error: result.error || 'render_failed' }) + '\n');
    } catch (err) {
      if (!done) {
        done = true;
        clearTimeout(timer);
        process.stdout.write(JSON.stringify({ id, ok: false, error: String(err) }) + '\n');
      }
    }
  });

  rl.on('close', async () => { await browser.close(); process.exit(0); });
  process.on('SIGINT', async () => { await browser.close(); process.exit(0); });
  process.on('SIGTERM', async () => { await browser.close(); process.exit(0); });
})();
```

Make executable:
```bash
chmod +x mermaid-renderer.js
```

---

### CLI Test Example
```bash
npm install
echo '{"id":"r1","diagram":"%%{init:{flowchart:{htmlLabels:false}}}%%\n graph TD; A-->B;"}' | node mermaid-renderer.js
```
Expected: JSON output containing `{ "ok": true, "svg": "<svg ... >" }`.

---

## 5. Rust LSP Integration

### IPC Overview
The Rust LSP server keeps a single persistent Node process alive and communicates via newline-delimited JSON messages.

### Minimal Rust Pseudocode
```rust
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, process::{Command, Stdio}};
use serde_json::json;

async fn render_mermaid(diagram: &str) -> anyhow::Result<String> {
    let mut child = Command::new("node")
        .arg("path/to/mermaid-renderer.js")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    let id = "req-1";
    let req = json!({
        "id": id,
        "diagram": diagram,
        "config": { "flowchart": { "htmlLabels": false } }
    });

    stdin.write_all(format!("{}\n", req).as_bytes()).await?;
    while let Some(line) = reader.next_line().await? {
        let v: serde_json::Value = serde_json::from_str(&line)?;
        if v["id"] == id {
            if v["ok"].as_bool() == Some(true) {
                return Ok(v["svg"].as_str().unwrap_or_default().to_string());
            } else {
                anyhow::bail!(v["error"].as_str().unwrap_or("render_failed"));
            }
        }
    }
    anyhow::bail!("no response from renderer");
}
```

### Production Recommendations
- Spawn **one persistent renderer** and reuse it for all requests.
- Maintain a map of `id → oneshot::Sender` to correlate responses.
- Restart the renderer automatically on crash.
- Add per-render timeouts and diagram size limits.

---

## 6. Packaging Options

| Approach | Description | Trade-offs |
|-----------|--------------|------------|
| **Plain Node runtime** | Require Node ≥18 installed on the host. | Smallest footprint; simplest. |
| **Self-contained binary** | Use `pkg` or `nexe` to bundle Node + dependencies. | Larger binaries; needs Chromium management. |
| **Playwright backend** | Replace Puppeteer with Playwright for cross-platform installs. | Slightly heavier, more robust browser management. |

---

## 7. Summary

**Old approach:**
`mmdc` (CLI) → brittle config passing, inconsistent versions, foreignObject issues.

**New approach:**
Rust LSP → Node.js (Mermaid API + Puppeteer) → SVG
✅ Deterministic behavior
✅ Version control
✅ Better performance
✅ Cross-platform
✅ Zed-friendly

---

## 8. Next Steps
- [ ] Add `mermaid-renderer/` to your repo.
- [ ] Integrate IPC calls in `render.rs`.
- [ ] Pin tested Mermaid version.
- [ ] Add CI validation for SVG output.
- [ ] (Optional) Extend renderer to support PNG exports.

---

## 9. Example Output

**Request**
```json
{"id": "r1", "diagram": "%%{init:{flowchart:{htmlLabels:false}}}%%\ngraph TD; A-->B;"}
```

**Response**
```json
{"id": "r1", "ok": true, "svg": "<svg xmlns='http://www.w3.org/2000/svg' ...>"}
```

---

## 10. License
MIT (or same as your project)

---

**Author’s Note:**
This architecture gives you deterministic Mermaid rendering, native Rust performance, and a fully controlled rendering pipeline — without depending on `mmdc`’s flaky CLI layer.
