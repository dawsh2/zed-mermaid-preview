use anyhow::{anyhow, Result};
use lsp_server::{Connection, Message, Request, Response, ResponseError};
use lsp_types::*;
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

mod render;

use crate::render::render_mermaid;

static SVG_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn main() -> Result<()> {
    // Log to stderr for debugging
    eprintln!("Mermaid LSP starting with debug logging enabled...");

    // Also try to log to a file
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/mermaid-lsp-debug.log")
    {
        use std::io::Write;
        let _ = writeln!(file, "[{}] Mermaid LSP starting", chrono::Utc::now().format("%H:%M:%S"));
    }

    // Create JSON-RPC connection
    let (connection, io_threads) = Connection::stdio();

    eprintln!("Connection established, waiting for initialization...");

    // Initialize LSP
    let server_capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        execute_command_provider: Some(ExecuteCommandOptions {
            commands: vec![
                "mermaid.renderAllLightweight".to_string(),
                "mermaid.renderSingle".to_string(),
            ],
            work_done_progress_options: WorkDoneProgressOptions {
                work_done_progress: None,
            },
        }),
        ..Default::default()
    };

    eprintln!("Sending server capabilities...");
    let initialize_params = connection.initialize(serde_json::to_value(server_capabilities)?)?;

    // Log initialization
    let root_uri = initialize_params
        .get("rootUri")
        .and_then(|v| v.as_str())
        .unwrap_or("<none>");
    eprintln!("Mermaid LSP initialized for workspace: {}", root_uri);

    // Store document content
    let mut documents: HashMap<String, String> = HashMap::new();

    // Main message loop
    loop {
        match connection.receiver.recv() {
            Ok(msg) => {
                match msg {
                    Message::Request(req) => {
                        eprintln!("Received request: {}", req.method);
                        let req_id = req.id.clone();
                        match handle_request(&connection, req, &mut documents) {
                            Ok(()) => {
                                eprintln!("Request handled successfully");
                            }
                            Err(e) => {
                                eprintln!("Error handling request: {}", e);
                                // Send error response
                                let error_response = Response {
                                    id: req_id,
                                    result: None,
                                    error: Some(ResponseError {
                                        code: -32603,
                                        message: format!("Internal error: {}", e),
                                        data: None,
                                    }),
                                };
                                let _ = connection.sender.send(Message::Response(error_response));
                            }
                        }
                    }
                    Message::Response(_) => {
                        // Handle responses if needed
                    }
                    Message::Notification(notif) => {
                        eprintln!("Received notification: {}", notif.method);
                        if let Err(e) = handle_notification(notif, &connection, &mut documents) {
                            eprintln!("Error handling notification: {}", e);
                        }
                    }
                }
            }
            Err(err) => {
                eprintln!("LSP connection error: {}", err);
                break;
            }
        }
    }

    eprintln!("LSP shutting down...");
    io_threads.join()?;
    Ok(())
}

fn handle_request(
    connection: &Connection,
    req: Request,
    documents: &mut HashMap<String, String>,
) -> Result<()> {
    eprintln!("Received request: {}", req.method);
    match req.method.as_str() {
        "textDocument/codeAction" => {
            eprintln!("Processing code action request...");
            let params: CodeActionParams = serde_json::from_value(req.params)
                .map_err(|e| anyhow::anyhow!("Invalid codeAction params: {}", e))?;

            let actions = get_code_actions(&params, documents)?;

            let response = Response {
                id: req.id,
                result: Some(json!(actions)),
                error: None,
            };

            connection.sender.send(Message::Response(response))?;
        }
        "workspace/executeCommand" => {
            eprintln!("Processing execute command request...");
            let params: ExecuteCommandParams = serde_json::from_value(req.params)
                .map_err(|e| anyhow::anyhow!("Invalid executeCommand params: {}", e))?;

            let result = execute_command(&params, documents)?;

            let response = Response {
                id: req.id,
                result: Some(json!(result)),
                error: None,
            };

            connection.sender.send(Message::Response(response))?;
        }
        "shutdown" => {
            eprintln!("LSP received shutdown request");
            let response = Response {
                id: req.id,
                result: Some(json!(null)),
                error: None,
            };
            connection.sender.send(Message::Response(response))?;
        }
        _ => {
            // Unknown method
            let response = Response {
                id: req.id,
                result: Some(json!(null)),
                error: Some(ResponseError {
                    code: -32601,
                    message: format!("Method not found: {}", req.method),
                    data: None,
                }),
            };
            connection.sender.send(Message::Response(response))?;
        }
    }

    Ok(())
}

