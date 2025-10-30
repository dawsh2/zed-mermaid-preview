# Test Edit Functionality

## Diagram 1

```mermaid
flowchart TD
    A[Start] --> B{Decision}
    B -->|Yes| C[Process]
    B -->|No| D[End]
    C --> D
```

## After Rendering

Once you render this, it should look like:

<!-- mermaid-source-file:.mermaid/test-edit-functionality_1730305000_0.mmd -->

![Mermaid Diagram](.mermaid/test-edit-functionality_diagram_1730305000_0.svg)

To edit: Place your cursor on any of the lines above (comment or image line) and the "Edit Mermaid Source" action should appear.