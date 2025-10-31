use anyhow::{anyhow, Result};
use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
};
use tempfile::tempdir;
use once_cell::sync::Lazy;
use regex::Regex;
use html_escape;

// Precompiled regex patterns to avoid DoS and improve performance
static FOREIGN_OBJECT_REGEX: Lazy<Regex> = Lazy::new(|| {
    // More efficient pattern that prevents catastrophic backtracking:
    // - Use [^<]+ instead of .*? to match content (stops at first <)
    // - Atomic grouping behavior by being more specific
    Regex::new(r#"<foreignObject\s+[^>]+>([^<]+(?:<(?!/foreignObject>)[^<]*)*)</foreignObject>"#)
        .expect("Foreign object regex should compile")
});

/// Render Mermaid code to SVG using mmdc and sanitize the output.
pub fn render_mermaid(mermaid_code: &str) -> Result<String> {
    if mermaid_code.trim().is_empty() {
        return Err(anyhow!("Mermaid code is empty"));
    }

    let mmdc_path = mmdc_path()?;

    let temp_dir = tempdir().map_err(|e| anyhow!("Failed to create temp dir: {}", e))?;
    let input_path = temp_dir.path().join("diagram.mmd");
    let output_path = temp_dir.path().join("diagram.svg");
    let config_path = temp_dir.path().join("mermaid-config.json");

    // Write mermaid code and config
    fs::write(&input_path, mermaid_code)
        .map_err(|e| anyhow!("Failed to write temp Mermaid file: {}", e))?;

    fs::write(&config_path, include_str!("mermaid-config.json"))
        .map_err(|e| anyhow!("Failed to write temp config file: {}", e))?;

    // Run mmdc with configuration file for htmlLabels: false
    let output = Command::new(&mmdc_path)
        .arg("-i")
        .arg(&input_path)
        .arg("-o")
        .arg(&output_path)
        .arg("-c")
        .arg(&config_path)
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
    let mut result = svg.to_string();

    // Process each foreignObject using precompiled regex
    while let Some(caps) = FOREIGN_OBJECT_REGEX.captures(&result) {
        let full_match = caps.get(0).unwrap().as_str();
        let content = caps.get(1).unwrap().as_str();

        // Extract text from HTML content
        let text = extract_text_from_html(content);

        // Skip empty or zero-size foreignObjects (these are often edge labels without content)
        if text.trim().is_empty() {
            result = result.replace(full_match, "");
            continue;
        }

        // Try to extract transform attribute first (used in class diagrams)
        let text_element = if let Some(transform) = extract_attr(full_match, "transform") {
            // Class diagrams use transform="translate(x, y)" for positioning
            // Preserve the transform to maintain correct positioning
            format!(
                "<text transform=\"{}\" text-anchor=\"start\" dominant-baseline=\"hanging\" font-family=\"Arial, sans-serif\" font-size=\"14\" fill=\"#333\">{}</text>",
                transform, text
            )
        } else {
            // Fallback to x/y attributes with centering (for simple diagrams)
            let x = extract_attr(full_match, "x").unwrap_or("0".to_string());
            let y = extract_attr(full_match, "y").unwrap_or("0".to_string());
            let width = extract_attr(full_match, "width").unwrap_or("0".to_string());
            let height = extract_attr(full_match, "height").unwrap_or("0".to_string());

            let x_val = x.parse::<f64>().unwrap_or(0.0);
            let y_val = y.parse::<f64>().unwrap_or(0.0);
            let width_val = width.parse::<f64>().unwrap_or(0.0);
            let height_val = height.parse::<f64>().unwrap_or(0.0);

            // Skip zero-size elements
            if width_val <= 0.0 || height_val <= 0.0 {
                result = result.replace(full_match, "");
                continue;
            }

            let center_x = x_val + width_val / 2.0;
            let center_y = y_val + height_val / 2.0;

            format!(
                "<text x=\"{:.2}\" y=\"{:.2}\" text-anchor=\"middle\" dominant-baseline=\"middle\" font-family=\"Arial, sans-serif\" font-size=\"14\" fill=\"#333\">{}</text>",
                center_x, center_y, text
            )
        };

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
        // Should be positioned at center (10 + 80/2 = 50, 10 + 30/2 = 25)
        assert!(sanitized.contains("x=\"50.00\""));
        assert!(sanitized.contains("y=\"25.00\""));
    }

    #[test]
    fn centers_text_correctly_in_foreignObject() {
        let svg = r#"<svg width="200" height="100"><foreignObject x="20" y="30" width="160" height="40"><div><p>Test Label</p></div></foreignObject></svg>"#;
        let sanitized = sanitize_svg(svg).unwrap();
        // Should be positioned at center (20 + 160/2 = 100, 30 + 40/2 = 50)
        assert!(sanitized.contains("x=\"100.00\""));
        assert!(sanitized.contains("y=\"50.00\""));
        assert!(sanitized.contains("Test Label"));
    }

    #[test]
    fn skips_empty_foreignObjects() {
        let svg = r#"<svg width="100" height="50"><foreignObject x="0" y="0" width="0" height="0"><div></div></foreignObject></svg>"#;
        let sanitized = sanitize_svg(svg).unwrap();
        // Should remove empty foreignObject entirely
        assert!(!sanitized.contains("foreignObject"));
        assert!(!sanitized.contains("<text"));
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