fn handle_notification(
    notif: lsp_server::Notification,
    _connection: &Connection,
    documents: &mut HashMap<String, String>,
) -> Result<()> {
    eprintln!("Received notification: {}", notif.method);
    // Handle notifications directly
    match notif.method.as_str() {
        "textDocument/didOpen" => {
            let params: DidOpenTextDocumentParams = serde_json::from_value(notif.params)
                .map_err(|e| anyhow::anyhow!("Invalid didOpen params: {}", e))?;

            let uri = params.text_document.uri.to_string();
            let text = params.text_document.text;
            documents.insert(uri, text);
        }
        "textDocument/didChange" => {
            let params: DidChangeTextDocumentParams = serde_json::from_value(notif.params)
                .map_err(|e| anyhow::anyhow!("Invalid didChange params: {}", e))?;

            let uri = params.text_document.uri.to_string();
            if let Some(existing) = documents.get_mut(&uri) {
                for change in params.content_changes {
                    match change.range {
                        Some(range) => {
                            // Apply incremental change
                            let start = position_to_offset(&range.start, existing);
                            let end = position_to_offset(&range.end, existing);
                            existing.replace_range(start..end, &change.text);
                        }
                        None => {
                            // Full document replace
                            *existing = change.text;
                        }
                    }
                }
            }
        }
        "textDocument/didClose" => {
            let params: DidCloseTextDocumentParams = serde_json::from_value(notif.params)
                .map_err(|e| anyhow::anyhow!("Invalid didClose params: {}", e))?;

            let uri = params.text_document.uri.to_string();
            documents.remove(&uri);
        }
        _ => {}
    }

    Ok(())
}

fn get_code_actions(
    params: &CodeActionParams,
    documents: &HashMap<String, String>,
) -> Result<Vec<CodeAction>> {
    let uri = params.text_document.uri.to_string();
    let cursor = params.range.start;

    eprintln!("DEBUG: get_code_actions called for URI: {}, cursor line: {}", uri, cursor.line);

    // Also log to file
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/mermaid-lsp-debug.log")
    {
        use std::io::Write;
        let _ = writeln!(file, "[{}] get_code_actions - URI: {}, line: {}", chrono::Utc::now().format("%H:%M:%S"), uri, cursor.line);
    }

    let content = documents
        .get(&uri)
        .ok_or_else(|| anyhow::anyhow!("Document not found: {}", uri))?;

    let mut actions = Vec::new();

    // Count total mermaid blocks in the document - O(1) operation
    let total_blocks = count_mermaid_blocks(content);
    eprintln!("DEBUG: Found {} mermaid blocks, cursor at line {}", total_blocks, cursor.line);

    // For "Render All", we need to pre-compute due to LSP limitations
    // But we can optimize with caching and better user feedback
    if total_blocks > 1 {
        eprintln!("DEBUG: PRE-RENDERING all {} diagrams (LSP limitation)", total_blocks);

        let edit = WorkspaceEdit {
            changes: Some(render_all_diagrams_content(&uri, content)?),
            document_changes: None,
            change_annotations: None,
        };

        actions.push(CodeAction {
            title: format!("Render All {} Mermaid Diagrams", total_blocks),
            kind: Some(CodeActionKind::REFACTOR_REWRITE),
            diagnostics: None,
            edit: Some(edit),
            command: None,
            is_preferred: Some(true),
            disabled: None,
            data: None,
        });
    } else {
        eprintln!("DEBUG: Not adding Render All (only {} blocks)", total_blocks);
    }

    // For single diagrams, we can be more responsive
    if let Some(block) = locate_mermaid_source_block(content, &uri, &cursor) {
        eprintln!("DEBUG: PRE-RENDERING single diagram (LSP limitation)");

        let edit = WorkspaceEdit {
            changes: Some(create_render_edits(&uri, &block)?),
            document_changes: None,
            change_annotations: None,
        };

        actions.push(CodeAction {
            title: "Render Mermaid Diagram".to_string(),
            kind: Some(CodeActionKind::REFACTOR_REWRITE),
            diagnostics: None,
            edit: Some(edit),
            command: None,
            is_preferred: Some(total_blocks <= 1),
            disabled: None,
            data: None,
        });
    }

    // Edit Mermaid action - this is cheap, just reading from file
    eprintln!("DEBUG: Checking for rendered mermaid block...");
    if let Some(block) = locate_rendered_mermaid_block(content, &uri, &cursor) {
        eprintln!("DEBUG: Found rendered mermaid block, adding Edit action!");
        let edit = WorkspaceEdit {
            changes: Some(create_source_edits(&uri, &block)?),
            document_changes: None,
            change_annotations: None,
        };

        actions.insert(
            0,
            CodeAction {
                title: "Edit Mermaid Source".to_string(),
                kind: Some(CodeActionKind::REFACTOR_REWRITE),
                diagnostics: None,
                edit: Some(edit),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            },
        );
    } else {
        eprintln!("DEBUG: No rendered mermaid block found for editing");
    }

    Ok(actions)
}

