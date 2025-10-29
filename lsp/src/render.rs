use anyhow::{anyhow, Result};
use std::fs;
use std::process::Command;

/// Render Mermaid code to SVG using the mmdc CLI
pub fn render_mermaid(mermaid_code: &str) -> Result<String> {
    // Validate input
    if mermaid_code.trim().is_empty() {
        return Err(anyhow!("Mermaid code is empty"));
    }

    // Create temporary file for the Mermaid input
    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(format!("mermaid_{}.mmd",
        std::process::id()));

    // Write the Mermaid content to temp file
    fs::write(&temp_path, mermaid_code)
        .map_err(|e| anyhow!("Failed to write temp file: {}", e))?;

    // Render using mmdc
    let output = Command::new("mmdc")
        .arg("-i")
        .arg(&temp_path)
        .arg("-o")
        .arg("-") // stdout output
        .arg("-t") // transparent background
        .arg("-w")
        .arg("1200") // default width
        .arg("-H")
        .arg("800") // default height
        .output()
        .map_err(|e| anyhow!("Failed to execute mmdc: {}", e))?;

    // Clean up temp file
    let _ = fs::remove_file(&temp_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Mermaid CLI error: {}", stderr));
    }

    let svg_string = String::from_utf8(output.stdout)
        .map_err(|e| anyhow!("Failed to parse SVG output: {}", e))?;

    if svg_string.trim().is_empty() {
        return Err(anyhow!("Mermaid CLI produced empty output"));
    }

    // Sanitize SVG to remove scripts and dangerous attributes
    let sanitized_svg = sanitize_svg(&svg_string)?;

    Ok(sanitized_svg)
}

/// Basic SVG sanitization to remove potentially dangerous content
fn sanitize_svg(svg: &str) -> Result<String> {
    // Check for obviously dangerous content - scripts are bad, foreignObject is okay for Mermaid
    if svg.contains("<script") {
        return Err(anyhow!("SVG contains potentially dangerous content"));
    }

    // Simple string-based sanitization to avoid regex dependency issues
    let mut sanitized = svg.to_string();

    // Remove common event handlers
    let dangerous_attributes = [
        "onclick", "onload", "onerror", "onmouseover", "onmouseout",
        "onfocus", "onblur", "onchange", "onsubmit", "onreset",
        "onkeydown", "onkeyup", "onkeypress", "onmousedown", "onmouseup",
        "onmousemove", "ondrag", "ondrop", "ontouchstart", "ontouchend",
    ];

    for attr in &dangerous_attributes {
        // Simple pattern matching for attribute removal
        let pattern = format!("{}=\"", attr);
        while let Some(start) = sanitized.find(&pattern) {
            if let Some(end) = sanitized[start..].find('"') {
                let end = start + end + 1;
                sanitized.replace_range(start..end, "");
            } else {
                break;
            }
        }
    }

    // Remove javascript: URLs
    let js_pattern = "javascript:";
    while let Some(start) = sanitized.find(js_pattern) {
        if let Some(end) = sanitized[start..].find('"') {
            let end = start + end + 1;
            sanitized.replace_range(start..end, "");
        } else {
            break;
        }
    }

    Ok(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_render() {
        let mermaid_code = r#"
graph TD
    A[Start] --> B{Question}
    B -->|Yes| C[Action 1]
    B -->|No| D[Action 2]
    C --> E[End]
    D --> E
"#;

        // This test requires mmdc to be installed
        // Skip if mmdc is not available
        if Command::new("mmdc").arg("--version").output().is_ok() {
            let result = render_mermaid(mermaid_code);
            assert!(result.is_ok(), "Failed to render simple diagram: {:?}", result.err());

            let svg = result.unwrap();
            assert!(svg.starts_with("<svg"), "Output should be SVG");
            assert!(svg.contains("Start"), "SVG should contain diagram content");
        }
    }

    #[test]
    fn test_sanitize_svg() {
        let dangerous_svg = r#"<svg><script>alert('xss')</script><rect onclick="alert('xss')" /></svg>"#;
        let result = sanitize_svg(dangerous_svg);
        assert!(result.is_err(), "Should reject SVG with script tag");

        let safe_svg = r#"<svg><rect x="10" y="10" width="100" height="100" /></svg>"#;
        let result = sanitize_svg(safe_svg);
        assert!(result.is_ok(), "Should accept safe SVG");
    }
}