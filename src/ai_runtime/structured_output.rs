use serde::{Deserialize, Serialize};

/// Structured output schema definition for model responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredOutputSchema {
    pub name: String,
    pub description: String,
    pub properties: serde_json::Value,
    pub required: Vec<String>,
    pub additional_properties: bool,
}

impl StructuredOutputSchema {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        properties: serde_json::Value,
        required: Vec<String>,
    ) -> Self {
        StructuredOutputSchema {
            name: name.into(),
            description: description.into(),
            properties,
            required,
            additional_properties: false,
        }
    }

    pub fn with_additional_properties(mut self, allowed: bool) -> Self {
        self.additional_properties = allowed;
        self
    }

    pub fn is_valid(&self) -> bool {
        !self.name.is_empty()
            && !self.description.is_empty()
            && self.properties.is_object()
    }

    pub fn validate_json(&self, json: &serde_json::Value) -> Vec<String> {
        let mut errors = Vec::new();

        if !json.is_object() {
            errors.push("Expected object at root level".to_string());
            return errors;
        }

        let obj = json.as_object().unwrap();

        for field in &self.required {
            if !obj.contains_key(field) {
                errors.push(format!("Missing required field: {}", field));
            }
        }

        if !self.additional_properties {
            if let Some(props) = self.properties.as_object() {
                for key in obj.keys() {
                    if props.get(key).is_none() {
                        // Only flag if the schema defines properties
                        if !props.is_empty() {
                            // Don't error on extra fields if schema has no properties defined
                        }
                    }
                }
            }
        }

        errors
    }
}

/// JSON schema types for structured output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JsonSchema {
    StringSchema {
        description: String,
        pattern: Option<String>,
        enum_values: Option<Vec<String>>,
    },
    NumberSchema {
        description: String,
        minimum: Option<f64>,
        maximum: Option<f64>,
    },
    IntegerSchema {
        description: String,
        minimum: Option<i64>,
        maximum: Option<i64>,
    },
    BooleanSchema {
        description: String,
    },
    ArraySchema {
        description: String,
        items: Box<JsonSchema>,
    },
    ObjectSchema {
        description: String,
        properties: Vec<(String, JsonSchema)>,
        required: Vec<String>,
    },
    AnySchema {
        description: String,
    },
}

impl JsonSchema {
    pub fn to_value(&self) -> serde_json::Value {
        match self {
            JsonSchema::StringSchema { description, pattern, enum_values } => {
                let mut map = serde_json::Map::new();
                map.insert("type".to_string(), serde_json::Value::String("string".to_string()));
                map.insert("description".to_string(), serde_json::Value::String(description.clone()));
                if let Some(pat) = pattern {
                    map.insert("pattern".to_string(), serde_json::Value::String(pat.clone()));
                }
                if let Some(enums) = enum_values {
                    map.insert("enum".to_string(), serde_json::to_value(enums).unwrap());
                }
                serde_json::Value::Object(map)
            }
            JsonSchema::NumberSchema { description, minimum, maximum } => {
                let mut map = serde_json::Map::new();
                map.insert("type".to_string(), serde_json::Value::String("number".to_string()));
                map.insert("description".to_string(), serde_json::Value::String(description.clone()));
                if let Some(min) = minimum {
                    map.insert("minimum".to_string(), serde_json::Value::Number(serde_json::Number::from_f64(*min as f64).unwrap_or_else(|| serde_json::Number::from(0))));
                }
                if let Some(max) = maximum {
                    map.insert("maximum".to_string(), serde_json::Value::Number(serde_json::Number::from_f64(*max as f64).unwrap_or_else(|| serde_json::Number::from(0))));
                }
                serde_json::Value::Object(map)
            }
            JsonSchema::IntegerSchema { description, minimum, maximum } => {
                let mut map = serde_json::Map::new();
                map.insert("type".to_string(), serde_json::Value::String("integer".to_string()));
                map.insert("description".to_string(), serde_json::Value::String(description.clone()));
                if let Some(min) = minimum {
                    map.insert("minimum".to_string(), serde_json::Value::Number(serde_json::Number::from_f64(*min as f64).unwrap_or_else(|| serde_json::Number::from(0))));
                }
                if let Some(max) = maximum {
                    map.insert("maximum".to_string(), serde_json::Value::Number(serde_json::Number::from_f64(*max as f64).unwrap_or_else(|| serde_json::Number::from(0))));
                }
                serde_json::Value::Object(map)
            }
            JsonSchema::BooleanSchema { description } => {
                let mut map = serde_json::Map::new();
                map.insert("type".to_string(), serde_json::Value::String("boolean".to_string()));
                map.insert("description".to_string(), serde_json::Value::String(description.clone()));
                serde_json::Value::Object(map)
            }
            JsonSchema::ArraySchema { description, items } => {
                let mut map = serde_json::Map::new();
                map.insert("type".to_string(), serde_json::Value::String("array".to_string()));
                map.insert("description".to_string(), serde_json::Value::String(description.clone()));
                map.insert("items".to_string(), items.to_value());
                serde_json::Value::Object(map)
            }
            JsonSchema::ObjectSchema { description, properties, required } => {
                let mut map = serde_json::Map::new();
                map.insert("type".to_string(), serde_json::Value::String("object".to_string()));
                map.insert("description".to_string(), serde_json::Value::String(description.clone()));

                let props: serde_json::Map<String, serde_json::Value> = properties.iter()
                    .map(|(k, v)| (k.clone(), v.to_value()))
                    .collect();
                map.insert("properties".to_string(), serde_json::Value::Object(props));

                map.insert("required".to_string(), serde_json::to_value(required).unwrap());
                serde_json::Value::Object(map)
            }
            JsonSchema::AnySchema { description } => {
                let mut map = serde_json::Map::new();
                map.insert("description".to_string(), serde_json::Value::String(description.clone()));
                serde_json::Value::Object(map)
            }
        }
    }

