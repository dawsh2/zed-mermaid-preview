use anyhow::{anyhow, Result};
use log::{debug, error, info, warn};
use lsp_server::{Connection, Message, Request, RequestId, Response, ResponseError};
use lsp_types::*;
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;

mod render;

use crate::render::render_mermaid;

// Constants to avoid magic strings
const MERMAID_MEDIA_DIR: &str = ".mermaid";
const MERMAID_CACHE_DIR: &str = ".cache";
const MERMAID_FENCE_START: &str = "```mermaid";
const MERMAID_PREVIEW_COMMENT_PREFIX: &str = "<!-- mermaid-preview:";
const MERMAID_INLINE_SOURCE_COMMENT: &str = "<!-- mermaid-inline-source -->";
const MERMAID_SOURCE_SUMMARY: &str = "Show Mermaid source";

static SVG_COUNTER: AtomicUsize = AtomicUsize::new(0);
static REQUEST_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Send an error notification to the LSP client
fn send_error_notification(connection: &Connection, message: &str) {
    let notification = lsp_server::Notification {
        method: "window/showMessage".to_string(),
        params: json!({
            "type": MessageType::ERROR,
            "message": format!("Mermaid: {}", message)
        }),
    };

    if let Err(e) = connection.sender.send(Message::Notification(notification)) {
        error!("Failed to send error notification: {}", e);
    }
}

/// Send a warning notification to the LSP client
#[allow(dead_code)]
fn send_warning_notification(connection: &Connection, message: &str) {
    let notification = lsp_server::Notification {
        method: "window/showMessage".to_string(),
        params: json!({
            "type": MessageType::WARNING,
            "message": format!("Mermaid: {}", message)
        }),
    };

    if let Err(e) = connection.sender.send(Message::Notification(notification)) {
        error!("Failed to send warning notification: {}", e);
    }
}

