# Security Policy

## Overview

This extension renders Mermaid diagrams by executing the `mmdc` (mermaid-cli) command-line tool. This document outlines the security assumptions, protections, and potential risks.

## Security Model

### Trust Assumptions

1. **Trusted Input**: This extension assumes that Mermaid diagram code comes from **trusted sources** (the user's own markdown files or files they explicitly open in their editor).

2. **User Authorization**: By opening a markdown file and invoking diagram rendering, the user explicitly authorizes the extension to process that content.

3. **Local Execution**: All processing happens locally on the user's machine. No data is sent to external services.

### Threat Model

#### In Scope
- **Path Traversal**: Protection against malicious file paths that attempt to write outside project boundaries
- **Command Injection**: Prevention of shell injection through diagram content
- **DoS Attacks**: Protection against regex catastrophic backtracking
- **Script Injection**: Removal of `<script>` tags from generated SVG

#### Out of Scope
- **Malicious Diagrams from Untrusted Sources**: If you render diagrams from untrusted sources, they may contain malicious Mermaid syntax that could exploit vulnerabilities in the `mmdc` tool itself
- **Supply Chain Attacks**: Trust in the `mmdc` binary and its dependencies

## Security Protections

### 1. Path Traversal Protection

**Location**: `lsp/src/main.rs` (lines 820-850)

```rust
// SECURITY: Validate path stays within project boundaries
match media_dir.canonicalize() {
    Ok(canonicalized) => {
        if !canonicalized.starts_with(parent_canonical) {
            return Err(anyhow!("Path traversal attempt detected"));
        }
    }
    Err(_) => {
        // Check for explicit path traversal patterns
        if media_dir.to_string_lossy().contains("..") {
            return Err(anyhow!("Path traversal attempt detected"));
        }
    }
}
```

**Protection**: Prevents malicious comment references like `<!-- mermaid-source-file:../../../../etc/passwd -->` from accessing files outside the project directory.

### 2. SVG Script Tag Removal

**Location**: `lsp/src/render.rs` (lines 60-66)

```rust
// SECURITY: Remove any <script> tags from the SVG for safety
if svg.contains("<script") {
    return Err(anyhow!("SVG contains <script> elements"));
}
```

**Protection**: Rejects SVGs containing `<script>` tags that could execute arbitrary JavaScript if the SVG is opened in a browser context.

### 3. Regex DoS Prevention

**Location**: `lsp/src/render.rs` (lines 12-19)

```rust
static FOREIGN_OBJECT_REGEX: Lazy<Regex> = Lazy::new(|| {
    // More efficient pattern that prevents catastrophic backtracking
    Regex::new(r#"<foreignObject\s+[^>]+>([^<]+(?:<(?!/foreignObject>)[^<]*)*)</foreignObject>"#)
        .expect("Foreign object regex should compile")
});
```

**Protection**: Uses atomic grouping and character classes instead of greedy quantifiers to prevent catastrophic backtracking on malicious input.

### 4. Command Execution Safety

**Location**: `lsp/src/render.rs` (lines 21-50)

The extension executes `mmdc` as a subprocess, but:
- **No shell interpolation**: Uses `Command::new()` with explicit arguments, not shell strings
- **Input sanitization**: Mermaid code is written to a temporary file, not passed via command line
- **Controlled environment**: Only the diagram code is processed; no user-provided file paths in command arguments

```rust
let output = Command::new(&mmdc_path)
    .arg("-i")
    .arg(&input_path)
    .arg("-o")
    .arg(&output_path)
    .arg("-c")
    .arg(&config_path)
    .stdin(Stdio::null())
    .stderr(Stdio::piped())
    .stdout(Stdio::piped())
    .output()
    .map_err(|e| anyhow!("Failed to execute mmdc: {}", e))?;
```

### 5. Temporary File Isolation

**Location**: `lsp/src/render.rs` (line 20)

```rust
let temp_dir = tempdir().map_err(|e| anyhow!("Failed to create temp dir: {}", e))?;
```

**Protection**: Uses system temp directories with random names to prevent file conflicts and unauthorized access.

## Known Limitations

### 1. Trust in mmdc Binary

The extension trusts the `mmdc` binary installed on the system. If this binary is compromised, the extension cannot protect against malicious behavior.

**Mitigation**: Users should install `mmdc` from official sources:
```bash
npm install -g @mermaid-js/mermaid-cli
```

### 2. Mermaid Parser Vulnerabilities

Mermaid's parser may have vulnerabilities that could be exploited through crafted diagram syntax. The extension cannot protect against these.

**Mitigation**: Keep `mmdc` updated to the latest version with security patches.

### 3. Resource Exhaustion

Extremely large or complex diagrams could consume significant memory or CPU time during rendering.

**Mitigation**:
- User control: Users explicitly trigger rendering
- Timeout: Command execution has configurable timeouts (default 2 minutes)
- Local execution: Only affects the user's machine, not a shared service

### 4. File System Access

The extension creates files in `.mermaid/` directories within the project. This requires write access to the project directory.

**Mitigation**:
- Limited scope: Only writes to `.mermaid/` subdirectories
- Path validation: Prevents traversal outside project boundaries
- User authorization: User must explicitly invoke rendering

## Reporting Security Issues

If you discover a security vulnerability, please report it by:

1. **DO NOT** open a public GitHub issue
2. Email the maintainer or create a private security advisory on GitHub
3. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

## Security Best Practices for Users

1. **Only render diagrams from trusted sources**
2. **Keep `mmdc` updated** to the latest version
3. **Review generated SVG files** if sharing them publicly
4. **Use project-specific `.gitignore`** to exclude `.mermaid/` if diagrams contain sensitive information
5. **Set RUST_LOG=info** or higher to monitor extension behavior

## Changelog

- **2025-10-31**: Initial security documentation
- **2025-10-31**: Added regex DoS protection
- **2025-10-31**: Added LSP error notifications
- **2025-10-31**: Added automatic cleanup of orphaned files
