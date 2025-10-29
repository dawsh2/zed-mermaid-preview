use zed_extension_api::{self as zed, Result};
use std::path::Path;

struct MermaidPreviewExtension {
    lsp_path: Option<String>,
}

impl zed::Extension for MermaidPreviewExtension {
    fn new() -> Self {
        Self {
            lsp_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        // Only handle our Mermaid language server
        if language_server_id.to_string() != "mermaid" {
            return Err(format!("Unknown language server: {}", language_server_id));
        }

        // Get or build the LSP binary path
        let lsp_path = self.get_lsp_path(worktree, language_server_id)?;

        eprintln!("Starting Mermaid LSP at: {}", lsp_path);
        Ok(zed::Command {
            command: lsp_path,
            args: vec![],
            env: Default::default(),
        })
    }
}

impl MermaidPreviewExtension {
    fn get_lsp_path(&mut self, _worktree: &zed::Worktree, _language_server_id: &zed::LanguageServerId) -> Result<String> {
        if let Some(ref path) = self.lsp_path {
            return Ok(path.clone());
        }

        let lsp_binary_name = if cfg!(target_os = "windows") {
            "mermaid-lsp.exe"
        } else {
            "mermaid-lsp"
        };

        // Check if we're in the development directory (has lsp/Cargo.toml)
        if Path::new("lsp/Cargo.toml").exists() {
            // We're in the development directory, look in target/release
            let lsp_path = std::env::current_dir()
                .unwrap()
                .join("target/release")
                .join(&lsp_binary_name);

            if lsp_path.exists() {
                let path_string = lsp_path.to_string_lossy().to_string();
                self.lsp_path = Some(path_string.clone());
                return Ok(path_string);
            }
        }

        // For installed extensions, use the same approach as HTML extension
        // The binary should be in the extension's directory
        let lsp_path = std::env::current_dir()
            .unwrap()
            .join(&lsp_binary_name);

        if lsp_path.exists() {
            let path_string = lsp_path.to_string_lossy().to_string();
            self.lsp_path = Some(path_string.clone());
            return Ok(path_string);
        }

        Err(format!(
            "LSP binary '{}' not found. Looked in: {:?}",
            lsp_binary_name,
            lsp_path
        ).into())
    }
}

zed_extension_api::register_extension!(MermaidPreviewExtension);
