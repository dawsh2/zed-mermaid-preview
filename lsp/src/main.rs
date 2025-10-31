use anyhow::{anyhow, Result};
use log::{debug, error, info, warn};
use lsp_server::{Connection, Message, Request, RequestId, Response, ResponseError};
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

// Constants to avoid magic strings
const MERMAID_SOURCE_COMMENT_PREFIX: &str = "<!-- mermaid-source-file:";
const MERMAID_SOURCE_COMMENT_SUFFIX: &str = "-->";
const MERMAID_MEDIA_DIR: &str = ".mermaid";
const MERMAID_CACHE_DIR: &str = ".cache";
const MERMAID_FILE_EXTENSION: &str = ".mmd";
const MERMAID_FENCE_START: &str = "```mermaid";
const MERMAID_FENCE_END: &str = "```";

static SVG_COUNTER: AtomicUsize = AtomicUsize::new(0);

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

// Strip mermaid wrapper (```mermaid ... ```) from code if present
fn strip_mermaid_wrapper(code: &str) -> String {
    let trimmed = code.trim();
    let lines: Vec<&str> = trimmed.lines().collect();

    if lines.is_empty() {
        return code.to_string();
    }

    let has_start = lines[0].trim().starts_with(MERMAID_FENCE_START);
    let has_end = lines.last().map(|l| l.trim() == MERMAID_FENCE_END).unwrap_or(false);

    if has_start && has_end && lines.len() >= 2 {
        return lines[1..lines.len() - 1].join("\n");
    }

    code.to_string()
}

