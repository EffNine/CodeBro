//! Engineering constraints — rules and conventions the system must
//! respect during task execution.

use serde::{Deserialize, Serialize};

/// A single engineering constraint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineeringConstraint {
    pub description: String,
    pub category: ConstraintCategory,
}

/// Category of an engineering constraint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstraintCategory {
    Language,
    Framework,
    Architecture,
    Security,
    Performance,
    Convention,
    Other,
}

impl std::fmt::Display for ConstraintCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstraintCategory::Language => write!(f, "language"),
            ConstraintCategory::Framework => write!(f, "framework"),
            ConstraintCategory::Architecture => write!(f, "architecture"),
            ConstraintCategory::Security => write!(f, "security"),
            ConstraintCategory::Performance => write!(f, "performance"),
            ConstraintCategory::Convention => write!(f, "convention"),
            ConstraintCategory::Other => write!(f, "other"),
        }
    }
}

/// Immutable collection of engineering constraints.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConstraintContext {
    pub constraints: Vec<EngineeringConstraint>,
}

impl ConstraintContext {
    pub fn new() -> Self {
        ConstraintContext {
            constraints: Vec::new(),
        }
    }

    pub fn with_constraints(mut self, constraints: Vec<EngineeringConstraint>) -> Self {
        self.constraints = constraints;
        self.constraints
            .sort_by(|a, b| a.description.cmp(&b.description));
        self
    }

    pub fn add_constraint(mut self, constraint: EngineeringConstraint) -> Self {
        self.constraints.push(constraint);
        self.constraints
            .sort_by(|a, b| a.description.cmp(&b.description));
        self
    }

    pub fn constraint_count(&self) -> usize {
        self.constraints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty()
    }

    pub fn by_category(&self, category: &ConstraintCategory) -> Vec<&EngineeringConstraint> {
        self.constraints
            .iter()
            .filter(|c| &c.category == category)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_constraints() {
        let ctx = ConstraintContext::new();
        assert!(ctx.is_empty());
        assert_eq!(ctx.constraint_count(), 0);
    }

    #[test]
    fn test_constraints_with_entries() {
        let ctx = ConstraintContext::new()
            .with_constraints(vec![
                EngineeringConstraint {
                    description: "No raw SQL".to_string(),
                    category: ConstraintCategory::Architecture,
                },
                EngineeringConstraint {
                    description: "All errors wrapped".to_string(),
                    category: ConstraintCategory::Convention,
                },
            ]);

        assert_eq!(ctx.constraint_count(), 2);
        assert_eq!(ctx.constraints[0].description, "All errors wrapped");
        assert_eq!(ctx.constraints[1].description, "No raw SQL");
    }

    #[test]
    fn test_by_category() {
        let ctx = ConstraintContext::new()
            .add_constraint(EngineeringConstraint {
                description: "Use Rust 2021".to_string(),
                category: ConstraintCategory::Language,
            })
            .add_constraint(EngineeringConstraint {
                description: "No raw SQL".to_string(),
                category: ConstraintCategory::Architecture,
            });

        let lang = ctx.by_category(&ConstraintCategory::Language);
        assert_eq!(lang.len(), 1);
        assert_eq!(lang[0].description, "Use Rust 2021");

        let arch = ctx.by_category(&ConstraintCategory::Architecture);
        assert_eq!(arch.len(), 1);
        assert_eq!(arch[0].description, "No raw SQL");

        let empty = ctx.by_category(&ConstraintCategory::Security);
        assert!(empty.is_empty());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let ctx = ConstraintContext::new()
            .add_constraint(EngineeringConstraint {
                description: "test constraint".to_string(),
                category: ConstraintCategory::Other,
            });
        let json = serde_json::to_string(&ctx).expect("serialize");
        let decoded: ConstraintContext = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(ctx, decoded);
    }
}
