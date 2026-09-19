#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Language-independent visibility model (P10.5).
//!
//! Visibility is expressed only in the six language-independent categories
//! the Engineering Facts Model supports. There is no language-specific
//! semantics here: mapping provider-specific visibility to these categories
//! is the producer's responsibility.

use serde::{Deserialize, Serialize};

/// A language-independent visibility category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Protected,
    Internal,
    Private,
    Package,
    Unknown,
}

impl Visibility {
    /// All supported categories, in a stable order.
    pub const ALL: [Visibility; 6] = [
        Visibility::Public,
        Visibility::Protected,
        Visibility::Internal,
        Visibility::Private,
        Visibility::Package,
        Visibility::Unknown,
    ];

    /// Canonical lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Protected => "protected",
            Visibility::Internal => "internal",
            Visibility::Private => "private",
            Visibility::Package => "package",
            Visibility::Unknown => "unknown",
        }
    }

    /// Parse a canonical name. Strings outside the supported set map to
    /// `None` — the model never accepts language-specific visibility as a
    /// first-class value.
    pub fn parse(s: &str) -> Option<Visibility> {
        match s {
            "public" => Some(Visibility::Public),
            "protected" => Some(Visibility::Protected),
            "internal" => Some(Visibility::Internal),
            "private" => Some(Visibility::Private),
            "package" => Some(Visibility::Package),
            "unknown" => Some(Visibility::Unknown),
            _ => None,
        }
    }

    /// True when the visibility has been resolved to a real category.
    pub fn is_resolved(self) -> bool {
        !matches!(self, Visibility::Unknown)
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Visibility {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Visibility::parse(s).ok_or(())
    }
}
