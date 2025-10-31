use anyhow::{anyhow, Result};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tempfile::tempdir;
use once_cell::sync::Lazy;
use regex::Regex;
use html_escape;

/// Render Mermaid code to SVG using mmdc and sanitize the output.
pub fn render_mermaid(mermaid_code: &str) -> Result<String> {
    if mermaid_code.trim().is_empty() {
        return Err(anyhow!("Mermaid code is empty"));
    }

    let mmdc_path = mmdc_path()?;

    let temp_dir = tempdir().map_err(|e| anyhow!("Failed to create temp dir: {}", e))?;
    let input_path = temp_dir.path().join("diagram.mmd");
    let output_path = temp_dir.path().join("diagram.svg");

    fs::write(&input_path, mermaid_code)
        .map_err(|e| anyhow!("Failed to write temp Mermaid file: {}", e))?;

    // Run mmdc with htmlLabels disabled
    let output = Command::new(&mmdc_path)
        .arg("-i")
        .arg(&input_path)
        .arg("-o")
        .arg(&output_path)
        .arg("-t")
        .arg("default")
        .arg("-b")
        .arg("white")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| anyhow!("Failed to execute mmdc: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("mmdc error: {}", stderr.trim()));
    }

    let svg_contents = fs::read_to_string(&output_path)
        .map_err(|e| anyhow!("Failed to read SVG output: {}", e))?;

    let sanitized = sanitize_svg(&svg_contents)?;

    Ok(sanitized)
}

fn sanitize_svg(svg: &str) -> Result<String> {
    // SECURITY: Case-insensitive script tag detection to prevent XSS
    if svg.to_lowercase().contains("<script") {
        return Err(anyhow!("SVG contains <script> elements"));
    }

    let mut sanitized = svg.to_string();

    // Remove event handlers
    sanitized = EVENT_HANDLER_ATTR.replace_all(&sanitized, "").into_owned();
    sanitized = JAVASCRIPT_HREF_ATTR
        .replace_all(&sanitized, "")
        .into_owned();

    // Convert foreignObject elements to text
    sanitized = convert_foreign_objects_to_text(&sanitized)?;

    Ok(sanitized)
}

fn convert_foreign_objects_to_text(svg: &str) -> Result<String> {
    let foreign_object_regex = Regex::new(r#"<foreignObject[^>]*>(.*?)</foreignObject>"#)
        .map_err(|e| anyhow!("Failed to compile foreignObject regex: {}", e))?;

    let mut result = svg.to_string();

    // Process each foreignObject
    while let Some(caps) = foreign_object_regex.captures(&result) {
        let full_match = caps.get(0).unwrap().as_str();
        let content = caps.get(1).unwrap().as_str();

        // Extract text from HTML content
        let text = extract_text_from_html(content);

        // Get positioning attributes from foreignObject tag
        let x = extract_attr(full_match, "x").unwrap_or("0".to_string());
        let y = extract_attr(full_match, "y").unwrap_or("0".to_string());

        // Create a text element to replace foreignObject
        let text_element = format!(
            "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\" font-family=\"Arial, sans-serif\" font-size=\"14\" fill=\"#333\">{}</text>",
            x, y, text
        );

        result = result.replace(full_match, &text_element);
    }

    Ok(result)
}

fn extract_text_from_html(html: &str) -> String {
    // Simple HTML text extraction - strip tags and decode entities
    let no_tags = HTML_TAG_REGEX.replace_all(html, "");
    let decoded = html_escape::decode_html_entities(&no_tags);
    decoded.trim().to_string()
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let attr_regex = Regex::new(&format!(r#"{}="([^"]*)""#, regex::escape(attr)))
        .ok()?;
    attr_regex.captures(tag).map(|c| c[1].to_string())
}

fn mmdc_path() -> Result<PathBuf> {
    // First check for MMDC_PATH environment variable
    if let Ok(path) = env::var("MMDC_PATH") {
        let candidate = PathBuf::from(&path);
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err(anyhow!(
            "MMDC_PATH points to '{}', but it is not a file",
            candidate.display()
        ));
    }

    // Try to find mmdc in PATH
    if let Ok(mmdc_path) = which::which("mmdc") {
        return Ok(mmdc_path);
    }

    Err(anyhow!("mmdc not found in PATH"))
}

// Regex patterns for security
static EVENT_HANDLER_ATTR: Lazy<Regex> = Lazy::new(|| {
    Regex::new("(?i)\\s+on[a-z0-9_.:-]+\\s*=\\s*(?:\\\"[^\\\"]*\\\"|'[^']*'|[^\\s>]+)")
        .expect("valid regex for event handler attributes")
});

static JAVASCRIPT_HREF_ATTR: Lazy<Regex> = Lazy::new(|| {
    Regex::new("(?i)\\s+(?:xlink:)?href\\s*=\\s*(?:\\\"\\s*javascript:[^\\\"]*\\\"|'\\s*javascript:[^']*')")
        .expect("valid regex for javascript href attributes")
});

static HTML_TAG_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"<[^>]*>").expect("valid regex for HTML tags")
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