// Removed script-related constants since we're using details wrapper

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DocumentKind {
    Markdown,
    Mermaid,
}

#[derive(Clone, Debug, Hash)]
struct MermaidSourceBlock {
    code: String,
    start: Position,
    end: Position,
    kind: DocumentKind,
}

#[derive(Clone, Debug)]
struct RenderedMermaidBlock {
    code: String,
    start: Position,
    end: Position,
    kind: DocumentKind,
}

fn is_mermaid_document(uri: &str) -> bool {
    uri.ends_with(".mmd") || uri.ends_with(".mermaid")
}

fn locate_mermaid_source_block(
    content: &str,
    uri: &str,
    cursor: &Position,
) -> Option<MermaidSourceBlock> {
    if is_mermaid_document(uri) {
        let lines: Vec<&str> = content.lines().collect();
        let last_line = lines.len().saturating_sub(1);
        let end_character = lines.get(last_line).map(|l| l.len()).unwrap_or(0);

        return Some(MermaidSourceBlock {
            code: content.to_string(),
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: last_line as u32,
                character: end_character as u32,
            },
            kind: DocumentKind::Mermaid,
        });
    }

    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let cursor_line = cursor.line.min((lines.len() - 1) as u32) as usize;
    let (start_line, end_line) = find_mermaid_fence(&lines, cursor_line)?;

    // Check if this block is already rendered (has source file comment before it)
    if start_line > 0 {
        let prev_line = lines[start_line - 1].trim();
        if prev_line.starts_with("<!-- mermaid-source-file:") {
            return None;
        }
    }

    let code = lines[start_line + 1..end_line].join("\n");

    let end_position = if end_line + 1 < lines.len() {
        Position {
            line: (end_line + 1) as u32,
            character: 0,
        }
    } else {
        Position {
            line: end_line as u32,
            character: lines[end_line].len() as u32,
        }
    };

    Some(MermaidSourceBlock {
        code,
        start: Position {
            line: start_line as u32,
            character: 0,
        },
        end: end_position,
        kind: DocumentKind::Markdown,
    })
}

