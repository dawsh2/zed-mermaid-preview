use anyhow::{anyhow, Result};
use lsp_server::{Connection, Message, Request, Response, ResponseError};
use lsp_types::*;
use serde_json::json;
use std::collections::HashMap;
use url::Url;

mod render;

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
    let range = params.range;

    // Get document content
    let content = documents.get(&uri)
        .ok_or_else(|| anyhow::anyhow!("Document not found: {}", uri))?;

    // Check if the selected range contains Mermaid code
    if is_mermaid_selection(content, &range) {
        let render_action = CodeAction {
            title: "Render Mermaid Diagram".to_string(),
            kind: Some(CodeActionKind::REFACTOR_REWRITE),
            diagnostics: None,
            edit: Some(WorkspaceEdit {
                changes: Some(create_workspace_edits(&uri, content, &range)?),
                document_changes: None,
                change_annotations: None,
            }),
            command: None,
            is_preferred: Some(true),
            disabled: None,
            data: None,
        };

        // Also add an edit source action if it looks like rendered content
        if is_rendered_mermaid(content, &range) {
            let edit_action = CodeAction {
                title: "Edit Mermaid Source".to_string(),
                kind: Some(CodeActionKind::REFACTOR_REWRITE),
                diagnostics: None,
                edit: Some(WorkspaceEdit {
                    changes: Some(create_source_edits(&uri, content, &range)?),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            };

            return Ok(vec![edit_action, render_action]);
        }

        return Ok(vec![render_action]);
    }

    Ok(vec![])
}

fn is_mermaid_selection(content: &str, range: &Range) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    let start_line = range.start.line as usize;
    let end_line = range.end.line as usize;

    // Check if selection is within a Mermaid code block or contains Mermaid syntax
    for line in lines.iter().skip(start_line).take(end_line - start_line + 1) {
        // Check for fenced code block
        if line.trim().starts_with("```mermaid") {
            return true;
        }

        // Check for common Mermaid diagram types
        let mermaid_patterns = [
            "graph TD", "graph LR", "graph TB", "graph BT", "graph RL",
            "flowchart TD", "flowchart LR", "flowchart TB", "flowchart BT", "flowchart RL",
            "sequenceDiagram", "classDiagram", "stateDiagram", "stateDiagram-v2",
            "gantt", "pie", "journey", "gitgraph", "C4Context", "mindmap",
            "timeline", "sankey", "block", "architecture", "erDiagram"
        ];

        for pattern in &mermaid_patterns {
            if line.trim().starts_with(pattern) {
                return true;
            }
        }
    }

    false
}

fn is_rendered_mermaid(content: &str, range: &Range) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    let start_line = range.start.line as usize;
    let end_line = range.end.line as usize;

    // Check if selection contains our rendered content
    for line in lines.iter().skip(start_line).take(end_line - start_line + 1) {
        if line.contains("<!-- mermaid-source") ||
           line.contains("![Mermaid Diagram](") ||
           line.contains("<svg") {
            return true;
        }
    }

    false
}

