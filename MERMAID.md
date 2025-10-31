# Mermaid Rendering Migration Guide

This document defines the concrete steps for replacing the current
`mmdc`-based rendering pipeline (see `lsp/src/render.rs`) with the
Node.js renderer described in `debug-test.md`. All components continue to
run locally inside the Zed extension; the goal is to ship a deterministic
Mermaid stack without requiring end users to install `mmdc` manually.

---

## 1. Objectives

- Remove the dependency on the Mermaid CLI binary and its temp-file workflow.
- Pin Mermaid + Chromium versions under our control to guarantee identical SVG
  output on every machine.
- Keep rendering responsive by reusing a persistent Node.js process managed by
  the Rust LSP server.
- Preserve or improve existing sanitization and security guarantees before SVG
  data is returned to the editor.

---

## 2. Current State Snapshot

| Area | Current Implementation | Pain Points |
| --- | --- | --- |
| Rendering executable | External `mmdc` binary invoked per request | User must install `mmdc`; version drift causes `htmlLabels` regressions |
| Config handling | JSON config file + `--disableHtmlLabels` flag | Some Mermaid builds ignore the flag; foreignObject cleanup done post-render |
| Lifecycle | Spawn-once-per-render, temp files under `tempdir()` | High overhead, more IO, hard to instrument |
| Error reporting | CLI exit codes forwarded through `anyhow!` | Limited diagnostics, no structured error payloads |


---

## 3. Target Architecture Overview

```
Zed Extension (Rust) ─┬─▶ Mermaid LSP (Rust) ── JSONL IPC ──▶ Node Renderer (Mermaid + Puppeteer)
                     └─────────────────────────────── SVG + metadata ◀────────────────────────────┘
```

**Key components**
- `mermaid-renderer/` (new): Node project that hosts Mermaid + Puppeteer in a
  single persistent process and exposes a newline-delimited JSON protocol over
  stdin/stdout.
- LSP render service: A Rust manager that spawns the renderer, sends render
  requests, enforces timeouts, and restarts the process on failure.
- Sanitization: Existing Rust sanitizer logic continues to run on SVG output.

---

## 4. Implementation Phases

1. **Scaffold renderer package**
   - Create `mermaid-renderer/package.json`, `tsconfig.json` (if using TS) or
     `mermaid-renderer.js`, pinning `mermaid@11.4.x` and `puppeteer@23.x`.
   - Add NPM scripts for `npm run start` (development) and `npm run build`
     (optional bundling using `esbuild` or `tsc`).

2. **Implement renderer logic**
   - Load Mermaid into a single headless Chromium page at startup.
   - Accept messages shaped as:
     ```jsonc
     { "id": "uuid", "diagram": "...", "config": { "flowchart": { "htmlLabels": false } } }
     ```
   - Respond with `{ "id": "uuid", "ok": true, "svg": "<svg..." }` or
     `{ "id": "uuid", "ok": false, "error": "…" }`.
   - Enforce per-render timeouts, payload size limits, and input validation to
     protect the renderer from runaway diagrams.

3. **Rust IPC manager** (`lsp/src/render.rs` + new module)
   - Replace direct `Command::new("mmdc")` with a long-lived child process
     launched via `tokio::process::Command`.
   - Maintain a request registry (e.g., `HashMap<String, oneshot::Sender<_>>`) to
     correlate responses.
   - Surface renderer logs via tracing for easier debugging.
   - Provide graceful shutdown hooks triggered by LSP `Drop`/`Ctrl+C`.

4. **Configuration + environment wiring**
   - Introduce optional env vars (`MERMAID_RENDERER_PATH`, `MERMAID_TIMEOUT_MS`)
     for power users, but default to the packaged renderer.
   - Remove `MERMAID_CLI_PATH`/`MERMAID_CONFIG` code paths once Node flow is
     stable.

5. **Packaging & installation**
   - Extend `build.sh` / `install.sh` to install NPM dependencies, build the
     renderer, and place the Node entry point next to the LSP binary.
   - Cache the Puppeteer Chromium download during `npm install` so the extension
     works without additional network access after installation (everything stays
     local on the user’s machine).
   - Update `extension.toml`/`extension.wasm` metadata if necessary to ship the
     new assets.