fn locate_rendered_mermaid_block(
    content: &str,
    uri: &str,
    cursor: &Position,
) -> Option<RenderedMermaidBlock> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let cursor_line = cursor.line.min((lines.len() - 1) as u32) as usize;
    eprintln!("DEBUG: locate_rendered_mermaid_block - cursor at line {}, total lines: {}", cursor_line, lines.len());

    // Find comment with mermaid source file reference - expand search range
    let search_start = cursor_line.saturating_sub(10);
    let search_end = (cursor_line + 5).min(lines.len() - 1);
    eprintln!("DEBUG: Searching for mermaid comment in lines {}-{}", search_start, search_end);

    // Debug: print lines in search range
    for i in search_start..=search_end {
        if i < lines.len() {
            eprintln!("DEBUG: Line {}: '{}'", i, lines[i]);
            // Also log to file
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/mermaid-lsp-debug.log")
            {
                use std::io::Write;
                let _ = writeln!(file, "[{}] Line {}: '{}'", chrono::Utc::now().format("%H:%M:%S"), i, lines[i]);
            }
        }
    }

    let source_line = (search_start..=search_end)
        .find(|&i| {
            let line = lines[i].trim();
            let is_comment = line.starts_with("<!-- mermaid-source-file:") && line.ends_with("-->");
            if is_comment {
                eprintln!("DEBUG: Found mermaid comment at line {}: {}", i, line);
            }
            is_comment
        })?;

    // Extract the source file path
    let line = lines[source_line].trim();
    let file_start = "<!-- mermaid-source-file:".len();
    let file_end = line.len() - "-->".len();
    let source_file_path = &line[file_start..file_end];

    // Get the full path to the source file
    let source_full_path = if let Ok(url) = Url::parse(uri) {
        if let Some(path) = url.to_file_path().ok() {
            // source_file_path is relative to the document's parent
            if let Some(parent) = path.parent() {
                parent.join(source_file_path)
            } else {
                Path::new(source_file_path).to_path_buf()
            }
        } else {
            Path::new(source_file_path).to_path_buf()
        }
    } else {
        Path::new(source_file_path).to_path_buf()
    };

    eprintln!("DEBUG: Looking for source file at: {:?}", source_full_path);

    // Read the source from the file
    let code = match fs::read_to_string(&source_full_path) {
        Ok(content) => {
            eprintln!("DEBUG: Successfully read source file ({} bytes)", content.len());
            content
        }
        Err(e) => {
            eprintln!("DEBUG: Failed to read source file: {}", e);
            // Log to file
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/mermaid-lsp-debug.log")
            {
                use std::io::Write;
                let _ = writeln!(file, "[{}] Failed to read source file: {}", chrono::Utc::now().format("%H:%M:%S"), e);
            }
            return None;
        }
    };

    // Find the image reference (usually on the next non-empty line)
    let mut img_line = source_line + 1;
    while img_line < lines.len() && lines[img_line].trim().is_empty() {
        img_line += 1;
    }

    // Find the end of the block (after the image)
    let end_line = if img_line < lines.len() && lines[img_line].contains("![Mermaid Diagram](") {
        // Include the comment line and image line and one blank line after
        img_line + 2
    } else {
        source_line + 2
    };

    eprintln!("DEBUG: Found rendered block - comment line {}, img line {}, end line {}", source_line, img_line, end_line);

    // Log to file
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/mermaid-lsp-debug.log")
    {
        use std::io::Write;
        let _ = writeln!(file, "[{}] SUCCESS: Found rendered block, returning Some", chrono::Utc::now().format("%H:%M:%S"));
    }

    Some(RenderedMermaidBlock {
        code,
        start: Position {
            line: source_line as u32,
            character: 0,
        },
        end: Position {
            line: end_line.min(lines.len()) as u32,
            character: 0,
        },
        kind: if is_mermaid_document(uri) {
            DocumentKind::Mermaid
        } else {
            DocumentKind::Markdown
        },
    })
}

fn find_mermaid_fence(lines: &[&str], cursor_line: usize) -> Option<(usize, usize)> {
    let mut opening = None;

    for i in (0..=cursor_line).rev() {
        let trimmed = lines[i].trim_start();
        if trimmed.starts_with("```") {
            if trimmed.starts_with("```mermaid") {
                opening = Some(i);
                break;
            } else {
                return None;
            }
        }
    }

    let start = opening?;
    let end = (start + 1..lines.len()).find(|&i| {
        lines[i].trim_start().starts_with("```") && !lines[i].trim_start().starts_with("```mermaid")
    })?;

    Some((start, end))
}

