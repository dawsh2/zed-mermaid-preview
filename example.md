# Mermaid Preview Examples

This file demonstrates various Mermaid diagram types that you can render with the extension.

## Simple Flowchart

```mermaid
flowchart LR
    A[Start]:::green --> B[Process]:::blue
    B --> C[End]:::green

    classDef green fill:#9f6,stroke:#333,stroke-width:2px
    classDef blue fill:#69f,stroke:#333,stroke-width:2px
```

## Complex Flowchart with Decision Points

```mermaid
flowchart TD
    A[Start Process]:::start --> B{Is data valid?}:::decision
    B -->|Yes| C[Process Data]:::process
    B -->|No| D[Log Error]:::error
    C --> E{Success?}:::decision
    E -->|Yes| F[Save Results]:::success
    E -->|No| G[Retry]:::warning
    D --> H[Notify User]:::warning
    F --> I[End]:::end
    G --> C
    H --> I

    classDef start fill:#90EE90,stroke:#333,stroke-width:3px
    classDef decision fill:#FFD700,stroke:#333,stroke-width:2px
    classDef process fill:#87CEEB,stroke:#333,stroke-width:2px
    classDef success fill:#98FB98,stroke:#333,stroke-width:2px
    classDef error fill:#FF6B6B,stroke:#333,stroke-width:2px
    classDef warning fill:#FFA500,stroke:#333,stroke-width:2px
    classDef end fill:#DDA0DD,stroke:#333,stroke-width:3px
```

## Sequence Diagram

```mermaid
sequenceDiagram
    participant User
    participant API
    participant Database
    participant Cache

    User->>API: GET /data
    API->>Cache: Check cache
    Cache-->>API: Cache miss
    API->>Database: Query data
    Database-->>API: Data rows
    API->>Cache: Store in cache
    API-->>User: JSON response
```

## Class Diagram

```mermaid
classDiagram
    class Animal {
        +String name
        +int age
        +eat()
        +sleep()
    }
    class Dog {
        +String breed
        +bark()
        +wagTail()
    }
    class Cat {
        +String color
        +meow()
        +scratch()
    }
    Animal <|-- Dog
    Animal <|-- Cat
    Dog --> "1..*" Toy : has

    style Animal fill:#f9f,stroke:#333,stroke-width:2px
    style Dog fill:#bbf,stroke:#333,stroke-width:2px
    style Cat fill:#fbb,stroke:#333,stroke-width:2px
    style Toy fill:#ff9,stroke:#333,stroke-width:2px
```

## State Diagram

```mermaid
stateDiagram-v2
    [*] --> Idle
    Idle --> Processing: Start
    Processing --> Validating: Validate data
    Validating --> Processing: Retry
    Validating --> Storing: Valid
    Storing --> Done: Success
    Processing --> Error: Failure
    Error --> Idle: Reset
    Done --> Idle: Complete
```

## Gantt Chart

```mermaid
gantt
    title Project Timeline
    dateFormat  YYYY-MM-DD
    section Design Phase
    Research        :a1, 2024-01-01, 7d
    Wireframes     :after a1, 5d
    UI Design      :after a1, 10d
    section Development
    Backend API    :2024-01-15, 14d
    Frontend Dev   :2024-01-20, 14d
    Testing        :2024-02-01, 7d
    section Deployment
    Staging Deploy :2024-02-10, 3d
    Production    :2024-02-13, 2d
```

## Pie Chart

```mermaid
pie
    title Technology Stack 2024
    "JavaScript/TypeScript" : 45
    "Python" : 25
    "Rust" : 15
    "SQL" : 10
    "Other" : 5
```

## Journey Map

```mermaid
journey
    title User Onboarding Journey
    section Discovery
      Visit website: 5: User
      Sign up: 3: User
      Download app: 2: User
    section First Use
      Install app: 2: User
      Create account: 3: User
      Complete profile: 4: User
    section Engagement
      Use core feature: 5: User
      Share with friends: 4: User
      Become power user: 5: User
```

## Git Graph

```mermaid
gitGraph
    commit
    branch feature
    checkout feature
    commit
    commit
    checkout main
    merge feature
    commit
    branch hotfix
    checkout hotfix
    commit
    checkout main
    merge hotfix
    commit
```

## Entity Relationship Diagram

```mermaid
erDiagram
    Customer ||--o{ Order : places
    Order ||--|{ LineItem : contains
    Product ||--o{ LineItem : ordered
    Customer {
        int id PK
        string name
        string email
        datetime created_at
    }
    Order {
        int id PK
        int customer_id FK
        datetime order_date
        decimal total_amount
    }
    LineItem {
        int id PK
        int order_id FK
        int product_id FK
        int quantity
        decimal unit_price
    }
    Product {
        int id PK
        string name
        decimal price
        int stock_quantity
    }
```

## User Journey

```mermaid
journey
    title Online Shopping Experience
    section Browse
      Visit Homepage: 5: Visitor
      Search Products: 4: Visitor
      View Product Details: 5: Visitor
    section Purchase
      Add to Cart: 5: Shopper
      View Cart: 4: Shopper
      Checkout: 3: Shopper
      Payment: 2: Shopper
    section Post-Purchase
      Order Confirmation: 5: Customer
      Track Order: 4: Customer
      Receive Package: 5: Customer
      Leave Review: 3: Customer
```

## Mind Map

```mermaid
mindmap
  root((Mermaid))
    Diagram Types
      Flowcharts
        Simple
        Complex
      Sequence Diagrams
        Systems
        APIs
      Structural
        Class
        ER
    Features
      Rendering
        SVG export
        Themes
      Integration
        Markdown
        Editors
```

## Quadrant Chart

```mermaid
quadrantChart
    title Features vs Effort
    x-axis Low Effort --> High Effort
    y-axis Low Value --> High Value
    quadrant-1 Quick Wins
    quadrant-2 Major Projects
    quadrant-3 Fill-ins
    quadrant-4 Thankless Tasks
    Bug fixes: [0.3, 0.8]
    New features: [0.6, 0.9]
    Documentation: [0.2, 0.4]
    Code cleanup: [0.4, 0.3]
```

## Usage

1. Place your cursor in any of the mermaid code blocks above
2. Right-click and select **"Render Mermaid Diagram"**
3. Or select **"Render All X Mermaid Diagrams"** to render all at once
4. The source code will be saved to separate `.mmd` files for editing
5. Use **"Edit Mermaid Source"** on rendered images to restore the code blocks

Each diagram type demonstrates different capabilities of Mermaid and how the extension handles them. Try rendering them all at once with the bulk render feature!