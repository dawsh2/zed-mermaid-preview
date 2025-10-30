use anyhow::{anyhow, Result};
use base64::engine::{general_purpose::STANDARD, Engine as _};
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

mod render;

use crate::render::render_mermaid;

static SVG_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn main() -> Result<()> {
    // Log to stderr for debugging
    eprintln!("Mermaid LSP starting...");

    // Create JSON-RPC connection
    let (connection, io_threads) = Connection::stdio();

    eprintln!("Connection established, waiting for initialization...");

    // Initialize LSP
    let server_capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
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

    let content = documents
        .get(&uri)
        .ok_or_else(|| anyhow::anyhow!("Document not found: {}", uri))?;

    let mut actions = Vec::new();

    if let Some(block) = locate_mermaid_source_block(content, &uri, &cursor) {
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
            is_preferred: Some(true),
            disabled: None,
            data: None,
        });
    }

    if let Some(block) = locate_rendered_mermaid_block(content, &uri, &cursor) {
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
    }

    Ok(actions)
}

// Removed script-related constants since we're using details wrapper

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocumentKind {
    Markdown,
    Mermaid,
}

#[derive(Clone, Debug)]
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

    // Find comment with mermaid source file reference
    let source_line = (0..=cursor_line)
        .rev()
        .find(|&i| {
            let line = lines[i].trim();
            line.starts_with("<!-- mermaid-source-file:") && line.ends_with("-->")
        })?;

    // Extract the source file path
    let line = lines[source_line].trim();
    let file_start = "<!-- mermaid-source-file:".len();
    let file_end = line.len() - "-->".len();
    let source_file_path = &line[file_start..file_end];

    // Get the full path to the source file
    let source_full_path = if let Ok(url) = Url::parse(uri) {
        if let Some(path) = url.to_file_path().ok() {
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

    // Read the source from the file
    let code = fs::read_to_string(&source_full_path)
        .ok()?;

    // Find the image reference (usually on the next non-empty line)
    let mut img_line = source_line + 1;
    while img_line < lines.len() && lines[img_line].trim().is_empty() {
        img_line += 1;
    }

    // Find the end of the block (after the image)
    let mut end_line = img_line + 1;
    if img_line < lines.len() && lines[img_line].contains("![Mermaid Diagram](") {
        while end_line < lines.len() && (lines[end_line].trim().is_empty() || end_line == img_line) {
            end_line += 1;
        }
    } else {
        end_line = source_line + 1;
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

    let svg_path = if let Some(parent) = path.parent() {
        parent.join(&svg_filename)
    } else {
        Path::new(&svg_filename).to_path_buf()
    };

    let svg_contents = render_mermaid(&block.code)?;

    if let Some(parent) = svg_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| anyhow!("Failed to create output directory: {}", e))?;
        }
    }

    fs::write(&svg_path, svg_contents.as_bytes())
        .map_err(|e| anyhow!("Failed to write SVG: {}", e))?;

    let relative_svg_path = match path.parent() {
        Some(parent) => svg_path
            .strip_prefix(parent)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| svg_filename.clone()),
        None => svg_filename.clone(),
    };

    
    // Store the source in a separate file and leave only the image
    let source_file_path = if let Some(parent) = path.parent() {
        let base_name = path.file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let source_filename = format!("{}_{}.mmd", base_name, unique_id);
        parent.join(source_filename)
    } else {
        Path::new(&format!("{}.mmd", unique_id)).to_path_buf()
    };

    // Write the source to the .mmd file
    fs::write(&source_file_path, &block.code)
        .map_err(|e| anyhow!("Failed to write source file: {}", e))?;

    // Add a reference to the source file
    let source_relative = source_file_path
        .strip_prefix(&path.parent().unwrap_or_else(|| Path::new(".")))
        .unwrap_or(&source_file_path)
        .to_string_lossy();

    let mut new_text = format!(
        "<!-- mermaid-source-file:{} -->\n\n![Mermaid Diagram]({})\n",
        source_relative, relative_svg_path
    );

    // Debug: Log what we're doing
    eprintln!("DEBUG: Rendering with external source file v0.2.6");

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

// Removed script-related encode/decode functions - no longer needed with details wrapper