6. **Testing & validation**
   - Add Node-side unit tests (e.g., Jest) for config handling and error cases.
   - Add Rust integration tests that spin up the renderer (using a lightweight
     headless mode) and assert consistent SVG output for representative diagrams.
   - Record golden SVG fixtures (`tests/fixtures/*.svg`) and compare against new
     renders in CI.

7. **Cleanup & deprecation**
   - Remove mmdc-specific sanitization workarounds once confirmed redundant.
   - Update documentation (README, changelog) to reflect the new architecture and
     eliminate `npm install -g @mermaid-js/mermaid-cli` instructions.

---

## 5. Renderer Design Details

### Directory Layout
```
mermaid-renderer/
├─ package.json
├─ package-lock.json (or pnpm-lock.yaml)
├─ src/
│  └─ index.ts (or index.js)
└─ scripts/
   └─ postinstall.js (optional Chromium cache logic)
```

### Dependency Pinning

- `mermaid`: pin to a known-good minor (e.g., `11.4.1`).
- `puppeteer`: pin to the matching release. Consider using `puppeteer-core`
  plus the Chromium that Puppeteer downloads at install time, stored under the
  extension directory.
- Optional: `zod` or similar schema library to validate incoming messages.

### Lifecycle Checklist

- Launch Chromium with `--no-sandbox` only when required (Zed extensions run on
  the host; document the trade-offs and consider allowing `PUPPETEER_EXECUTABLE_PATH`).
- Implement a watchdog that terminates and relaunches the renderer after N
  consecutive failures.
- Ensure `SIGINT`/`SIGTERM` handlers close the browser to avoid zombie
  processes.
- Limit concurrent renders (e.g., semaphore) if memory spikes occur with large
  diagrams.

---

## 6. IPC Contract (Draft)

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Unique per request; echoed back in responses |
| `diagram` | string | Mermaid source code (UTF-8) |
| `config` | object | Optional Mermaid config overrides |
| `options` | object | Optional render flags (theme, background, format) |

**Responses**
- Success: `{ "id": "…", "ok": true, "svg": "<svg …>" }`
- Failure: `{ "id": "…", "ok": false, "error": "message", "diagnostics": { … } }`

Document this schema in a shared module so both Rust and Node stay in sync.

---

## 7. Build & Install Workflow Updates

1. `build.sh`
   - Run `npm install --prefix mermaid-renderer` (or `pnpm install`).
   - Optionally run `npm run build` to emit a compact bundle.
   - Copy the built renderer (`mermaid-renderer/dist/index.js`, lockfile, and
     `.local-chromium/`) into the release package alongside `mermaid-lsp`.

2. `install.sh`
   - Ensure the renderer directory is copied into the Zed extensions folder.
   - Provide informative messaging if Node isn’t available at build time, since
     installation happens on the developer’s machine before distributing the
     extension.

3. Runtime
   - LSP resolves the renderer path relative to its executable (e.g.,
     `$EXT_DIR/mermaid-renderer/dist/index.js`).

---

## 8. Testing Strategy

- **Node unit tests**: Validate config initialization, rendering success,
  timeout handling, malformed JSON handling.
- **Rust integration tests**: Launch the renderer in-process using `tokio::test`
  and assert deterministic SVG output for:
  - Flowchart with `htmlLabels: false`
  - Sequence diagram
  - Diagram that previously required foreignObject cleanup
- **Regression suite**: Maintain golden SVGs and compare hashes/normalized
  output to catch upstream Mermaid changes.
- **Manual QA**: Add a Zed workspace scenario file (`manual/qa.md`) for quick
  smoke tests inside the editor.

---

## 9. Deliverables Checklist

- [ ] `mermaid-renderer/` directory committed with lockfile
- [ ] Renderer implementation with structured logging and timeout protection
- [ ] Updated LSP render pipeline using persistent Node process
- [ ] Updated build/install scripts and README instructions
- [ ] Test coverage (Node + Rust) and CI job execution
- [ ] Removal of legacy `mmdc` flow and related configuration hooks

---

## 10. Open Questions

- Do we want to support additional output formats (PNG/PDF) from day one?
- Should theme configuration be exposed via extension settings or stay hardcoded
  to match current behavior?
- How should we surface renderer failures to users inside Zed (inline error
  message vs. logging only)?

Document any answers by appending to this section as decisions are made.

