use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
use zed_extension_api::{
    self as zed, Architecture, DownloadedFileType, LanguageServerId, Os, Result,
};

const GITHUB_REPOSITORY: &str = "dawsh2/zed-mermaid-preview";
const CACHE_ROOT: &str = "mermaid-lsp-cache";

struct MermaidPreviewExtension {
    lsp_path: Option<String>,
}

impl zed::Extension for MermaidPreviewExtension {
    fn new() -> Self {
        let mut extension = Self { lsp_path: None };

        // Pre-download LSP binary during extension initialization
        // This prevents delay on first file open
        if let Err(e) = extension.initialize_lsp_binary() {
            eprintln!("Failed to pre-download Mermaid LSP binary: {}", e);
            eprintln!("Will attempt to download on first use instead");
        }

        extension
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        if language_server_id.as_ref() != "mermaid" {
            return Err(format!("Unknown language server: {}", language_server_id));
        }

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
    /// Initialize LSP binary during extension startup to prevent first-use delay
    fn initialize_lsp_binary(&mut self) -> Result<()> {
        eprintln!("=== Initializing Mermaid LSP binary during extension load ===");

        // First, ensure Mermaid CLI is available
        if let Err(e) = self.ensure_mermaid_cli() {
            eprintln!("⚠️  Warning: Failed to ensure Mermaid CLI: {}", e);
            eprintln!("Diagram rendering may fail until @mermaid-js/mermaid-cli is installed manually");
        }

        // Create a dummy language_server_id for initialization
        let dummy_id = LanguageServerId::from("mermaid");

        // Use current directory as extension directory
        let current_dir = env::current_dir()
            .map_err(|e| format!("Failed to get current directory: {}", e))?;

        // Try to find or download the binary
        match self.get_lsp_path_impl(&dummy_id, &current_dir) {
            Ok(path) => {
                eprintln!("✅ Mermaid LSP binary initialized: {}", path);
                self.lsp_path = Some(path);
                Ok(())
            }
            Err(e) => {
                eprintln!("❌ Failed to initialize LSP binary: {}", e);
                Err(e)
            }
        }
    }

    /// Ensure Mermaid CLI is available, attempt to install if missing
    fn ensure_mermaid_cli(&self) -> Result<()> {
        eprintln!("=== Checking Mermaid CLI availability ===");

        // Check if mmdc is already available
        if let Ok(path) = Command::new("which").arg("mmdc").output() {
            if path.status.success() {
                let path_str = String::from_utf8_lossy(&path.stdout).trim();
                eprintln!("✅ Mermaid CLI found at: {}", path_str);
                return Ok(());
            }
        }

        // Check if MERMAID_CLI_PATH is set and valid
        if let Ok(custom_path) = env::var("MERMAID_CLI_PATH") {
            let path = PathBuf::from(&custom_path);
            if path.is_file() {
                eprintln!("✅ Mermaid CLI found via MERMAID_CLI_PATH: {}", path.display());
                return Ok(());
            } else {
                eprintln!("❌ MERMAID_CLI_PATH points to non-existent file: {}", path.display());
            }
        }

        eprintln!("❌ Mermaid CLI (mmdc) not found. Attempting to install...");

        // Try to install using npm
        match self.install_mermaid_cli() {
            Ok(()) => {
                eprintln!("✅ Mermaid CLI installed successfully");
                Ok(())
            }
            Err(e) => {
                eprintln!("❌ Failed to install Mermaid CLI: {}", e);
                eprintln!("Please install manually: npm install -g @mermaid-js/mermaid-cli");
                Err(e)
            }
        }
    }

    /// Install Mermaid CLI using npm
    fn install_mermaid_cli(&self) -> Result<()> {
        eprintln!("Installing @mermaid-js/mermaid-cli globally...");

        // Check if npm is available
        if let Ok(output) = Command::new("which").arg("npm").output() {
            if output.status.success() {
                let npm_path = String::from_utf8_lossy(&output.stdout).trim();
                eprintln!("Found npm at: {}", npm_path);
            } else {
                return Err("npm not found. Please install Node.js and npm first.".to_string());
            }
        } else {
            return Err("npm not found. Please install Node.js and npm first.".to_string());
        }

        // Run npm install globally
        let output = Command::new("npm")
            .args(["install", "-g", "@mermaid-js/mermaid-cli"])
            .output()
            .map_err(|e| format!("Failed to run npm install: {}", e))?;

        if output.status.success() {
            eprintln!("npm install completed successfully");
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            Err(format!(
                "npm install failed. Status: {}. Stdout: {}. Stderr: {}",
                output.status, stdout, stderr
            ))
        }
    }

    fn get_lsp_path(
        &mut self,
        worktree: &zed::Worktree,
        language_server_id: &LanguageServerId,
    ) -> Result<String> {
        // If we already have the path from initialization, use it
        if let Some(ref path) = self.lsp_path {
            return Ok(path.clone());
        }

        // Otherwise, try to get it now (fallback for first file open)
        let worktree_path = worktree.path()
            .map_err(|e| format!("Failed to get worktree path: {}", e))?;
        self.get_lsp_path_impl(language_server_id, &worktree_path)
    }

    fn get_lsp_path_impl(
        &mut self,
        language_server_id: &LanguageServerId,
        extension_dir: &Path,
    ) -> Result<String> {
        // Check for explicit local development path first
        eprintln!("=== get_lsp_path_impl called for directory: {} ===", extension_dir.display());
        match env::var("MERMAID_LSP_PATH") {
            Ok(path) => {
                eprintln!("✅ MERMAID_LSP_PATH is set: {}", path);
                let candidate = PathBuf::from(&path);
                if candidate.is_file() {
                    eprintln!("✅ File exists, using local build!");
                    return Self::finalize_path(language_server_id, candidate, &mut self.lsp_path);
                } else {
                    eprintln!("❌ File does not exist at: {}", path);
                }
            }
            Err(_) => {
                eprintln!("❌ MERMAID_LSP_PATH not set, will search local binaries or download from GitHub");
            }
        }

        // For development, check local PATH before GitHub releases
        if let Ok(output) = Command::new("which").arg("mermaid-lsp").output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout).trim();
                return Self::finalize_path(
                    language_server_id,
                    PathBuf::from(path_str),
                    &mut self.lsp_path,
                );
            }
        }

        // During development, prioritize local binaries over GitHub releases
        // This ensures we use our fixed binary with wrapper stripping
        let lsp_binary_name = Self::lsp_binary_name();

        eprintln!("Extension working directory: {:?}", extension_dir);

        // Check for bundled/local binary first (no download required)
        if let Some(path) = Self::candidate_paths(&extension_dir, lsp_binary_name)
            .into_iter()
            .find(|candidate| {
                let exists = candidate.is_file();
                if exists {
                    eprintln!("Found candidate binary: {}", candidate.display());
                }
                exists
            })
        {
            eprintln!("✅ Using bundled LSP binary: {}", path.display());
            return Self::finalize_path(language_server_id, path, &mut self.lsp_path);
        }

        eprintln!("No bundled binary found, will download from GitHub");

        // If no local binary found, try to download from GitHub
        match self.download_lsp(language_server_id, &extension_dir, lsp_binary_name) {
            Ok(downloaded) if downloaded.is_file() => {
                return Self::finalize_path(language_server_id, downloaded, &mut self.lsp_path);
            }
            Err(e) => {
                eprintln!("Failed to download LSP: {}", e);
            }
            _ => {}
        }

  
        let search_locations = Self::candidate_paths(&extension_dir, lsp_binary_name)
            .into_iter()
            .map(|candidate| candidate.display().to_string())
            .collect::<Vec<_>>();

        Err(format!(
            "LSP binary '{binary}' not found. Set MERMAID_LSP_PATH, place it on PATH, or publish a GitHub release asset. Searched in: {paths:?}",
            binary = lsp_binary_name,
            paths = search_locations
        ))
    }

    fn finalize_path(
        language_server_id: &LanguageServerId,
        path: PathBuf,
        cache: &mut Option<String>,
    ) -> Result<String> {
        let resolved = path
            .canonicalize()
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        *cache = Some(resolved.clone());

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::None,
        );

        Ok(resolved)
    }

    fn candidate_paths(extension_dir: &Path, binary_name: &str) -> Vec<PathBuf> {
        let mut candidates = vec![extension_dir.join(binary_name)];

        let target = extension_dir.join("target");
        candidates.push(target.join("release").join(binary_name));
        candidates.push(target.join("debug").join(binary_name));
        candidates.push(extension_dir.join("bin").join(binary_name));

        if Path::new("lsp/Cargo.toml").exists() {
            candidates.push(extension_dir.join("lsp/target/release").join(binary_name));
        }

        let cache_root = extension_dir.join(CACHE_ROOT);
        if let Ok(entries) = fs::read_dir(cache_root) {
            for entry in entries.flatten() {
                candidates.push(entry.path().join(binary_name));
            }
        }

        candidates
    }

    fn download_lsp(
        &mut self,
        language_server_id: &LanguageServerId,
        extension_dir: &Path,
        binary_name: &str,
    ) -> Result<PathBuf> {
        eprintln!("🔍 Checking for Mermaid LSP updates...");
        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        eprintln!("📡 Fetching latest release information...");
        let release = zed::latest_github_release(
            GITHUB_REPOSITORY,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        eprintln!("📦 Found latest release: v{}", release.version);
        let asset = Self::match_asset(&release)?;
        eprintln!("🎯 Matched platform asset: {}", asset.name);

        let version_dir = extension_dir.join(CACHE_ROOT).join(&release.version);
        let binary_path = version_dir.join(binary_name);

        // Check if we already have the latest version
        if binary_path.is_file() {
            eprintln!("🔍 Testing existing binary...");
            // Check if the binary is actually functional by testing it
            match std::process::Command::new(&binary_path)
                .arg("--version")
                .output()
            {
                Ok(output) => {
                    if output.status.success() {
                        let version = String::from_utf8_lossy(&output.stdout).trim();
                        eprintln!("✅ Using existing LSP version: {} ({})", release.version, version);
                        zed::set_language_server_installation_status(
                            language_server_id,
                            &zed::LanguageServerInstallationStatus::None,
                        );
                        return Ok(binary_path);
                    } else {
                        eprintln!("⚠️  Existing binary is broken, re-downloading version: {}", release.version);
                        // Continue to re-download
                    }
                }
                Err(e) => {
                    eprintln!("⚠️  Failed to test existing binary ({}), re-downloading: {}", e, release.version);
                    // Continue to re-download
                }
            }
        } else {
            eprintln!("📂 Binary not found locally, will download...");
        }

        eprintln!("📁 Creating cache directory: {}", version_dir.display());
        fs::create_dir_all(&version_dir)
            .map_err(|err| format!("failed to create cache directory '{version_dir:?}': {err}"))?;

        eprintln!("⬇️  Starting download of {} ({:.1}MB)...", asset.name, asset.size as f64 / 1024.0 / 1024.0);
        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::Downloading,
        );

        let start_time = std::time::Instant::now();
        zed::download_file(
            &asset.download_url,
            version_dir
                .to_str()
                .ok_or_else(|| "failed to stringify cache directory path".to_string())?,
            DownloadedFileType::Zip,
        )
        .map_err(|err| format!("failed to download mermaid-lsp asset: {err}"))?;

        let download_duration = start_time.elapsed();
        eprintln!("✅ Download completed in {:.1}s", download_duration.as_secs_f64());

        if !binary_path.is_file() {
            let error_msg = format!(
                "downloaded asset '{}' did not contain expected binary '{}'.",
                asset.name, binary_name
            );
            eprintln!("❌ {}", error_msg);
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Failed(error_msg.clone()),
            );
            return Err(format!(
                "downloaded asset '{asset_name}' did not contain expected binary '{binary_name}'",
                asset_name = asset.name
            ));
        }

        eprintln!("🔧 Making binary executable...");
        zed::make_file_executable(
            binary_path
                .to_str()
                .ok_or_else(|| "failed to stringify downloaded binary path".to_string())?,
        )?;

        eprintln!("🧹 Cleaning up old cache versions...");
        Self::purge_old_cache_versions(extension_dir, &release.version);

        eprintln!("🎉 Mermaid LSP v{} successfully installed!", release.version);
        eprintln!("📍 Binary location: {}", binary_path.display());

        Ok(binary_path)
    }

    fn purge_old_cache_versions(extension_dir: &Path, keep_version: &str) {
        let cache_root = extension_dir.join(CACHE_ROOT);
        if let Ok(entries) = fs::read_dir(&cache_root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|version| version != keep_version)
                    .unwrap_or(false)
                {
                    let _ = fs::remove_dir_all(path);
                }
            }
        }
    }

    fn match_asset(release: &zed::GithubRelease) -> Result<zed::GithubReleaseAsset> {
        let (os, arch) = zed::current_platform();
        let arch_str = match arch {
            Architecture::Aarch64 => "aarch64",
            Architecture::X86 => "x86",
            Architecture::X8664 => "x86_64",
        };

        let os_str = match os {
            Os::Mac => "apple-darwin",
            Os::Linux => "unknown-linux-gnu",
            Os::Windows => "pc-windows-msvc",
        };

        let expected_name = format!("mermaid-lsp-{arch}-{os}.zip", os = os_str, arch = arch_str);

        let assets = &release.assets;

        assets
            .iter()
            .find(|asset| asset.name == expected_name)
            .cloned()
            .ok_or_else(|| {
                let available_assets = assets
                    .iter()
                    .map(|asset| asset.name.as_str())
                    .collect::<Vec<_>>();
                format!(
                    "no GitHub release asset named '{expected}' for platform {os:?}/{arch:?}. Available assets: {available_assets:?}",
                    expected = expected_name,
                    os = os,
                    arch = arch,
                    available_assets = available_assets
                )
            })
    }

    fn lsp_binary_name() -> &'static str {
        if cfg!(target_os = "windows") {
            "mermaid-lsp.exe"
        } else {
            "mermaid-lsp"
        }
    }
}

zed_extension_api::register_extension!(MermaidPreviewExtension);
