use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::tempdir;
use which::which;

/// Render Mermaid code to SVG using the mmdc CLI and sanitize the output.
pub fn render_mermaid(mermaid_code: &str) -> Result<String> {
    if mermaid_code.trim().is_empty() {
        return Err(anyhow!("Mermaid code is empty"));
    }

    let cli_path = mermaid_cli_path()?;

    let temp_dir = tempdir().map_err(|e| anyhow!("Failed to create temp dir: {}", e))?;
    let input_path = temp_dir.path().join("diagram.mmd");
    let output_path = temp_dir.path().join("diagram.svg");

    fs::write(&input_path, mermaid_code)
        .map_err(|e| anyhow!("Failed to write temp Mermaid file: {}", e))?;

    let default_config_path = temp_dir.path().join("config.json");
    fs::write(
        &default_config_path,
        r#"{"flowchart":{"htmlLabels":true},"sequence":{"htmlLabels":true},"class":{"htmlLabels":true},"er":{"htmlLabels":true}}"#,
    )
    .map_err(|e| anyhow!("Failed to write Mermaid config: {}", e))?;

    let config_path = env::var("MERMAID_CONFIG")
        .map(PathBuf::from)
        .unwrap_or(default_config_path);

    let output = run_mermaid_cli(&cli_path, &input_path, &output_path, &config_path, false)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("unknown option '--disableHtmlLabels'") {
            let fallback =
                run_mermaid_cli(&cli_path, &input_path, &output_path, &config_path, false)?;
            if !fallback.status.success() {
                let stderr = String::from_utf8_lossy(&fallback.stderr);
                return Err(anyhow!("Mermaid CLI error: {}", stderr.trim()));
            }
        } else {
            return Err(anyhow!("Mermaid CLI error: {}", stderr.trim()));
        }
    }

    if !output_path.exists() {
        return Err(anyhow!("Mermaid CLI did not produce an SVG output"));
    }

    let svg_contents = fs::read_to_string(&output_path)
        .map_err(|e| anyhow!("Failed to read rendered SVG: {}", e))?;

    let sanitized = sanitize_svg(&svg_contents)?;

    Ok(sanitized)
}

fn sanitize_svg(svg: &str) -> Result<String> {
    if svg.contains("<script") {
        return Err(anyhow!("SVG contains <script> elements"));
    }

    let mut sanitized = svg.to_string();

    sanitized = EVENT_HANDLER_ATTR.replace_all(&sanitized, "").into_owned();
    sanitized = JAVASCRIPT_HREF_ATTR
        .replace_all(&sanitized, "")
        .into_owned();

    Ok(sanitized)
}

fn run_mermaid_cli(
    cli_path: &Path,
    input_path: &Path,
    output_path: &Path,
    config_path: &Path,
    disable_html_labels: bool,
) -> Result<std::process::Output> {
    let mut command = Command::new(cli_path);
    command
        .arg("-i")
        .arg(input_path)
        .arg("-o")
        .arg(output_path)
        .arg("-b")
        .arg("transparent")
        .arg("-c")
        .arg(config_path);

    if disable_html_labels {
        command.arg("--disableHtmlLabels");
    }

    command
        .output()
        .map_err(|e| anyhow!("Failed to execute mmdc: {}", e))
}

fn mermaid_cli_path() -> Result<PathBuf> {
    if let Ok(path) = env::var("MERMAID_CLI_PATH") {
        let candidate = PathBuf::from(&path);
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err(anyhow!(
            "MERMAID_CLI_PATH points to '{}', but it is not a file",
            candidate.display()
        ));
    }

    which("mmdc").map_err(|_| anyhow!("Mermaid CLI (mmdc) not found in PATH"))
}

static EVENT_HANDLER_ATTR: Lazy<Regex> = Lazy::new(|| {
    Regex::new("(?is)\\s+on[a-z0-9_.:-]+\\s*=\\s*(?:\"[^\"]*\"|'[^']*'|[^\\s>]+)")
        .expect("valid regex for event handler attributes")
});

static JAVASCRIPT_HREF_ATTR: Lazy<Regex> = Lazy::new(|| {
    Regex::new("(?is)\\s+(?:xlink:)?href\\s*=\\s*(?:\"\\s*javascript:[^\"]*\"|'\\s*javascript:[^']*')")
        .expect("valid regex for javascript href attributes")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_scripts() {
        let svg = "<svg><script>alert('xss')</script></svg>";
        assert!(sanitize_svg(svg).is_err());
    }

    #[test]
    fn removes_event_handlers() {
        let svg = "<svg><rect onclick=\"alert()\" width=\"10\" /></svg>";
        let sanitized = sanitize_svg(svg).unwrap();
        assert!(!sanitized.contains("onclick"));
        assert!(!sanitized.contains("alert()"));
        assert!(sanitized.contains("<rect"));
    }

    #[test]
    fn removes_event_handlers_with_single_quotes() {
        let svg = "<svg><rect onmouseover='doSomething()' width=\"10\" /></svg>";
        let sanitized = sanitize_svg(svg).unwrap();
        assert!(!sanitized.contains("onmouseover"));
        assert!(!sanitized.contains("doSomething()"));
    }

    #[test]
    fn removes_event_handlers_without_quotes() {
        let svg = "<svg><rect onload=init() width=\"10\" /></svg>";
        let sanitized = sanitize_svg(svg).unwrap();
        assert!(!sanitized.contains("onload"));
        assert!(!sanitized.contains("init()"));
    }

    #[test]
    fn removes_javascript_hrefs() {
        let svg = "<svg><a href=\"javascript:alert('xss')\">link</a></svg>";
        let sanitized = sanitize_svg(svg).unwrap();
        assert!(!sanitized.contains("javascript:"));
        assert!(!sanitized.contains("alert"));
    }

    #[test]
    fn removes_xlink_javascript_hrefs() {
        let svg = "<svg><a xlink:href='javascript:malicious()'>link</a></svg>";
        let sanitized = sanitize_svg(svg).unwrap();
        assert!(!sanitized.contains("javascript:"));
        assert!(!sanitized.contains("malicious"));
    }

    #[test]
    fn keeps_foreign_object_but_sanitizes() {
        let svg = "<svg><foreignObject><div onclick=\"alert()\">Label</div></foreignObject></svg>";
        let sanitized = sanitize_svg(svg).unwrap();
        assert!(sanitized.contains("foreignObject"));
        assert!(sanitized.contains("Label"));
        assert!(!sanitized.contains("onclick"));
    }

    #[test]
    fn regression_broken_sanitize_doesnt_leave_malformed_markup() {
        let svg = "<svg><rect onclick=\"alert('xss')\" width=\"10\" /></svg>";
        let sanitized = sanitize_svg(svg).unwrap();
        // Should not contain truncated attributes
        assert!(!sanitized.contains("onclick=\"alert('xss')\""));
        assert!(!sanitized.contains("alert('xss')\""));
        assert!(!sanitized.contains("…")); // ellipsis from truncation
        // Should be well-formed
        assert!(sanitized.contains("<rect"));
        assert!(sanitized.contains("width=\"10\""));
        assert!(sanitized.ends_with("</svg>"));
    }
}