fn create_render_edits(
    uri: &str,
    block: &MermaidSourceBlock,
) -> Result<HashMap<Url, Vec<TextEdit>>> {
    let url = Url::parse(uri)?;
    let path = url
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

    // Create .mermaid directory in the document's parent directory
    let media_dir = if let Some(parent) = path.parent() {
        parent.join(".mermaid")
    } else {
        Path::new(".mermaid").to_path_buf()
    };

    // Ensure the .mermaid directory exists
    fs::create_dir_all(&media_dir)
        .map_err(|e| anyhow!("Failed to create .mermaid directory: {}", e))?;

    // Create cache directory
    let cache_dir = media_dir.join(".cache");
    fs::create_dir_all(&cache_dir)
        .map_err(|e| anyhow!("Failed to create cache directory: {}", e))?;

    // Generate a hash of the mermaid code for caching
    let mut hasher = DefaultHasher::new();
    block.code.hash(&mut hasher);
    let code_hash = hasher.finish();
    let cache_filename = format!("mermaid_{:x}.svg", code_hash);
    let cache_path = cache_dir.join(&cache_filename);

    // Check if we have a cached version
    let svg_contents = if cache_path.exists() {
        eprintln!("DEBUG: Using cached SVG for hash {:x}", code_hash);
        fs::read_to_string(&cache_path)
            .map_err(|e| anyhow!("Failed to read cached SVG: {}", e))?
    } else {
        eprintln!("DEBUG: Rendering new SVG (cache miss) for hash {:x}", code_hash);
        let contents = render_mermaid(&block.code)?;

        // Cache the result
        fs::write(&cache_path, contents.as_bytes())
            .map_err(|e| anyhow!("Failed to write cached SVG: {}", e))?;

        contents
    };

    // Generate unique filename for output (not cache)
    let counter = SVG_COUNTER.fetch_add(1, Ordering::SeqCst);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let unique_id = format!("{}_{}", timestamp, counter);

    let svg_filename = match path.file_stem() {
        Some(stem) => {
            let stem_str = stem.to_string_lossy();
            format!("{}_diagram_{}.svg", stem_str, unique_id)
        }
        None => format!("diagram_{}.svg", unique_id),
    };

    let svg_path = media_dir.join(&svg_filename);

    // Copy from cache to output location
    fs::write(&svg_path, svg_contents.as_bytes())
        .map_err(|e| anyhow!("Failed to write SVG: {}", e))?;

    let source_file_path = {
        let base_name = path.file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let source_filename = format!("{}_{}.mmd", base_name, unique_id);
        media_dir.join(source_filename)
    };

    // Write the source to the .mmd file
    fs::write(&source_file_path, &block.code)
        .map_err(|e| anyhow!("Failed to write source file: {}", e))?;

    // Calculate relative paths from the markdown file to .mermaid directory
    let source_relative = source_file_path
        .strip_prefix(&path.parent().unwrap_or_else(|| Path::new(".")))
        .unwrap_or(&source_file_path)
        .to_string_lossy();

    let svg_path_buf = Path::new(".mermaid").join(&svg_filename);
    let svg_relative = svg_path_buf.to_string_lossy();

    let mut new_text = format!(
        "<!-- mermaid-source-file:{} -->\n\n![Mermaid Diagram]({})\n",
        source_relative, svg_relative
    );

    // Debug: Log what we're doing
    eprintln!("DEBUG: Rendering with external source file v0.2.8");

    if !new_text.ends_with('\n') {
        new_text.push('\n');
    }

    let mut changes = HashMap::new();
    changes.insert(
        Url::parse(uri)?,
        vec![TextEdit {
            range: Range {
                start: block.start.clone(),
                end: block.end.clone(),
            },
            new_text,
        }],
    );

    Ok(changes)
}

fn create_source_edits(
    uri: &str,
    block: &RenderedMermaidBlock,
) -> Result<HashMap<Url, Vec<TextEdit>>> {
    let trimmed_code = block.code.trim_end();

    let new_text = match block.kind {
        DocumentKind::Markdown => format!("```mermaid\n{}\n```\n", trimmed_code),
        DocumentKind::Mermaid => format!("{}\n", trimmed_code),
    };

    let mut changes = HashMap::new();
    changes.insert(
        Url::parse(uri)?,
        vec![TextEdit {
            range: Range {
                start: block.start.clone(),
                end: block.end.clone(),
            },
            new_text,
        }],
    );

    Ok(changes)
}

fn position_to_offset(pos: &Position, text: &str) -> usize {
    let lines: Vec<&str> = text.lines().collect();
    let mut offset = 0;

    for line in lines.iter().take(pos.line as usize) {
        offset += line.len() + 1; // +1 for newline
    }

    offset + pos.character as usize
}

