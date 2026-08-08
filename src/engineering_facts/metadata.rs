#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Immutable, deterministic metadata for every fact (P10.5).
//!
//! Metadata is free-form but canonical: tags and attributes are stored
//! sorted and de-duplicated, so two facts with identical metadata are `==`
//! and serialise byte-identically. Lookups (`has_tag`, `get`) run over the
//! sorted storage with zero heap allocation.

use serde::{Deserialize, Serialize};

/// A free-form tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tag(String);

impl Tag {
    pub fn new(tag: impl Into<String>) -> Self {
        Tag(tag.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Tag {
    fn from(s: &str) -> Self {
        Tag(s.to_string())
    }
}

impl From<String> for Tag {
    fn from(s: String) -> Self {
        Tag(s)
    }
}

impl AsRef<str> for Tag {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A single key/value attribute.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Attribute {
    pub key: String,
    pub value: String,
}

/// Immutable, deterministic metadata carried by every fact.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FactMetadata {
    /// Sorted, de-duplicated tags.
    pub tags: Vec<Tag>,
    /// Sorted (by key, then value) attribute pairs.
    pub attributes: Vec<Attribute>,
    pub description: Option<String>,
    pub language: Option<String>,
}

impl FactMetadata {
    pub fn new() -> Self {
        FactMetadata::default()
    }

    /// Start a builder that sorts and de-duplicates on `build`.
    pub fn builder() -> FactMetadataBuilder {
        FactMetadataBuilder::new()
    }

    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
            && self.attributes.is_empty()
            && self.description.is_none()
            && self.language.is_none()
    }

    /// O(log n) tag membership check over sorted storage; no allocation.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags
            .binary_search_by(|t| t.0.as_str().cmp(tag))
            .is_ok()
    }

    /// O(log n) attribute lookup by key; no allocation.
    pub fn get(&self, key: &str) -> Option<&str> {
        let idx = self.attributes.partition_point(|a| a.key.as_str() < key);
        self.attributes
            .get(idx)
            .filter(|a| a.key == key)
            .map(|a| a.value.as_str())
    }
}

/// Mutable builder that freezes into an immutable `FactMetadata`.
#[derive(Debug, Clone, Default)]
pub struct FactMetadataBuilder {
    tags: Vec<Tag>,
    attributes: Vec<Attribute>,
    description: Option<String>,
    language: Option<String>,
}

impl FactMetadataBuilder {
    pub fn new() -> Self {
        FactMetadataBuilder::default()
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(Tag(tag.into()));
        self
    }

    pub fn attr(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.push(Attribute {
            key: key.into(),
            value: value.into(),
        });
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Freeze into sorted, de-duplicated, immutable metadata.
    pub fn build(self) -> FactMetadata {
        let mut tags = self.tags;
        tags.sort();
        tags.dedup();

        let mut attributes = self.attributes;
        attributes.sort_by(|a, b| a.key.cmp(&b.key).then(a.value.cmp(&b.value)));
        attributes.dedup_by(|a, b| a.key == b.key && a.value == b.value);

        FactMetadata {
            tags,
            attributes,
            description: self.description,
            language: self.language,
        }
    }
}