fn main() -> Result<()> {
    // Initialize logging to a file so we can actually see what's happening
    let log_file = Path::new("/tmp/mermaid-lsp.log");

    // Try to create/open the log file, but don't fail if we can't
    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
    {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .format_timestamp_millis()
            .target(env_logger::Target::Pipe(Box::new(file)))
            .init();
    } else {
        // Fallback to stderr if file logging fails
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .format_timestamp_millis()
            .init();
    }

    info!("============================================");
    info!("Mermaid LSP starting...");
    info!("Log file: {:?}", log_file);
    // Log current working directory
    if let Ok(cwd) = std::env::current_dir() {
        info!("LSP working directory: {:?}", cwd);
    }

    // Create JSON-RPC connection
    let (connection, io_threads) = Connection::stdio();

    info!("Connection established, waiting for initialization...");

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
                "mermaid.editAllSources".to_string(),
                "mermaid.editSingleSource".to_string(),
            ],
            work_done_progress_options: WorkDoneProgressOptions {
                work_done_progress: None,
            },
        }),
        ..Default::default()
    };

    info!("Sending server capabilities...");
    let initialize_params = connection.initialize(serde_json::to_value(server_capabilities)?)?;

    // Log initialization
    let root_uri = initialize_params
        .get("rootUri")
        .and_then(|v| v.as_str())
        .unwrap_or("<none>");
    info!("Mermaid LSP initialized for workspace: {}", root_uri);

    // Store document content
    let mut documents: HashMap<String, String> = HashMap::new();

    // Main message loop
    loop {
        match connection.receiver.recv() {
            Ok(msg) => {
                match msg {
                    Message::Request(req) => {
                        debug!("Received request: {}", req.method);
                        let req_id = req.id.clone();
                        match handle_request(&connection, req, &mut documents) {
                            Ok(()) => {
                                debug!("Request handled successfully");
                            }
                            Err(e) => {
                                error!("Error handling request: {}", e);
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
                        debug!("Received notification: {}", notif.method);
                        if let Err(e) = handle_notification(notif, &connection, &mut documents) {
                            error!("Error handling notification: {}", e);
                        }
                    }
                }
            }
            Err(err) => {
                error!("LSP connection error: {}", err);
                break;
            }
        }
    }

    info!("LSP shutting down...");
    io_threads.join()?;
    Ok(())
}

fn handle_request(
    connection: &Connection,
    req: Request,
    documents: &mut HashMap<String, String>,
) -> Result<()> {
    debug!("Received request: {}", req.method);
    match req.method.as_str() {
        "textDocument/codeAction" => {
            info!("=== CODE ACTION REQUEST RECEIVED ===");
            let params: CodeActionParams = serde_json::from_value(req.params.clone())
                .map_err(|e| anyhow::anyhow!("Invalid codeAction params: {}", e))?;

            info!("URI: {}", params.text_document.uri);
            info!("Range: {:?}", params.range);

            let actions = get_code_actions(&params, documents)?;

            info!("Returning {} code actions", actions.len());
            for action in &actions {
                info!("  - {}", action.title);
            }

            let response = Response {
                id: req.id,
                result: Some(json!(actions)),
                error: None,
            };

            connection.sender.send(Message::Response(response))?;
            info!("=== CODE ACTION RESPONSE SENT ===");
        }
        "workspace/executeCommand" => {
            info!("Processing execute command request...");
            let params: ExecuteCommandParams = serde_json::from_value(req.params)
                .map_err(|e| anyhow::anyhow!("Invalid executeCommand params: {}", e))?;

            execute_command(&params, documents, connection)?;

            // Return empty response - the edit is applied via workspace/applyEdit
            let response = Response {
                id: req.id,
                result: Some(json!(null)),
                error: None,
            };

            connection.sender.send(Message::Response(response))?;
        }
        "shutdown" => {
            info!("LSP received shutdown request");
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
    debug!("Received notification: {}", notif.method);
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

    info!("=== get_code_actions called ===");
    info!("URI: {}", uri);
    info!("Cursor: line {}, char {}", cursor.line, cursor.character);

    let content = documents
        .get(&uri)
        .ok_or_else(|| anyhow::anyhow!("Document not found: {}", uri))?;

    info!("Document content length: {} bytes", content.len());

    let mut actions = Vec::new();

    // Count total mermaid blocks in the document - O(1) operation
    let total_blocks = count_mermaid_blocks(content);
    info!(
        "Found {} mermaid blocks, cursor at line {}",
        total_blocks, cursor.line
    );

    if total_blocks > 1 {
        info!("Adding Render All action for {} diagrams", total_blocks);
        let arguments = vec![json!({ "uri": uri })];
        actions.push(CodeAction {
            title: format!("Render All {} Mermaid Diagrams", total_blocks),
            kind: Some(CodeActionKind::REFACTOR_REWRITE),
            diagnostics: None,
            edit: None,
            command: Some(Command {
                title: "Render All Mermaid Diagrams".to_string(),
                command: "mermaid.renderAllLightweight".to_string(),
                arguments: Some(arguments),
            }),
            is_preferred: Some(true),
            disabled: None,
            data: None,
        });
    } else {
        info!("Not adding Render All (only {} blocks)", total_blocks);
    }

    let rendered_count = count_rendered_blocks(content);
    if rendered_count > 1 {
        debug!(
            "Adding Edit All action for {} rendered diagrams",
            rendered_count
        );
        let arguments = vec![json!({ "uri": uri })];
        actions.push(CodeAction {
            title: format!("Edit All {} Mermaid Sources", rendered_count),
            kind: Some(CodeActionKind::REFACTOR_REWRITE),
            diagnostics: None,
            edit: None,
            command: Some(Command {
                title: "Edit All Mermaid Sources".to_string(),
                command: "mermaid.editAllSources".to_string(),
                arguments: Some(arguments),
            }),
            is_preferred: Some(false),
            disabled: None,
            data: None,
        });
    }

    // Render Single - skip for now, only support bulk operations
    // (Pre-computing single renders is complex and not needed for testing)

    // Edit Mermaid action - only show when cursor is ON the HTML comment line
    // This prevents confusion when cursor is on the image line
    debug!("Checking if cursor is on a mermaid comment line...");

    let lines: Vec<&str> = content.lines().collect();
    let cursor_line = cursor.line.min((lines.len() - 1) as u32) as usize;

    if cursor_line < lines.len() {
        let line = lines[cursor_line].trim();
        let is_on_comment = line == MERMAID_INLINE_SOURCE_COMMENT;

        debug!(
            "Line {}: '{}' - is_comment: {}",
            cursor_line, line, is_on_comment
        );

        // Skip Edit Single for now - only support Edit All
        debug!("Cursor state checked, skipping Edit Single action");
    } else {
        debug!("Not checking for edit actions");
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

    // Locate the preview comment that anchors a rendered block
    let preview_line = {
        let search_start = cursor_line.saturating_sub(15);
        let backward = (search_start..=cursor_line)
            .rev()
            .find(|&i| lines[i].trim().starts_with(MERMAID_PREVIEW_COMMENT_PREFIX));

        if let Some(idx) = backward {
            Some(idx)
        } else {
            let search_end = (cursor_line + 15).min(lines.len().saturating_sub(1));
            (cursor_line..=search_end)
                .find(|&i| lines[i].trim().starts_with(MERMAID_PREVIEW_COMMENT_PREFIX))
        }
    }?;

    // Find the inline source marker and fenced code block that follows it
    let mut inline_comment_line = None;
    for idx in preview_line + 1..lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed == MERMAID_INLINE_SOURCE_COMMENT {
            inline_comment_line = Some(idx);
            break;
        }
        if trimmed.starts_with(MERMAID_PREVIEW_COMMENT_PREFIX) {
            break;
        }
    }
    let inline_comment_line = inline_comment_line?;

    let code_start_line = inline_comment_line + 1;
    if code_start_line >= lines.len() {
        return None;
    }

    if lines[code_start_line].trim_start() != MERMAID_FENCE_START {
        return None;
    }

    let mut code_end_line = None;
    for idx in code_start_line + 1..lines.len() {
        if lines[idx].trim_start().starts_with("```") {
            code_end_line = Some(idx);
            break;
        }
    }
    let code_end_line = code_end_line?;

    let code = lines[code_start_line + 1..code_end_line].join("\n");

    // Find the closing </details>
    let mut details_end_line = None;
    for idx in code_end_line + 1..lines.len() {
        if lines[idx].trim().starts_with("</details>") {
            details_end_line = Some(idx + 1);
            break;
        }
    }
    let details_end_line = details_end_line.unwrap_or(code_end_line + 1);

    Some(RenderedMermaidBlock {
        code,
        start: Position {
            line: preview_line as u32,
            character: 0,
        },
        end: Position {
            line: details_end_line as u32,
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
    info!("=== create_render_edits called for URI: {} ===", uri);
    let url = Url::parse(uri)?;
    let path = url
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("Invalid file path"))?;
    info!("File path: {:?}", path);

    // Create mermaid media directory in the document's parent directory
    // SECURITY: Validate path stays within project boundaries
    let media_dir = if let Some(parent) = path.parent() {
        let media_dir = parent.join(MERMAID_MEDIA_DIR);

        // SECURITY: Ensure the resolved path stays within the parent directory
        match media_dir.canonicalize() {
            Ok(canonicalized) => {
                if let Ok(parent_canonical) = parent.canonicalize() {
                    if !canonicalized.starts_with(&parent_canonical) {
                        return Err(anyhow!("Path traversal attempt detected: attempted to access {:?} outside of parent {:?}",
                                        canonicalized, parent_canonical));
                    }
                }
            }
            Err(_) => {
                // Path doesn't exist yet, validate the components
                if !media_dir.to_string_lossy().contains("..") {
                    // Additional validation: no parent directory references
                    for component in media_dir.components() {
                        if component == std::path::Component::ParentDir {
                            return Err(anyhow!("Path traversal attempt detected: parent directory reference in path"));
                        }
                    }
                } else {
                    return Err(anyhow!(
                        "Path traversal attempt detected: path contains '..'"
                    ));
                }
            }
        }

        media_dir
    } else {
        Path::new(MERMAID_MEDIA_DIR).to_path_buf()
    };

    // Ensure the mermaid media directory exists
    fs::create_dir_all(&media_dir)
        .map_err(|e| anyhow!("Failed to create mermaid media directory: {}", e))?;

    // Create cache directory
    let cache_dir = media_dir.join(MERMAID_CACHE_DIR);
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
        debug!("Using cached SVG for hash {:x}", code_hash);
        fs::read_to_string(&cache_path).map_err(|e| anyhow!("Failed to read cached SVG: {}", e))?
    } else {
        debug!("Rendering new SVG (cache miss) for hash {:x}", code_hash);
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

    info!("Writing SVG to: {:?}", svg_path);
    // Copy from cache to output location
    fs::write(&svg_path, svg_contents.as_bytes())
        .map_err(|e| anyhow!("Failed to write SVG: {}", e))?;
    info!("Successfully wrote SVG file");

    let svg_path_buf = Path::new(MERMAID_MEDIA_DIR).join(&svg_filename);
    let svg_relative = svg_path_buf.to_string_lossy();

    let preview_comment = format!("{}{} -->", MERMAID_PREVIEW_COMMENT_PREFIX, svg_relative);

    let mut new_text = String::new();
    new_text.push_str(&preview_comment);
    new_text.push('\n');
    new_text.push_str("<div class=\"mermaid-preview\">\n");
    new_text.push_str(&format!("![Mermaid Diagram]({})\n", svg_relative));
    new_text.push_str("</div>\n\n");
    new_text.push_str("<details class=\"mermaid-source\">\n");
    new_text.push_str(&format!(
        "  <summary>{}</summary>\n",
        MERMAID_SOURCE_SUMMARY
    ));
    new_text.push_str(&format!("  {}\n", MERMAID_INLINE_SOURCE_COMMENT));
    new_text.push_str("```mermaid\n");
    new_text.push_str(block.code.trim_end());
    new_text.push('\n');
    new_text.push_str("```\n");
    new_text.push_str("</details>\n");

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
            if start == 0 || lines[start - 1].trim() != MERMAID_INLINE_SOURCE_COMMENT {
                count += 1;
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }

    count
}

fn count_rendered_blocks(content: &str) -> usize {
    let lines: Vec<&str> = content.lines().collect();
    let mut count = 0;

    for line in lines {
        if line.trim().starts_with(MERMAID_PREVIEW_COMMENT_PREFIX) {
            count += 1;
        }
    }

    count
}

fn edit_all_sources_content(uri: &str, content: &str) -> Result<HashMap<Url, Vec<TextEdit>>> {
    let lines: Vec<&str> = content.lines().collect();
    let mut all_edits: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    let mut i = 0;

    debug!("Searching for rendered blocks to edit...");

    while i < lines.len() {
        let line = lines[i].trim();

        if line.starts_with(MERMAID_PREVIEW_COMMENT_PREFIX) {
            debug!("Found rendered block at line {}", i);

            let cursor = Position {
                line: i as u32,
                character: 0,
            };

            if let Some(block) = locate_rendered_mermaid_block(content, uri, &cursor) {
                match create_source_edits(uri, &block) {
                    Ok(mut edits) => {
                        if let Some((url, mut text_edits)) = edits.drain().next() {
                            if let Some(existing_edits) = all_edits.get_mut(&url) {
                                existing_edits.append(&mut text_edits);
                            } else {
                                all_edits.insert(url, text_edits);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to create source edits for line {}: {}", i + 1, e);
                    }
                }

                i = block.end.line as usize;
                continue;
            }
        }

        i += 1;
    }

    debug!(
        "Found {} sets of edits across all rendered blocks",
        all_edits.len()
    );
    Ok(all_edits)
}

fn render_all_diagrams_content(
    uri: &str,
    content: &str,
    connection: Option<&Connection>,
) -> Result<HashMap<Url, Vec<TextEdit>>> {
    let lines: Vec<&str> = content.lines().collect();
    let mut all_edits: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    let mut rendered_any = false; // Track if we actually rendered anything
    let mut i = 0;

    while i < lines.len() {
        if let Some((start, end)) = find_mermaid_fence(&lines, i) {
            // Skip if already rendered
            if start == 0 || lines[start - 1].trim() != MERMAID_INLINE_SOURCE_COMMENT {
                rendered_any = true; // Mark that we're rendering something
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

                match create_render_edits(uri, &block) {
                    Ok(mut edits) => {
                        if let Some((url, mut text_edits)) = edits.drain().next() {
                            if let Some(existing_edits) = all_edits.get_mut(&url) {
                                existing_edits.append(&mut text_edits);
                            } else {
                                all_edits.insert(url, text_edits);
                            }
                        }
                    }
                    Err(e) => {
                        let error_msg =
                            format!("Failed to render diagram at line {}: {}", start + 1, e);
                        error!("{}", error_msg);
                        if let Some(conn) = connection {
                            send_error_notification(conn, &error_msg);
                        }
                    }
                }
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }

    // IMPORTANT: Do NOT run cleanup here!
    // When called from CodeAction pre-computation, the edits haven't been applied yet,
    // so cleanup sees the old content and deletes all the newly created SVG files.
    // Cleanup should only run during actual command execution when we have the updated content.
    if rendered_any {
        info!("Rendered new diagrams, but skipping cleanup (not safe during pre-computation)");
    } else {
        info!("No new diagrams rendered (all already rendered), skipping cleanup");
    }

    Ok(all_edits)
}

fn apply_workspace_edit(connection: &Connection, edit: WorkspaceEdit, label: &str) -> Result<()> {
    info!("Sending workspace/applyEdit request: {}", label);

    let params = ApplyWorkspaceEditParams {
        label: Some(label.to_string()),
        edit,
    };

    let request_id = REQUEST_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let request = Request::new(
        RequestId::from(request_id.to_string()),
        "workspace/applyEdit".to_string(),
        serde_json::to_value(params)?,
    );

    connection.sender.send(Message::Request(request))?;
    info!("workspace/applyEdit request sent successfully");

    Ok(())
}

fn execute_command(
    params: &ExecuteCommandParams,
    documents: &HashMap<String, String>,
    connection: &Connection,
) -> Result<()> {
    info!("=== EXECUTE COMMAND: {} ===", params.command);

    match params.command.as_str() {
        "mermaid.renderAllLightweight" => {
            // Get URI from command arguments
            let uri = params
                .arguments
                .first()
                .and_then(|arg| arg.get("uri"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing URI argument"))?;

            let content = documents
                .get(uri)
                .ok_or_else(|| anyhow::anyhow!("Document not found: {}", uri))?;

            info!("Rendering all diagrams for {}", uri);
            let changes = render_all_diagrams_content(uri, content, Some(connection))?;

            let edit = WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            };

            // Send workspace/applyEdit to Zed
            apply_workspace_edit(connection, edit, "Render All Mermaid Diagrams")?;
            Ok(())
        }
        "mermaid.renderSingle" => {
            // Get parameters from command arguments
            let args = params
                .arguments
                .first()
                .ok_or_else(|| anyhow::anyhow!("No arguments provided"))?;

            let uri = args
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing URI argument"))?;

            let start_line =
                args.get("startLine")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| anyhow::anyhow!("Missing startLine"))? as u32;

            let end_line =
                args.get("endLine")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| anyhow::anyhow!("Missing endLine"))? as u32;

            let code = args
                .get("code")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing code"))?;

            info!("Rendering single diagram for {}", uri);

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

            let edit = WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            };

            // Send workspace/applyEdit to Zed
            apply_workspace_edit(connection, edit, "Render Mermaid Diagram")?;
            Ok(())
        }
        "mermaid.editSingleSource" => {
            let args = params
                .arguments
                .first()
                .ok_or_else(|| anyhow::anyhow!("No arguments provided"))?;

            let uri = args
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing URI argument"))?;

            let start_line =
                args.get("startLine")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| anyhow::anyhow!("Missing startLine"))? as u32;

            let end_line =
                args.get("endLine")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| anyhow::anyhow!("Missing endLine"))? as u32;

            let code = args
                .get("code")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing code"))?;

            info!("Editing single mermaid source for {}", uri);

            let block = RenderedMermaidBlock {
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

            let changes = create_source_edits(uri, &block)?;

            let edit = WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            };

            apply_workspace_edit(connection, edit, "Edit Mermaid Source")?;
            Ok(())
        }
        "mermaid.editAllSources" => {
            let uri = params
                .arguments
                .first()
                .and_then(|arg| arg.get("uri"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing URI argument"))?;

            let content = documents
                .get(uri)
                .ok_or_else(|| anyhow::anyhow!("Document not found: {}", uri))?;

            info!("Editing all mermaid sources for {}", uri);
            let changes = edit_all_sources_content(uri, content)?;

            let edit = WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            };

            apply_workspace_edit(connection, edit, "Edit All Mermaid Sources")?;
            Ok(())
        }
        _ => Err(anyhow::anyhow!("Unknown command: {}", params.command)),
    }
}