fn count_mermaid_blocks(content: &str) -> usize {
    let lines: Vec<&str> = content.lines().collect();
    let mut count = 0;
    let mut i = 0;

    while i < lines.len() {
        if let Some((start, end)) = find_mermaid_fence(&lines, i) {
            // Check if it's already rendered
            if start == 0 || !lines[start - 1].starts_with("<!-- mermaid-source-file:") {
                count += 1;
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }

    count
}

fn render_all_diagrams_content(uri: &str, content: &str) -> Result<HashMap<Url, Vec<TextEdit>>> {
    let lines: Vec<&str> = content.lines().collect();
    let mut all_edits: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    let mut i = 0;

    while i < lines.len() {
        if let Some((start, end)) = find_mermaid_fence(&lines, i) {
            // Skip if already rendered
            if start == 0 || !lines[start - 1].starts_with("<!-- mermaid-source-file:") {
                let code = lines[start + 1..end].join("\n");

                let block = MermaidSourceBlock {
                    code,
                    start: Position {
                        line: start as u32,
                        character: 0,
                    },
                    end: if end + 1 < lines.len() {
                        Position {
                            line: (end + 1) as u32,
                            character: 0,
                        }
                    } else {
                        Position {
                            line: end as u32,
                            character: lines[end].len() as u32,
                        }
                    },
                    kind: if is_mermaid_document(uri) {
                        DocumentKind::Mermaid
                    } else {
                        DocumentKind::Markdown
                    },
                };

                if let Ok(mut edits) = create_render_edits(uri, &block) {
                    if let Some((url, mut text_edits)) = edits.drain().next() {
                        if let Some(existing_edits) = all_edits.get_mut(&url) {
                            existing_edits.append(&mut text_edits);
                        } else {
                            all_edits.insert(url, text_edits);
                        }
                    }
                }
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }

    Ok(all_edits)
}

fn execute_command(
    params: &ExecuteCommandParams,
    documents: &HashMap<String, String>,
) -> Result<WorkspaceEdit> {
    eprintln!("DEBUG: Executing command: {}", params.command);

    match params.command.as_str() {
        "mermaid.renderAllLightweight" => {
            // Get URI from command arguments
            let uri = params.arguments
                .first()
                .and_then(|arg| arg.get("uri"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing URI argument"))?;

            let content = documents
                .get(uri)
                .ok_or_else(|| anyhow::anyhow!("Document not found: {}", uri))?;

            eprintln!("DEBUG: Actually rendering all diagrams for {} (ON DEMAND)", uri);
            let changes = render_all_diagrams_content(uri, content)?;

            Ok(WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            })
        }
        "mermaid.renderSingle" => {
            // Get parameters from command arguments
            let args = params.arguments
                .first()
                .ok_or_else(|| anyhow::anyhow!("No arguments provided"))?;

            let uri = args
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing URI argument"))?;

            let start_line = args
                .get("startLine")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| anyhow::anyhow!("Missing startLine"))? as u32;

            let end_line = args
                .get("endLine")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| anyhow::anyhow!("Missing endLine"))? as u32;

            let code = args
                .get("code")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing code"))?;

            eprintln!("DEBUG: Rendering single diagram for {} (ON DEMAND)", uri);

            // Create the block
            let block = MermaidSourceBlock {
                code: code.to_string(),
                start: Position {
                    line: start_line,
                    character: 0,
                },
                end: Position {
                    line: end_line,
                    character: 0,
                },
                kind: DocumentKind::Markdown,
            };

            let changes = create_render_edits(uri, &block)?;

            Ok(WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            })
        }
        _ => Err(anyhow::anyhow!("Unknown command: {}", params.command)),
    }
}

fn render_all_diagrams(
    params: &TextDocumentIdentifier,
    documents: &HashMap<String, String>,
) -> Result<WorkspaceEdit> {
    let uri = params.uri.to_string();
    let content = documents
        .get(&uri)
        .ok_or_else(|| anyhow::anyhow!("Document not found: {}", uri))?;

    let changes = render_all_diagrams_content(&uri, content)?;

    Ok(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}
