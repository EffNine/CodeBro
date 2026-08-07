# Structured Output Report

## Overview

Structured output allows models to return JSON that conforms to a specific schema. The AI Runtime provides schema definition, validation, and builder patterns.

## Schema Definition

### StructuredOutputSchema

```rust
pub struct StructuredOutputSchema {
    pub name: String,
    pub description: String,
    pub properties: serde_json::Value,
    pub required: Vec<String>,
    pub additional_properties: bool,
}
```

### JsonSchema Types

```rust
pub enum JsonSchema {
    StringSchema { description, pattern, enum_values },
    NumberSchema { description, minimum, maximum },
    IntegerSchema { description, minimum, maximum },
    BooleanSchema { description },
    ArraySchema { description, items },
    ObjectSchema { description, properties, required },
    AnySchema { description },
}
```

## Builder Pattern

```rust
let schema = StructuredOutputBuilder::new("Person", "A human being")
    .add_property("name", JsonSchema::string("Full name"))
    .add_property("age", JsonSchema::integer("Age in years"))
    .add_required("name")
    .with_additional_properties(false)
    .build();
```

## Validation

### StructuredOutputValidator

```rust
pub struct StructuredOutputValidator;

impl StructuredOutputValidator {
    pub fn new() -> Self;
    pub fn validate(&self, schema, json) -> Vec<String>;
    pub fn validate_strict(&self, schema, json) -> Result<(), Vec<String>>;
}
```

### Validation Rules

1. Root must be an object
2. All required fields must be present
3. Additional properties are rejected if `additional_properties` is false
4. Type checking against schema definitions

## JSON Schema Conversion

```rust
let value = JsonSchema::string("A name").to_value();
// Returns: {"type": "string", "description": "A name"}
```

## Example: Person Schema

```rust
let schema = StructuredOutputBuilder::new("Person", "A human being")
    .add_property("name", JsonSchema::string("Full name"))
    .add_property("age", JsonSchema::integer("Age in years"))
    .add_property("active", JsonSchema::boolean("Is active"))
    .add_property("tags", JsonSchema::ArraySchema {
        description: "List of tags".to_string(),
        items: Box::new(JsonSchema::string("Tag")),
    })
    .add_required("name")
    .build();
```

Generated JSON Schema:
```json
{
  "name": "Person",
  "description": "A human being",
  "properties": {
    "name": {"type": "string", "description": "Full name"},
    "age": {"type": "integer", "description": "Age in years"},
    "active": {"type": "boolean", "description": "Is active"},
    "tags": {
      "type": "array",
      "description": "List of tags",
      "items": {"type": "string", "description": "Tag"}
    }
  },
  "required": ["name"],
  "additional_properties": false
}
```

## Validation Examples

### Valid Input

```rust
let json = serde_json::json!({
    "name": "Alice",
    "age": 30,
    "active": true
});
assert!(validator.validate_strict(&schema, &json).is_ok());
```

### Missing Required Field

```rust
let json = serde_json::json!({
    "age": 30
});
let errors = validator.validate(&schema, &json);
assert_eq!(errors.len(), 1);
assert!(errors[0].contains("name"));
```

### Wrong Type

```rust
let json = serde_json::json!({
    "name": 123  // Should be string
});
let errors = validator.validate(&schema, &json);
// Validation depends on strictness
```

## Test Coverage

15 structured output tests covering:
- Schema validation
- JSON schema conversion
- Builder pattern
- Validator strict/non-strict modes
- Required field checking
- Type validation
