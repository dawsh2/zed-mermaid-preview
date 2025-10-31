#!/usr/bin/env node

/**
 * Mermaid Renderer Server
 * Uses Mermaid API directly to render diagrams without external dependencies
 * Accepts Mermaid code via stdin and outputs SVG to stdout
 */

const { JSDOM } = require('jsdom');
const DOMPurify = require('dompurify');
const mermaid = require('mermaid').default;
const { readFileSync } = require('fs');

// Set up a minimal DOM environment for Mermaid
const dom = new JSDOM('<!DOCTYPE html><html><body><div id="container"></div></body></html>');
global.document = dom.window.document;
global.window = dom.window;

// Set up DOMPurify for Mermaid's internal use
const purify = DOMPurify(dom.window);
global.DOMPurify = purify;
// Also expose it on window for Mermaid
dom.window.DOMPurify = purify;

// Initialize Mermaid with deterministic configuration
mermaid.initialize({
    startOnLoad: false,
    flowchart: { htmlLabels: false },
    sequence: { htmlLabels: false },
    class: { htmlLabels: false },
    state: { htmlLabels: false },
    gantt: { htmlLabels: false },
    journey: { htmlLabels: false },
    er: { htmlLabels: false },
    info: { htmlLabels: false },
    pie: { htmlLabels: false },
    requirement: { htmlLabels: false },
    gitgraph: { htmlLabels: false },
    c4: { htmlLabels: false },
    mindmap: { htmlLabels: false },
    timeline: { htmlLabels: false },
    quadrantChart: { htmlLabels: false },
    sankey: { htmlLabels: false },
    block: { htmlLabels: false },
    architecture: { htmlLabels: false },
    network: { htmlLabels: false },
    // Security settings
    securityLevel: 'loose',
    fontFamily: 'monospace',
    fontSize: 14,
    logLevel: 1  // Disable logging
});

let diagramId = 0;

// Error handling
process.on('uncaughtException', (error) => {
    console.error('Uncaught exception:', error);
    process.exit(1);
});

process.on('unhandledRejection', (reason, promise) => {
    console.error('Unhandled rejection at:', promise, 'reason:', reason);
    process.exit(1);
});

// Main rendering function
async function renderMermaid(mermaidCode) {
    try {
        diagramId++;
        const id = `mermaid-${diagramId}`;

        // Append the container to body
        const container = document.getElementById('container');

        // Render the diagram using the render API
        const { svg } = await mermaid.render(id, mermaidCode, container);

        // Output the SVG
        process.stdout.write(svg);
        return true;
    } catch (error) {
        // Write error to stderr so LSP can capture it
        process.stderr.write(`Error rendering diagram: ${error.message}\n`);
        return false;
    }
}

// Read from stdin or from file if provided
async function main() {
    let mermaidCode = '';

    if (process.argv.length > 2) {
        // Read from file
        const filePath = process.argv[2];
        try {
            mermaidCode = readFileSync(filePath, 'utf8');
        } catch (error) {
            process.stderr.write(`Error reading file ${filePath}: ${error.message}\n`);
            process.exit(1);
        }
    } else {
        // Read from stdin
        let chunk;
        const encoding = 'utf8';

        process.stdin.setEncoding(encoding);

        for await (chunk of process.stdin) {
            mermaidCode += chunk;
        }
    }

    if (!mermaidCode.trim()) {
        process.stderr.write('Error: No Mermaid code provided\n');
        process.exit(1);
    }

    const success = await renderMermaid(mermaidCode);
    process.exit(success ? 0 : 1);
}

// Handle stdin close event
process.stdin.on('end', () => {
    // This will be handled in the async main function
});

// Start the server
main().catch(error => {
    console.error('Fatal error:', error);
    process.exit(1);
});