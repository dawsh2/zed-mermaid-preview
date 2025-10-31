# Test Mermaid Labels - v0.1.21

This test file verifies that all labels are visible with the new mermaid-preview extension v0.1.21 that uses the built-in `htmlLabels: false` configuration.

## Simple Flowchart

```mermaid
graph TD
    A[Start Node] --> B[Process Data]
    B --> C{Decision Point?}
    C -->|Yes| D[Success State]
    C -->|No| E[Failure State]
    E --> F[End Node]
```

## Complex Diagram with Multiple Node Types

```mermaid
graph LR
    subgraph Input Phase
        A1[User Input] --> A2[Validate Data]
        A2 --> A3[Parse Request]
    end

    subgraph Processing
        B1[Core Logic] --> B2[Transform]
        B2 --> B3[Calculate Result]
    end

    subgraph Output
        C1[Format Output] --> C2[Send Response]
    end

    A3 --> B1
    B3 --> C1

    style A1 fill:#e1f5fe
    style B1 fill:#f3e5f5
    style C1 fill:#e8f5e9
```

## Sequence Diagram

```mermaid
sequenceDiagram
    participant Client
    participant Server
    participant Database

    Client->>Server: Send Request
    Server->>Database: Query Data
    Database-->>Server: Return Results
    Server-->>Client: Send Response
```

All labels should be visible:
- Node labels: Start Node, Process Data, Decision Point?, etc.
- Edge labels: Yes, No, Send Request, etc.
- Subgraph labels: Input Phase, Processing, Output

The diagrams should render with proper SVG text elements instead of foreignObject elements.