// Find the most recent matching source file when the referenced file doesn't exist
fn find_most_recent_source_file(missing_path: &Path, _uri: &str) -> Option<String> {
    debug!("Searching for recent source file matching pattern");

    // Extract the base filename pattern from the missing path
    if let Some(file_name) = missing_path.file_name().and_then(|n| n.to_str()) {
        // Extract base name and diagram number (e.g., "example_0" from "example_1761843815_0.mmd")
        let parts: Vec<&str> = file_name.split('_').collect();
        if parts.len() >= 3 {
            let base_name = parts[0]; // e.g., "example"
            let diagram_num = parts[parts.len() - 2]; // e.g., "0"
            let extension = parts[parts.len() - 1]; // e.g., "mmd"

            // Construct search pattern
            let pattern = format!("{}_{}_{}", base_name, "*", diagram_num);

            // Get the directory to search in
            let search_dir = missing_path.parent().unwrap_or_else(|| Path::new(MERMAID_MEDIA_DIR));

            debug!("Searching in {:?} for pattern {}", search_dir, pattern);

            // Find all matching files and get the most recent one
            if let Ok(entries) = std::fs::read_dir(search_dir) {
                let mut best_match: Option<(std::fs::DirEntry, std::time::SystemTime)> = None;

                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        // Check if it matches our pattern
                        if name.starts_with(&format!("{}_{}", base_name, diagram_num)) && name.ends_with(&format!(".{}", extension)) {
                            // Get modification time
                            if let Ok(metadata) = entry.metadata() {
                                if let Ok(modified) = metadata.modified() {
                                    match &best_match {
                                        None => best_match = Some((entry, modified)),
                                        Some((_, best_time)) => {
                                            if modified > *best_time {
                                                best_match = Some((entry, modified));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if let Some((best_entry, _)) = best_match {
                    let best_path = best_entry.path();
                    debug!("Found most recent match: {:?}", best_path);

                    // Try to read it
                    if let Ok(content) = std::fs::read_to_string(&best_path) {
                        debug!("Successfully read recent file ({} bytes)", content.len());
                        return Some(content);
                    }
                }
            }
        }
    }

    debug!("No recent source file found");
    None
}

fn main() -> Result<()> {
    // Initialize logging to a file so we can actually see what's happening
    let log_file = Path::new("/tmp/mermaid-lsp.log");

    // Try to create/open the log file, but don't fail if we can't
    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file) {

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

            let actions = get_code_actions(&params, documents, connection)?;

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
    _connection: &Connection,
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
    info!("Found {} mermaid blocks, cursor at line {}", total_blocks, cursor.line);

    // Render All - pre-compute edit for Zed compatibility
    if total_blocks > 1 {
        info!("Adding Render All action for {} diagrams (pre-computing edit)", total_blocks);

        // Pre-compute the WorkspaceEdit
        info!("Calling render_all_diagrams_content...");
        match render_all_diagrams_content(&uri, content, Some(_connection)) {
            Ok(changes) => {
                info!("Successfully rendered all diagrams, got {} file changes", changes.len());
                let edit = WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                };

                actions.push(CodeAction {
                    title: format!("Render All {} Mermaid Diagrams", total_blocks),
                    kind: Some(CodeActionKind::REFACTOR_REWRITE),
                    diagnostics: None,
                    edit: Some(edit),  // Direct edit, no command
                    command: None,
                    is_preferred: Some(true),
                    disabled: None,
                    data: None,
                });
                info!("Render All action added successfully");
            }
            Err(e) => {
                error!("Failed to pre-compute Render All edit: {}", e);
            }
        }
    } else {
        info!("Not adding Render All (only {} blocks)", total_blocks);
    }

    // Edit All - pre-compute edit for Zed compatibility
    let rendered_count = count_rendered_blocks(content);
    if rendered_count > 1 {
        debug!("Adding Edit All action for {} rendered diagrams (pre-computing edit)", rendered_count);

        // Pre-compute the WorkspaceEdit
        match edit_all_sources_content(&uri, content) {
            Ok(changes) => {
                let edit = WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                };

                actions.push(CodeAction {
                    title: format!("Edit All {} Mermaid Sources", rendered_count),
                    kind: Some(CodeActionKind::REFACTOR_REWRITE),
                    diagnostics: None,
                    edit: Some(edit),  // Direct edit, no command
                    command: None,
                    is_preferred: Some(false),
                    disabled: None,
                    data: None,
                });
            }
            Err(e) => {
                warn!("Failed to pre-compute Edit All edit: {}", e);
            }
        }
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
        let is_on_comment = line.starts_with(MERMAID_SOURCE_COMMENT_PREFIX) && line.ends_with(MERMAID_SOURCE_COMMENT_SUFFIX);

        debug!("Line {}: '{}' - is_comment: {}", cursor_line, line, is_on_comment);

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

fn locate_mermaid_source_block(
    content: &str,
    uri: &str,
    cursor: &Position,
) -> Option<MermaidSourceBlock> {
    if is_mermaid_document(uri) {
        let lines: Vec<&str> = content.lines().collect();
        let last_line = lines.len().saturating_sub(1);
        let end_character = lines.get(last_line).map(|l| l.len()).unwrap_or(0);

        // Strip mermaid wrapper if present
        let clean_code = strip_mermaid_wrapper(content);

        return Some(MermaidSourceBlock {
            code: clean_code,
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
        if prev_line.starts_with(MERMAID_SOURCE_COMMENT_PREFIX) {
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
    debug!("locate_rendered_mermaid_block ENTRY - content length: {}", content.len());

    let lines: Vec<&str> = content.lines().collect();
    debug!("locate_rendered_mermaid_block - parsed {} lines", lines.len());

    if lines.is_empty() {
        debug!("locate_rendered_mermaid_block - EARLY RETURN: lines.is_empty()");
        return None;
    }

    let cursor_line = cursor.line.min((lines.len() - 1) as u32) as usize;
    debug!("locate_rendered_mermaid_block - cursor at line {}, total lines: {}", cursor_line, lines.len());

    debug!("=== locate_rendered_mermaid_block called ===");
    debug!("Cursor line: {}, total lines: {}", cursor_line, lines.len());

    // Find comment with mermaid source file reference
    // Search BACKWARDS from cursor first (most common: cursor on image line after comment)
    // Then search forward if not found
    debug!("Searching for mermaid comment near cursor line {}", cursor_line);

    let source_line = {
        // First, search backwards from cursor (up to 10 lines)
        let search_start = cursor_line.saturating_sub(10);
        let backward_result = (search_start..=cursor_line).rev().find(|&i| {
            let line = lines[i].trim();
            let is_comment = line.starts_with(MERMAID_SOURCE_COMMENT_PREFIX) && line.ends_with(MERMAID_SOURCE_COMMENT_SUFFIX);
            if is_comment {
                debug!("Found mermaid comment (backward) at line {}: {}", i, line);
            }
            is_comment
        });

        if let Some(line) = backward_result {
            line
        } else {
            // If not found backward, search forward (up to 5 lines)
            let search_end = (cursor_line + 5).min(lines.len() - 1);
            let forward_result = (cursor_line..=search_end).find(|&i| {
                let line = lines[i].trim();
                let is_comment = line.starts_with(MERMAID_SOURCE_COMMENT_PREFIX) && line.ends_with(MERMAID_SOURCE_COMMENT_SUFFIX);
                if is_comment {
                    debug!("Found mermaid comment (forward) at line {}: {}", i, line);
                }
                is_comment
            });

            forward_result?
        }
    };

    // Extract the source file path
    let line = lines[source_line].trim();
    let file_start = MERMAID_SOURCE_COMMENT_PREFIX.len();
    let file_end = line.len() - "-->".len();
    let source_file_path = &line[file_start..file_end].trim();

    // Get the full path to the source file
    let source_full_path = if let Ok(url) = Url::parse(uri) {
        if let Some(path) = url.to_file_path().ok() {
            // source_file_path is relative to the document's parent
            if let Some(parent) = path.parent() {
                let full_path = parent.join(source_file_path);
                debug!("Document path: {:?}", path);
                debug!("Document parent: {:?}", parent);
                debug!("Relative source file: {}", source_file_path);
                debug!("Resolved full path: {:?}", full_path);

                full_path
            } else {
                debug!("No parent directory for document, using relative path");
                Path::new(source_file_path).to_path_buf()
            }
        } else {
            debug!("Could not parse URI to file path: {}", uri);
            Path::new(source_file_path).to_path_buf()
        }
    } else {
        debug!("Could not parse URI: {}", uri);
        Path::new(source_file_path).to_path_buf()
    };

    debug!("Looking for source file at: {:?}", source_full_path);
    debug!("File exists: {}", source_full_path.exists());

    // Read the source from the file
    let code = match fs::read_to_string(&source_full_path) {
        Ok(content) => {
            debug!("Successfully read source file ({} bytes)", content.len());
            content
        }
        Err(e) => {
            debug!("Failed to read source file: {}, attempting to find recent file...", e);
            debug!("Error details: {:?}", e.kind());

            // Try to find the most recent matching file
            if let Some(recent_code) = find_most_recent_source_file(&source_full_path, &uri) {
                debug!("Found recent source file, using that instead");
                recent_code
            } else {
                debug!("Could not find any recent source file");
                return None;
            }
        }
    };

    // Find the image reference (usually on the next non-empty line)
    let mut img_line = source_line + 1;
    while img_line < lines.len() && lines[img_line].trim().is_empty() {
        img_line += 1;
    }

    // Find the end of the block (after the image and any trailing blank lines)
    let end_line = if img_line < lines.len() && lines[img_line].contains("![Mermaid Diagram](") {
        // Start after the image line
        let mut end = img_line + 1;

        // Skip ONE blank line if present (common formatting)
        if end < lines.len() && lines[end].trim().is_empty() {
            end += 1;
        }

        // But stop if we hit another diagram or content
        // Don't consume the next diagram's comment or headers
        end
    } else {
        source_line + 2
    };

    debug!("Found rendered block - comment line {}, img line {}, end line {}", source_line, img_line, end_line);


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

/// Clean up old diagram files that are no longer referenced in the document
/// Keeps cache files (.cache/*.svg) but removes unreferenced output files
fn cleanup_old_diagram_files(_uri: &str, content: &str, media_dir: &Path) -> Result<()> {
    info!("=== CLEANUP: Cleaning up old diagram files in {:?}", media_dir);

    // Find all currently referenced files in the document
    let mut referenced_files = std::collections::HashSet::new();
    for line in content.lines() {
        if line.contains(MERMAID_SOURCE_COMMENT_PREFIX) {
            // Extract the .mmd file path from comment
            if let Some(start) = line.find(MERMAID_SOURCE_COMMENT_PREFIX) {
                let path_start = start + MERMAID_SOURCE_COMMENT_PREFIX.len();
                if let Some(end) = line[path_start..].find(MERMAID_SOURCE_COMMENT_SUFFIX) {
                    let file_path = line[path_start..path_start + end].trim();
                    referenced_files.insert(file_path.to_string());
                }
            }
        }
        // Also collect SVG references from markdown image links
        if line.contains("![Mermaid Diagram](") {
            if let Some(start) = line.find("](") {
                let path_start = start + 2;
                if let Some(end) = line[path_start..].find(')') {
                    let file_path = line[path_start..path_start + end].trim();
                    referenced_files.insert(file_path.to_string());
                }
            }
        }
    }

    info!("CLEANUP: Found {} referenced files in document", referenced_files.len());
    for ref_file in &referenced_files {
        info!("CLEANUP: Referenced: {}", ref_file);
    }

    // Scan the media directory for orphaned files
    if let Ok(entries) = std::fs::read_dir(media_dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            // Skip directories (like .cache)
            if path.is_dir() {
                continue;
            }

            // Only clean up .mmd and .svg files
            if let Some(ext) = path.extension() {
                if ext != "mmd" && ext != "svg" {
                    continue;
                }
            } else {
                continue;
            }

            // Check if this file is referenced
            let file_name = path.file_name().unwrap().to_string_lossy();
            let relative_path = format!("{}/{}", MERMAID_MEDIA_DIR, file_name);

            if !referenced_files.contains(file_name.as_ref()) &&
               !referenced_files.contains(&relative_path) {
                info!("CLEANUP: Removing unreferenced file: {:?}", path);
                if let Err(e) = std::fs::remove_file(&path) {
                    warn!("CLEANUP: Failed to remove old file {:?}: {}", path, e);
                }
            } else {
                info!("CLEANUP: Keeping referenced file: {:?}", path);
            }
        }
    }

    Ok(())
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
                    return Err(anyhow!("Path traversal attempt detected: path contains '..'"));
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
        fs::read_to_string(&cache_path)
            .map_err(|e| anyhow!("Failed to read cached SVG: {}", e))?
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

    let source_file_path = {
        let base_name = path.file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let source_filename = format!("{}_{}{}", base_name, unique_id, MERMAID_FILE_EXTENSION);
        media_dir.join(source_filename)
    };

    // Write the source to the .mmd file
    fs::write(&source_file_path, &block.code)
        .map_err(|e| anyhow!("Failed to write source file: {}", e))?;

    // Calculate relative paths from the markdown file to mermaid media directory
    let source_relative = source_file_path
        .strip_prefix(&path.parent().unwrap_or_else(|| Path::new(".")))
        .unwrap_or(&source_file_path)
        .to_string_lossy();

    let svg_path_buf = Path::new(MERMAID_MEDIA_DIR).join(&svg_filename);
    let svg_relative = svg_path_buf.to_string_lossy();

    let mut new_text = format!(
        "{}{}{}\n\n![Mermaid Diagram]({})\n",
        MERMAID_SOURCE_COMMENT_PREFIX, source_relative, MERMAID_SOURCE_COMMENT_SUFFIX,
        svg_relative
    );

    debug!("Rendering with external source file");

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
            if start == 0 || !lines[start - 1].starts_with(MERMAID_SOURCE_COMMENT_PREFIX) {
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
        if line.trim().starts_with(MERMAID_SOURCE_COMMENT_PREFIX) {
            count += 1;
        }
    }

    count
}

fn edit_all_sources_content(
    uri: &str,
    content: &str,
) -> Result<HashMap<Url, Vec<TextEdit>>> {
    let lines: Vec<&str> = content.lines().collect();
    let mut all_edits: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    let mut i = 0;

    debug!("Searching for rendered blocks to edit...");

    while i < lines.len() {
        let line = lines[i].trim();

        // Look for mermaid source comment lines
        if line.starts_with(MERMAID_SOURCE_COMMENT_PREFIX) && line.ends_with(MERMAID_SOURCE_COMMENT_SUFFIX) {
            debug!("Found rendered block at line {}", i);

            // Find the end of the rendered block (next blank line or mermaid fence)
            let mut end = i + 1;
            while end < lines.len() {
                let next_line = lines[end].trim();
                if next_line.is_empty() || next_line.starts_with("```mermaid") || next_line.starts_with(MERMAID_SOURCE_COMMENT_PREFIX) {
                    break;
                }
                end += 1;
            }

            // Extract the source file path from comment
            let start_pos = line.find(MERMAID_SOURCE_COMMENT_PREFIX).unwrap() + MERMAID_SOURCE_COMMENT_PREFIX.len();
            let end_pos = line.len() - MERMAID_SOURCE_COMMENT_SUFFIX.len();
            let source_file = &line[start_pos..end_pos];

            debug!("Loading source from: {}", source_file);

            // Read the source file
            if let Ok(source_url) = Url::parse(uri) {
                if let Ok(doc_path) = source_url.to_file_path() {
                    if let Some(parent) = doc_path.parent() {
                        let source_path = parent.join(source_file);
                        if let Ok(source_code) = std::fs::read_to_string(&source_path) {
                            // Create the block
                            let block = RenderedMermaidBlock {
                                code: source_code,
                                start: Position {
                                    line: i as u32,
                                    character: 0,
                                },
                                end: Position {
                                    line: end as u32,
                                    character: 0,
                                },
                                kind: DocumentKind::Markdown,
                            };

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
                        }
                    }
                }
            }

            i = end;
        } else {
            i += 1;
        }
    }

    debug!("Found {} sets of edits across all rendered blocks", all_edits.len());
    Ok(all_edits)
}

fn render_all_diagrams_content(
    uri: &str,
    content: &str,
    connection: Option<&Connection>,
) -> Result<HashMap<Url, Vec<TextEdit>>> {
    let lines: Vec<&str> = content.lines().collect();
    let mut all_edits: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    let mut rendered_any = false;  // Track if we actually rendered anything
    let mut i = 0;

    while i < lines.len() {
        if let Some((start, end)) = find_mermaid_fence(&lines, i) {
            // Skip if already rendered
            if start == 0 || !lines[start - 1].starts_with(MERMAID_SOURCE_COMMENT_PREFIX) {
                rendered_any = true;  // Mark that we're rendering something
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
                        let error_msg = format!("Failed to render diagram at line {}: {}", start + 1, e);
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

fn apply_workspace_edit(
    connection: &Connection,
    edit: WorkspaceEdit,
    label: &str,
) -> Result<()> {
    info!("Sending workspace/applyEdit request: {}", label);

    let params = ApplyWorkspaceEditParams {
        label: Some(label.to_string()),
        edit,
    };

    let request = Request::new(
        RequestId::from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos() as i32),
        "workspace/applyEdit".to_string(),
        serde_json::to_value(params)?
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
            let uri = params.arguments
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
            let uri = params.arguments
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