fn create_workspace_edits(
    uri: &str,
    content: &str,
    range: &Range,
) -> Result<HashMap<Url, Vec<TextEdit>>> {
    let lines: Vec<&str> = content.lines().collect();
    let start_line = range.start.line as usize;
    let end_line = range.end.line as usize;

    // Check if this is a fenced code block in Markdown or a whole .mmd file
    let (mermaid_code, start_pos, end_pos) = if lines.iter().any(|line| line.trim_start().starts_with("```mermaid")) {
        // This is a fenced code block - find the boundaries
        let block_start = (0..=start_line)
            .rev()
            .find(|&i| lines[i].trim_start().starts_with("```mermaid"))
            .ok_or_else(|| anyhow::anyhow!("No ```mermaid found"))?;

        let block_end = (end_line..lines.len())
            .find(|&i| lines[i].trim_start() == "```")
            .ok_or_else(|| anyhow::anyhow!("No closing ``` found"))?;

        // Extract Mermaid code from inside the code block
        let code = lines[block_start + 1..block_end]
            .iter()
            .copied()
            .collect::<Vec<&str>>()
            .join("\n");

        let start_pos = Position {
            line: block_start as u32,
            character: 0,
        };
        let end_pos = Position {
            line: block_end as u32,
            character: lines[block_end].len() as u32,
        };

        (code, start_pos, end_pos)
    } else {
        // This is a whole .mmd file - use the entire content
        let code = content.to_string();

        let start_pos = Position {
            line: 0,
            character: 0,
        };
        let end_pos = Position {
            line: (lines.len() - 1) as u32,
            character: lines[lines.len() - 1].len() as u32,
        };

        (code, start_pos, end_pos)
    };

    // Convert directly to PNG for inline viewing
    use std::fs;
    use std::process::Command;

    let url = Url::parse(uri)?;
    let path = url.to_file_path()
        .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

    // Generate PNG filename based on the original file
    let png_filename = match path.file_stem() {
        Some(stem) => {
            let stem_str = stem.to_string_lossy();
            format!("{}_diagram.png", stem_str)
        }
        None => "diagram.png".to_string(),
    };

    let png_path = path.parent()
        .unwrap_or_else(|| &path)
        .join(&png_filename);
    let png_path_str = png_path.to_string_lossy();

    // Convert directly to PNG using mmdc
    let mut png_output = Command::new("mmdc")
        .arg("-i") // input from stdin
        .arg("-")
        .arg("-o") // output to stdout
        .arg("-") // PNG output
        .arg("-e") // specify PNG format
        .arg("png")
        .arg("-b") // transparent background
        .arg("transparent")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("Failed to start mmdc: {}", e))?;

    // Write mermaid code to stdin
    {
        use std::io::Write;
        let stdin = png_output.stdin.as_mut().unwrap();
        stdin.write_all(mermaid_code.as_bytes())
            .map_err(|e| anyhow!("Failed to write to mmdc stdin: {}", e))?;
    }

    // Get the output
    let output = png_output.wait_with_output()
        .map_err(|e| anyhow!("Failed to get mmdc output: {}", e))?;

    let png_data = if output.status.success() {
        output.stdout
    } else {
        let stderr = output.stderr;
        return Err(anyhow!("PNG conversion failed: {}", String::from_utf8_lossy(&stderr)));
    };

    // Write PNG to file
    fs::write(&png_path, &png_data)
        .map_err(|e| anyhow!("Failed to write PNG file: {}", e))?;

    // Create absolute path for img tag to ensure Zed can find it
    let absolute_png_path = png_path_str.to_string();

    // Create output with Markdown image reference
    let output = if lines.iter().any(|line| line.trim_start().starts_with("```mermaid")) {
        // Markdown file - replace code block with image
        format!(
            "![Mermaid Diagram]({})\n\n<!-- mermaid-source\n``mermaid\n{}\n``\n-->",
            absolute_png_path, mermaid_code
        )
    } else {
        // .mmd file - add image at top and keep source
        format!(
            "![Mermaid Diagram]({})\n\n<!-- mermaid-source\n{}\n-->",
            absolute_png_path, mermaid_code
        )
    };

    let mut changes = HashMap::new();
    changes.insert(
        Url::parse(uri)?,
        vec![TextEdit {
            range: Range {
                start: start_pos,
                end: end_pos,
            },
            new_text: output,
        }],
    );

    Ok(changes)
}

fn create_source_edits(
    uri: &str,
    content: &str,
    range: &Range,
) -> Result<HashMap<Url, Vec<TextEdit>>> {
    let lines: Vec<&str> = content.lines().collect();
    let start_line = range.start.line as usize;
    let _end_line = range.end.line as usize;

    // Find the img reference or mermaid-source comment
    let img_start = (0..=start_line)
        .rev()
        .find(|&i| lines[i].contains("![Mermaid Diagram]("))
        .ok_or_else(|| anyhow::anyhow!("No Mermaid diagram reference found"))?;

    // Find the mermaid-source comment
    let source_start = (img_start..lines.len())
        .find(|&i| lines[i].contains("<!-- mermaid-source"))
        .ok_or_else(|| anyhow::anyhow!("No mermaid-source comment found"))?;

    let source_end = (source_start..lines.len())
        .find(|&i| lines[i].contains("-->"))
        .ok_or_else(|| anyhow::anyhow!("No end comment found"))?;

    // Extract source code from inside the comment
    let mut mermaid_code = String::new();
    let mut in_code_block = false;

    for line in lines.iter().skip(source_start + 1).take(source_end - source_start - 1) {
        if line.trim() == "```mermaid" {
            in_code_block = true;
            continue;
        }
        if line.trim() == "```" && in_code_block {
            break;
        }
        if in_code_block {
            if !mermaid_code.is_empty() {
                mermaid_code.push('\n');
            }
            mermaid_code.push_str(line);
        }
    }

    // If no code block was found, extract all lines
    if mermaid_code.is_empty() {
        for line in lines.iter().skip(source_start + 1).take(source_end - source_start - 1) {
            if !mermaid_code.is_empty() {
                mermaid_code.push('\n');
            }
            mermaid_code.push_str(line);
        }
    }

    let start_pos = Position {
        line: img_start as u32,
        character: 0,
    };

    let end_pos = Position {
        line: (source_end + 1) as u32,
        character: 0,
    };

    // Create code block
    let output = format!("```mermaid\n{}\n```", mermaid_code);

    let mut changes = HashMap::new();
    changes.insert(
        Url::parse(uri)?,
        vec![TextEdit {
            range: Range {
                start: start_pos,
                end: end_pos,
            },
            new_text: output,
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