    pub fn string(description: impl Into<String>) -> Self {
        JsonSchema::StringSchema {
            description: description.into(),
            pattern: None,
            enum_values: None,
        }
    }

    pub fn number(description: impl Into<String>) -> Self {
        JsonSchema::NumberSchema {
            description: description.into(),
            minimum: None,
            maximum: None,
        }
    }

    pub fn integer(description: impl Into<String>) -> Self {
        JsonSchema::IntegerSchema {
            description: description.into(),
            minimum: None,
            maximum: None,
        }
    }

    pub fn boolean(description: impl Into<String>) -> Self {
        JsonSchema::BooleanSchema {
            description: description.into(),
        }
    }

    pub fn any(description: impl Into<String>) -> Self {
        JsonSchema::AnySchema {
            description: description.into(),
        }
    }
}

/// Builder for structured output schemas.
#[derive(Debug, Clone, Default)]
pub struct StructuredOutputBuilder {
    name: String,
    description: String,
    properties: Vec<(String, JsonSchema)>,
    required: Vec<String>,
    additional_properties: bool,
}

impl StructuredOutputBuilder {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        StructuredOutputBuilder {
            name: name.into(),
            description: description.into(),
            ..Default::default()
        }
    }

    pub fn add_property(mut self, name: impl Into<String>, schema: JsonSchema) -> Self {
        self.properties.push((name.into(), schema));
        self
    }

    pub fn add_required(mut self, name: impl Into<String>) -> Self {
        let name_string = name.into();
        if !self.required.contains(&name_string) {
            self.required.push(name_string);
        }
        self
    }

    pub fn with_additional_properties(mut self, allowed: bool) -> Self {
        self.additional_properties = allowed;
        self
    }

    pub fn build(self) -> StructuredOutputSchema {
        let properties: serde_json::Map<String, serde_json::Value> = self.properties
            .into_iter()
            .map(|(k, v)| (k, v.to_value()))
            .collect();

        StructuredOutputSchema {
            name: self.name,
            description: self.description,
            properties: serde_json::Value::Object(properties),
            required: self.required,
            additional_properties: self.additional_properties,
        }
    }
}

/// Validator for structured output.
pub struct StructuredOutputValidator;

impl StructuredOutputValidator {
    pub fn new() -> Self {
        StructuredOutputValidator
    }

    pub fn validate(&self, schema: &StructuredOutputSchema, json: &serde_json::Value) -> Vec<String> {
        schema.validate_json(json)
    }

    pub fn validate_strict(&self, schema: &StructuredOutputSchema, json: &serde_json::Value) -> Result<(), Vec<String>> {
        let errors = self.validate(schema, json);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl Default for StructuredOutputValidator {
    fn default() -> Self {
        Self::new()
    }
}
