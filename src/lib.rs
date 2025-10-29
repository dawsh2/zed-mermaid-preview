use std::{
    env, fs,
    path::{Path, PathBuf},
};
use zed_extension_api::{
    self as zed, Architecture, DownloadedFileType, LanguageServerId, Os, Result,
};

const GITHUB_REPOSITORY: &str = "daws/mermaid-preview";
const CACHE_ROOT: &str = "mermaid-lsp-cache";

struct MermaidPreviewExtension {
    lsp_path: Option<String>,
}

impl zed::Extension for MermaidPreviewExtension {
    fn new() -> Self {
        Self { lsp_path: None }
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
    fn get_lsp_path(
        &mut self,
        worktree: &zed::Worktree,
        language_server_id: &LanguageServerId,
    ) -> Result<String> {
        if let Some(path) = self
            .lsp_path
            .as_ref()
            .filter(|candidate| Path::new(candidate).is_file())
            .cloned()
        {
            return Ok(path);
        }

        if let Ok(path) = env::var("MERMAID_LSP_PATH") {
            let candidate = PathBuf::from(path);
            if candidate.is_file() {
                return Self::finalize_path(language_server_id, candidate, &mut self.lsp_path);
            }
        }

        if let Some(path) = worktree.which("mermaid-lsp") {
            return Self::finalize_path(
                language_server_id,
                PathBuf::from(path),
                &mut self.lsp_path,
            );
        }

        let lsp_binary_name = Self::lsp_binary_name();
        let extension_dir = env::current_dir()
            .map_err(|error| format!("unable to determine extension directory: {error}"))?;

        if let Some(path) = Self::candidate_paths(&extension_dir, lsp_binary_name)
            .into_iter()
            .find(|candidate| candidate.is_file())
        {
            return Self::finalize_path(language_server_id, path, &mut self.lsp_path);
        }

        let downloaded = self.download_lsp(language_server_id, &extension_dir, lsp_binary_name)?;
        if downloaded.is_file() {
            return Self::finalize_path(language_server_id, downloaded, &mut self.lsp_path);
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
        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let release = zed::latest_github_release(
            GITHUB_REPOSITORY,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let asset = Self::match_asset(&release)?;
        let version_dir = extension_dir.join(CACHE_ROOT).join(&release.version);
        let binary_path = version_dir.join(binary_name);

        if binary_path.is_file() {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::None,
            );
            return Ok(binary_path);
        }

        fs::create_dir_all(&version_dir)
            .map_err(|err| format!("failed to create cache directory '{version_dir:?}': {err}"))?;

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::Downloading,
        );

        zed::download_file(
            &asset.download_url,
            version_dir
                .to_str()
                .ok_or_else(|| "failed to stringify cache directory path".to_string())?,
            DownloadedFileType::Zip,
        )
        .map_err(|err| format!("failed to download mermaid-lsp asset: {err}"))?;

        if !binary_path.is_file() {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Failed(format!(
                    "downloaded asset '{}' did not contain expected binary '{}'.",
                    asset.name, binary_name
                )),
            );
            return Err(format!(
                "downloaded asset '{asset_name}' did not contain expected binary '{binary_name}'",
                asset_name = asset.name
            ));
        }

        zed::make_file_executable(
            binary_path
                .to_str()
                .ok_or_else(|| "failed to stringify downloaded binary path".to_string())?,
        )?;

        Self::purge_old_cache_versions(extension_dir, &release.version);

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

        let expected_name = format!("mermaid-lsp-{os}-{arch}.zip", os = os_str, arch = arch_str);

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
