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
        r#"{"flowchart":{"htmlLabels":false},"sequence":{"htmlLabels":false},"class":{"htmlLabels":false},"er":{"htmlLabels":false}}"#,
    )
    .map_err(|e| anyhow!("Failed to write Mermaid config: {}", e))?;

    let config_path = env::var("MERMAID_CONFIG")
        .map(PathBuf::from)
        .unwrap_or(default_config_path);

    let output = run_mermaid_cli(&cli_path, &input_path, &output_path, &config_path, true)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("unknown option '--disableHtmlLabels'") {
            let fallback =
                run_mermaid_cli(&cli_path, &input_path, &output_path, &config_path, true)?;
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
    // SECURITY: Case-insensitive script tag detection to prevent XSS
    if svg.to_lowercase().contains("<script") {
        return Err(anyhow!("SVG contains <script> elements"));
    }

    let mut sanitized = svg.to_string();

    // Convert foreignObject elements to text elements for Zed compatibility
    // This is needed for mmdc v11+ which ignores htmlLabels: false
    sanitized = convert_foreign_objects_to_text(&sanitized);

    sanitized = EVENT_HANDLER_ATTR.replace_all(&sanitized, "").into_owned();
    sanitized = JAVASCRIPT_HREF_ATTR
        .replace_all(&sanitized, "")
        .into_owned();

    Ok(sanitized)
}

/// Convert foreignObject elements to SVG text elements for better compatibility
/// with markdown viewers that don't support foreignObject (like Zed)
/// This is needed for mmdc v11+ which ignores htmlLabels: false
fn convert_foreign_objects_to_text(svg: &str) -> String {
    let mut result = svg.to_string();

    // Process each foreignObject element
    while let Some(caps) = FOREIGNOBJECT_POSITION_REGEX.captures(&result) {
        if let (Some(x), Some(y), Some(width), Some(height)) = (
            caps.get(1).map(|m| m.as_str()),
            caps.get(2).map(|m| m.as_str()),
            caps.get(3).map(|m| m.as_str()),
            caps.get(4).map(|m| m.as_str()),
        ) {
            // Extract the full foreignObject element
            if let Some(fo_match) = FOREIGNOBJECT_REGEX.find(&result) {
                let fo_element = fo_match.as_str();

                // Extract text content from the div
                if let Some(text_caps) = DIV_TEXT_REGEX.captures(fo_element) {
                    if let Some(text_match) = text_caps.get(1) {
                        let mut text_content = text_match.as_str().trim().to_string();

                        // Remove any remaining HTML tags from the text
                        let html_tag_re = Regex::new(r#"<[^>]*>"#).unwrap();
                        text_content = html_tag_re.replace_all(&text_content, "").to_string();

                        // Calculate text positioning (center of the foreignObject)
                        let x_num: f32 = x.parse().unwrap_or(0.0);
                        let y_num: f32 = y.parse().unwrap_or(0.0);
                        let width_num: f32 = width.parse().unwrap_or(0.0);
                        let height_num: f32 = height.parse().unwrap_or(0.0);

                        let text_x = x_num + width_num / 2.0;
                        let text_y = y_num + height_num / 2.0 + 5.0; // +5 for better vertical alignment

                        // Create SVG text element
                        let svg_text = format!(
                            "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\" font-family=\"'trebuchet ms',verdana,arial,sans-serif\" font-size=\"16px\" fill=\"#333\">{}</text>",
                            text_x, text_y, html_escape::encode_text(&text_content)
                        );

                        // Replace the foreignObject with the text element
                        result = result.replace(fo_element, &svg_text);
                    }
                }
            }
        } else {
            break; // If we can't parse the position, break out to avoid infinite loop
        }
    }

    result
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

static FOREIGNOBJECT_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<foreignObject[^>]*>(?s).*?</foreignObject>"#)
        .expect("valid regex for foreignObject elements")
});

static DIV_TEXT_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<foreignObject[^>]*>.*?<div[^>]*>(?s)(.*?)</div>.*?</foreignObject>"#)
        .expect("valid regex for extracting text from foreignObject divs")
});

static FOREIGNOBJECT_POSITION_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<foreignObject[^>]*\bx="([^"]*)"[^>]*\by="([^"]*)"[^>]*\bwidth="([^"]*)"[^>]*\bheight="([^"]*)"[^>]*>"#)
        .expect("valid regex for extracting foreignObject position and dimensions")
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
    fn rejects_scripts_case_insensitive() {
        // Test various case combinations to ensure case-insensitive detection
        let test_cases = vec![
            "<svg><SCRIPT>alert('xss')</SCRIPT></svg>",
            "<svg><Script>alert('xss')</Script></svg>",
            "<svg><ScRiPt>alert('xss')</ScRiPt></svg>",
            "<svg><script language='javascript'>alert('xss')</script></svg>",
        ];

        for svg in test_cases {
            assert!(sanitize_svg(svg).is_err(), "Should reject case-insensitive script tags");
        }
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

    #[test]
    fn converts_foreign_objects_to_text() {
        let svg = r#"<svg width="100" height="50"><foreignObject x="10" y="10" width="80" height="30"><div style="text-align: center;">Start Here</div></foreignObject></svg>"#;
        let sanitized = sanitize_svg(svg).unwrap();
        // Should convert foreignObject to text element
        assert!(!sanitized.contains("foreignObject"));
        assert!(sanitized.contains("<text"));
        assert!(sanitized.contains("Start Here"));
        assert!(sanitized.contains("text-anchor=\"middle\""));
    }

    #[test]
    fn removes_html_tags_from_foreign_object_text() {
        let svg = r#"<svg width="100" height="50"><foreignObject x="10" y="10" width="80" height="30"><div><p>Label</p></div></foreignObject></svg>"#;
        let sanitized = sanitize_svg(svg).unwrap();
        // Should remove HTML tags but keep the text
        assert!(sanitized.contains("Label"));
        assert!(!sanitized.contains("<p>"));
        assert!(!sanitized.contains("</p>"));
    }
}
