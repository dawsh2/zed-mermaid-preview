use anyhow::{anyhow, Result};
use lsp_server::{Connection, Message, Request, Response, ResponseError};
use lsp_types::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashMap, fs, path::Path};
use url::Url;

mod render;

use crate::render::render_mermaid;

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

const MERMAID_STORAGE_ATTR: &str = "data-mermaid-source";
const MERMAID_STORAGE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocumentKind {
    Markdown,
    Mermaid,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum StoredDocumentKind {
    Markdown,
    Mermaid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredMermaidPayload {
    version: u32,
    kind: StoredDocumentKind,
    code: String,
}

impl From<DocumentKind> for StoredDocumentKind {
    fn from(kind: DocumentKind) -> Self {
        match kind {
            DocumentKind::Markdown => StoredDocumentKind::Markdown,
            DocumentKind::Mermaid => StoredDocumentKind::Mermaid,
        }
    }
}

impl From<StoredDocumentKind> for DocumentKind {
    fn from(kind: StoredDocumentKind) -> Self {
        match kind {
            StoredDocumentKind::Markdown => DocumentKind::Markdown,
            StoredDocumentKind::Mermaid => DocumentKind::Mermaid,
        }
    }
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

    if start_line > 0 {
        let mut i = start_line;
        while i > 0 {
            i -= 1;
            let trimmed = lines[i].trim_start();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with("<script") && trimmed.contains(MERMAID_STORAGE_ATTR) {
                return None;
            }
            break;
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
    _uri: &str,
    cursor: &Position,
) -> Option<RenderedMermaidBlock> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let cursor_line = cursor.line.min((lines.len() - 1) as u32) as usize;
    let script_start = (0..=cursor_line).rev().find(|&i| {
        let trimmed = lines[i].trim_start();
        trimmed.starts_with("<script") && trimmed.contains(MERMAID_STORAGE_ATTR)
    })?;

    let (script_end, payload_raw) = extract_script_payload(&lines, script_start)?;
    let payload = decode_stored_payload(&payload_raw)?;
    let StoredMermaidPayload { code, kind, .. } = payload;

    let mut end_line = script_end + 1;
    if let Some(img_line) =
        (script_end + 1..lines.len()).find(|&i| lines[i].contains("![Mermaid Diagram]("))
    {
        end_line = img_line + 1;
        while end_line < lines.len() && lines[end_line].trim().is_empty() {
            end_line += 1;
        }
    }

    Some(RenderedMermaidBlock {
        code,
        start: Position {
            line: script_start as u32,
            character: 0,
        },
        end: Position {
            line: end_line.min(lines.len()) as u32,
            character: 0,
        },
        kind: DocumentKind::from(kind),
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

    let svg_filename = match path.file_stem() {
        Some(stem) => {
            let stem_str = stem.to_string_lossy();
            format!("{}_diagram.svg", stem_str)
        }
        None => "diagram.svg".to_string(),
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

    let storage_json = encode_stored_payload(block)?;
    let mut new_text = format!(
        "<script type=\"application/json\" {}=\"true\">{}</script>\n\n![Mermaid Diagram]({})\n",
        MERMAID_STORAGE_ATTR, storage_json, relative_svg_path
    );

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

fn encode_stored_payload(block: &MermaidSourceBlock) -> Result<String> {
    let payload = StoredMermaidPayload {
        version: MERMAID_STORAGE_VERSION,
        kind: block.kind.into(),
        code: block.code.clone(),
    };

    let mut json = serde_json::to_string(&payload)
        .map_err(|e| anyhow!("Failed to serialize Mermaid source: {}", e))?;
    json = json.replace("</script>", "<\\/script>");
    Ok(json)
}

fn decode_stored_payload(raw: &str) -> Option<StoredMermaidPayload> {
    let payload: StoredMermaidPayload = serde_json::from_str(raw).ok()?;
    if payload.version != MERMAID_STORAGE_VERSION {
        return None;
    }
    Some(payload)
}

fn extract_script_payload(lines: &[&str], script_start: usize) -> Option<(usize, String)> {
    let first_line = *lines.get(script_start)?;
    let mut segments: Vec<&str> = Vec::new();
    let after_tag = first_line.splitn(2, '>').nth(1)?;

    if let Some(idx) = after_tag.find("</script>") {
        let content = &after_tag[..idx];
        return Some((script_start, content.trim_matches('\n').to_string()));
    }

    if !after_tag.is_empty() {
        segments.push(after_tag);
    }

    let mut script_end = script_start;

    loop {
        script_end += 1;
        if script_end >= lines.len() {
            return None;
        }

        let line = lines[script_end];
        if let Some(idx) = line.find("</script>") {
            segments.push(&line[..idx]);
            break;
        } else {
            segments.push(line);
        }
    }

    let mut payload = segments.join("\n");
    while payload.ends_with('\n') {
        payload.pop();
    }

    Some((script_end, payload))
}
