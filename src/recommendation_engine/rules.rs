//! Recommendation Rules — deterministic rule-based recommendations.
//!
//! Each rule is a pure function: input → Option<Recommendation>.
//! No state, no side effects, no LLM calls.

use super::types::*;

/// A single recommendation rule.
#[derive(Debug, Clone)]
pub struct RecommendationRule {
    pub id: String,
    pub name: String,
    pub pattern: regex::Regex,
    pub rec_type: RecommendationType,
    pub title: &'static str,
    pub explanation: &'static str,
    pub confidence: f64,
    pub target_key: Option<&'static str>,
    pub target_value: Option<&'static str>,
    pub evidence_template: &'static str,
}

impl RecommendationRule {
    pub fn new(
        id: &str,
        name: &str,
        pattern: &str,
        rec_type: RecommendationType,
        title: &'static str,
        explanation: &'static str,
        confidence: f64,
        target_key: Option<&'static str>,
        target_value: Option<&'static str>,
        evidence_template: &'static str,
    ) -> Self {
        RecommendationRule {
            id: id.to_string(),
            name: name.to_string(),
            pattern: regex::Regex::new(pattern).unwrap(),
            rec_type,
            title,
            explanation,
            confidence,
            target_key,
            target_value,
            evidence_template,
        }
    }

    /// Check if this rule matches the given input.
    pub fn matches(&self, input: &str) -> bool {
        self.pattern.is_match(input)
    }

    /// Generate a recommendation if this rule matches.
    pub fn generate(
        &self,
        input: &str,
        related_intent_id: &str,
        context: &RecommendationContext,
    ) -> Option<Recommendation> {
        if !self.matches(input) {
            return None;
        }

        let evidence = vec![format!(
            "Matched rule: {} — {}",
            self.name, self.evidence_template
        )];

        let confidence = if self.confidence >= 0.8 {
            RecommendationConfidence::High(self.confidence)
        } else if self.confidence >= 0.5 {
            RecommendationConfidence::Medium(self.confidence)
        } else {
            RecommendationConfidence::Low(self.confidence)
        };

        Some(Recommendation::new(
            self.rec_type.clone(),
            self.title,
            self.explanation,
            evidence,
            confidence,
            &self.id,
            self.target_key.map(|s| s.to_string()),
            self.target_value.map(|s| s.to_string()),
            related_intent_id,
        ))
    }
}

/// Returns all registered recommendation rules.
pub fn all_rules() -> Vec<RecommendationRule> {
    vec![
        // ─── Keyboard / Layout Rules ──────────────────────────────────────
        RecommendationRule::new(
            "rule-vim-mode",
            "Vim Mode",
            r"(?i)(enable|use|switch\s+to|set)\s+(vim|nvim|neovim)",
            RecommendationType::Keyboard,
            "Enable Vim Keybindings",
            "If you're using Vim mode, consider enabling dedicated Vim keybindings for faster navigation.",
            0.85,
            Some("keybindings_vim"),
            Some("true"),
            "User enabled or switched to Vim mode",
        ),
        RecommendationRule::new(
            "rule-emacs-mode",
            "Emacs Mode",
            r"(?i)(enable|use|switch\s+to|set)\s+(emacs|emulate)",
            RecommendationType::Keyboard,
            "Enable Emacs Keybindings",
            "If you're using Emacs mode, consider enabling Emacs-style keybindings.",
            0.85,
            Some("keybindings_emacs"),
            Some("true"),
            "User enabled or switched to Emacs mode",
        ),
        RecommendationRule::new(
            "rule-compact-layout",
            "Compact Layout",
            r"(?i)(compact|dense|minimal|minimalist)\s+(layout|interface|ui|theme)",
            RecommendationType::Layout,
            "Enable Compact Layout",
            "Compact layouts benefit from dense sidebars and minimal chrome to maximize workspace.",
            0.75,
            Some("layout_compact"),
            Some("true"),
            "User requested compact or minimal layout",
        ),
        RecommendationRule::new(
            "rule-wide-layout",
            "Wide Layout",
            r"(?i)(wide|spacious|comfortable)\s+(layout|interface|ui)",
            RecommendationType::Layout,
            "Enable Wide Layout",
            "Wide layouts benefit from expanded side panels and comfortable spacing.",
            0.70,
            Some("layout_wide"),
            Some("true"),
            "User requested wide or spacious layout",
        ),

        // ─── Appearance Rules ─────────────────────────────────────────────
        RecommendationRule::new(
            "rule-dark-theme",
            "Dark Theme",
            r"(?i)(dark\s+)?theme|dark\s+(mode|ui|interface)",
            RecommendationType::Appearance,
            "Enable High Contrast",
            "Dark themes benefit from high-contrast text and carefully chosen accent colors.",
            0.70,
            Some("contrast_high"),
            Some("true"),
            "User enabled or requested dark theme",
        ),
        RecommendationRule::new(
            "rule-light-theme",
            "Light Theme",
            r"(?i)(light\s+)?theme|light\s+(mode|ui|interface)",
            RecommendationType::Appearance,
            "Enable Soft Contrast",
            "Light themes benefit from soft contrast and warm accent colors.",
            0.65,
            Some("contrast_soft"),
            Some("true"),
            "User enabled or requested light theme",
        ),
        RecommendationRule::new(
            "rule-high-contrast",
            "High Contrast",
            r"(?i)(high\s+)?contrast",
            RecommendationType::Appearance,
            "Enable Bold Accents",
            "High contrast mode benefits from bold accent colors and clear visual hierarchy.",
            0.80,
            Some("accent_bold"),
            Some("true"),
            "User requested high contrast mode",
        ),
        RecommendationRule::new(
            "rule-monochrome",
            "Monochrome",
            r"(?i)(mono|monochrome|grayscale|black.and.white)",
            RecommendationType::Appearance,
            "Enable Monochrome Palette",
            "Monochrome displays benefit from a restricted color palette optimized for clarity.",
            0.75,
            Some("palette_monochrome"),
            Some("true"),
            "User requested monochrome or grayscale display",
        ),

        // ─── Integration Rules ────────────────────────────────────────────
        RecommendationRule::new(
            "rule-git-integration",
            "Git Integration",
            r"(?i)(git|version.control|source.control)",
            RecommendationType::Integration,
            "Enable Git Decorations",
            "Git integration benefits from inline decorations showing branch, status, and changes.",
            0.80,
            Some("git_decorations"),
            Some("true"),
            "User enabled or referenced Git integration",
        ),
        RecommendationRule::new(
            "rule-git-status",
            "Git Status",
            r"(?i)(git\s+status|branch|commit|push|pull|merge|rebase)",
            RecommendationType::Integration,
            "Enable Git Status Bar",
            "Git operations benefit from a persistent status bar showing current branch and changes.",
            0.75,
            Some("git_status_bar"),
            Some("true"),
            "User referenced Git status or operations",
        ),
        RecommendationRule::new(
            "rule-lsp-integration",
            "LSP Integration",
            r"(?i)(lsp|language.server|autocomplete|intelligence)",
            RecommendationType::Integration,
            "Enable LSP Enhancements",
            "LSP integration benefits from enhanced autocomplete, diagnostics, and hover information.",
            0.80,
            Some("lsp_enabled"),
            Some("true"),
            "User enabled or referenced LSP features",
        ),
        RecommendationRule::new(
            "rule-terminal-integration",
            "Terminal Integration",
            r"(?i)(terminal|shell|command.line|cli)",
            RecommendationType::Integration,
            "Enable Terminal Integration",
            "Terminal workflows benefit from integrated shell panels with history and quick access.",
            0.70,
            Some("terminal_integrated"),
            Some("true"),
            "User referenced terminal or shell usage",
        ),

        // ─── Performance Rules ────────────────────────────────────────────
        RecommendationRule::new(
            "rule-large-project",
            "Large Project",
            r"(?i)(large\s+)?(project|codebase|repository)",
            RecommendationType::Performance,
            "Enable Incremental Indexing",
            "Large projects benefit from incremental indexing and lazy loading to improve responsiveness.",
            0.65,
            Some("indexing_incremental"),
            Some("true"),
            "User referenced large project or codebase",
        ),
        RecommendationRule::new(
            "rule-low-memory",
            "Low Memory",
            r"(?i)(low\s+)?memory|memory.(saving|efficient)",
            RecommendationType::Performance,
            "Enable Memory Saving Mode",
            "Memory-constrained environments benefit from reduced caching and aggressive cleanup.",
            0.75,
            Some("memory_saving"),
            Some("true"),
            "User referenced low memory or memory saving",
        ),
        RecommendationRule::new(
            "rule-fast-type",
            "Fast Type",
            r"(?i)(fast|performance|speed|responsive)",
            RecommendationType::Performance,
            "Enable Performance Mode",
            "Performance-focused users benefit from aggressive caching and background processing.",
            0.60,
            Some("performance_mode"),
            Some("true"),
            "User referenced speed or performance",
        ),

        // ─── Workflow Rules ───────────────────────────────────────────────
        RecommendationRule::new(
            "rule-automated-testing",
            "Automated Testing",
            r"(?i)(test|testing|pytest|cargo.test|jest|mocha)",
            RecommendationType::Workflow,
            "Enable Test Runner Integration",
            "Testing workflows benefit from integrated test runners with inline results.",
            0.75,
            Some("test_runner_integration"),
            Some("true"),
            "User referenced testing or test commands",
        ),
        RecommendationRule::new(
            "rule-ci-cd",
            "CI/CD",
            r"(?i)(ci/cd|continuous|integration|deployment|pipeline)",
            RecommendationType::Workflow,
            "Enable CI/CD Integration",
            "CI/CD workflows benefit from pipeline visualization and status tracking.",
            0.70,
            Some("ci_cd_integration"),
            Some("true"),
            "User referenced CI/CD or deployment",
        ),
        RecommendationRule::new(
            "rule-debug-mode",
            "Debug Mode",
            r"(?i)(debug|debugging|breakpoint|trace)",
            RecommendationType::Workflow,
            "Enable Debug Panel",
            "Debugging workflows benefit from an integrated debug panel with variable inspection.",
            0.80,
            Some("debug_panel"),
            Some("true"),
            "User referenced debugging or breakpoints",
        ),

        // ─── Language Rules ───────────────────────────────────────────────
        RecommendationRule::new(
            "rule-rust-lang",
            "Rust Language",
            r"(?i)(rust|cargo|clippy|rust.analyzer)",
            RecommendationType::Language,
            "Enable Rust Toolchain",
            "Rust projects benefit from cargo integration, clippy linting, and rust-analyzer.",
            0.85,
            Some("language_rust"),
            Some("true"),
            "User referenced Rust or cargo",
        ),
        RecommendationRule::new(
            "rule-python-lang",
            "Python Language",
            r"(?i)(python|pip|pyproject|black|flake8|mypy)",
            RecommendationType::Language,
            "Enable Python Toolchain",
            "Python projects benefit from pip integration, formatting, and type checking.",
            0.85,
            Some("language_python"),
            Some("true"),
            "User referenced Python or pip",
        ),
        RecommendationRule::new(
            "rule-typescript-lang",
            "TypeScript Language",
            r"(?i)(typescript|ts.node|eslint|prettier)",
            RecommendationType::Language,
            "Enable TypeScript Toolchain",
            "TypeScript projects benefit from ts-node integration, ESLint, and Prettier.",
            0.85,
            Some("language_typescript"),
            Some("true"),
            "User referenced TypeScript or ts-node",
        ),
        RecommendationRule::new(
            "rule-go-lang",
            "Go Language",
            r"(?i)(go\s+lang|golang|go.mod|gofmt|golangci)",
            RecommendationType::Language,
            "Enable Go Toolchain",
            "Go projects benefit from go.mod integration, gofmt formatting, and golangci-lint.",
            0.85,
            Some("language_go"),
            Some("true"),
            "User referenced Go or golang",
        ),

        // ─── Editor Rules ─────────────────────────────────────────────────
        RecommendationRule::new(
            "rule-word-wrap",
            "Word Wrap",
            r"(?i)(word.?wrap|soft.?wrap|long.lines)",
            RecommendationType::Editor,
            "Enable Word Wrap",
            "Word wrap improves readability for long lines and narrow terminals.",
            0.60,
            Some("editor_word_wrap"),
            Some("true"),
            "User referenced word wrap or long lines",
        ),
        RecommendationRule::new(
            "rule-tab-size",
            "Tab Size",
            r"(?i)(tab|indent|spaces|whitespace)",
            RecommendationType::Editor,
            "Configure Tab Settings",
            "Tab settings should match project conventions (tabs vs spaces).",
            0.55,
            Some("editor_tab_size"),
            Some("4"),
            "User referenced tabs, indent, or whitespace",
        ),
        RecommendationRule::new(
            "rule-font-size",
            "Font Size",
            r"(?i)(font.size|text.size|zoom|scale)",
            RecommendationType::Editor,
            "Adjust Font Size",
            "Font size should be comfortable for extended coding sessions.",
            0.50,
            Some("editor_font_size"),
            Some("14"),
            "User referenced font size or zoom",
        ),

        // ─── Notification Rules ───────────────────────────────────────────
        RecommendationRule::new(
            "rule-silent-mode",
            "Silent Mode",
            r"(?i)(silent|quiet|do.not.disturb|focus)",
            RecommendationType::Notification,
            "Enable Silent Mode",
            "Silent mode should suppress all non-critical notifications.",
            0.75,
            Some("notifications_silent"),
            Some("true"),
            "User requested silent or focus mode",
        ),
        RecommendationRule::new(
            "rule-busy-indicator",
            "Busy Indicator",
            r"(?i)(busy|loading|processing|working)",
            RecommendationType::Notification,
            "Enable Busy Indicator",
            "Busy states benefit from clear visual indicators of ongoing operations.",
            0.65,
            Some("notifications_busy"),
            Some("true"),
            "User referenced busy or loading states",
        ),

        // ─── General Heuristic Rules ──────────────────────────────────────
        RecommendationRule::new(
            "rule-new-user",
            "New User",
            r"(?i)(new|first.?time|getting.started|onboarding)",
            RecommendationType::General,
            "Enable Onboarding Tour",
            "New users benefit from an interactive onboarding tour covering key features.",
            0.60,
            None,
            None,
            "User indicated they are new or starting fresh",
        ),
        RecommendationRule::new(
            "rule-productivity",
            "Productivity",
            r"(?i)(productivity|efficiency|workflow.automation|time.saving)",
            RecommendationType::General,
            "Enable Productivity Tools",
            "Productivity-focused users benefit from shortcuts, snippets, and automation.",
            0.55,
            Some("productivity_tools"),
            Some("true"),
            "User referenced productivity or efficiency",
        ),
        RecommendationRule::new(
            "rule-accessibility",
            "Accessibility",
            r"(?i)(accessibility|a11y|screen.reader|keyboard.navigation)",
            RecommendationType::General,
            "Enable Accessibility Features",
            "Accessibility-focused users benefit from screen reader support and keyboard navigation.",
            0.80,
            Some("accessibility_enabled"),
            Some("true"),
            "User referenced accessibility or a11y",
        ),
    ]
}

/// Static storage for all rules to avoid temporary value issues.
static ALL_RULES: std::sync::LazyLock<Vec<RecommendationRule>> =
    std::sync::LazyLock::new(all_rules);

/// Find all rules that match the given input.
pub fn find_matching_rules(
    input: &str,
    context: &RecommendationContext,
) -> Vec<&'static RecommendationRule> {
    ALL_RULES
        .iter()
        .filter(|rule| rule.matches(input))
        .collect()
}

/// Generate recommendations from matching rules.
pub fn generate_from_rules(
    input: &str,
    related_intent_id: &str,
    context: &RecommendationContext,
) -> Vec<Recommendation> {
    let matching = find_matching_rules(input, context);
    matching
        .into_iter()
        .filter_map(|rule| rule.generate(input, related_intent_id, context))
        .filter(|rec| rec.confidence.score() >= context.min_confidence)
        .collect()
}

/// Generate recommendations based on command kinds in the plan.
pub fn generate_from_commands(
    command: &crate::intent_engine::IntentCommand,
    intent_id: &str,
    context: &RecommendationContext,
) -> Vec<Recommendation> {
    let mut recs = Vec::new();

    match command {
        crate::intent_engine::IntentCommand::ExecuteCommand { command, reason } => {
            let cmd_lower = command.to_lowercase();
            if cmd_lower.contains("cargo") || cmd_lower.contains("rustup") {
                recs.push(Recommendation::new(
                    RecommendationType::Integration,
                    "Enable Rust Toolchain",
                    "Cargo commands detected — consider enabling Rust-specific tooling.",
                    vec!["Detected cargo command execution".to_string()],
                    RecommendationConfidence::High(0.85),
                    "rule-cargo-detect",
                    Some("language_rust".to_string()),
                    Some("true".to_string()),
                    intent_id,
                ));
            }
            if cmd_lower.contains("python") || cmd_lower.contains("pip") {
                recs.push(Recommendation::new(
                    RecommendationType::Integration,
                    "Enable Python Toolchain",
                    "Python commands detected — consider enabling Python-specific tooling.",
                    vec!["Detected python/pip command execution".to_string()],
                    RecommendationConfidence::High(0.85),
                    "rule-python-detect",
                    Some("language_python".to_string()),
                    Some("true".to_string()),
                    intent_id,
                ));
            }
            if cmd_lower.contains("test") {
                recs.push(Recommendation::new(
                    RecommendationType::Workflow,
                    "Enable Test Runner",
                    "Test commands detected — consider enabling integrated test runner.",
                    vec!["Detected test command execution".to_string()],
                    RecommendationConfidence::Medium(0.75),
                    "rule-test-detect",
                    Some("test_runner_integration".to_string()),
                    Some("true".to_string()),
                    intent_id,
                ));
            }
        }
        crate::intent_engine::IntentCommand::UpdateModelPreference { new_value, .. } => {
            let val_lower = new_value.to_lowercase();
            if val_lower.contains("claude") || val_lower.contains("anthropic") {
                recs.push(Recommendation::new(
                    RecommendationType::General,
                    "Enable Claude-Specific Settings",
                    "Claude model detected — consider enabling Claude-optimized settings.",
                    vec!["Model preference set to Claude".to_string()],
                    RecommendationConfidence::Medium(0.70),
                    "rule-claude-detect",
                    Some("model_claude_optimized".to_string()),
                    Some("true".to_string()),
                    intent_id,
                ));
            }
            if val_lower.contains("gpt") || val_lower.contains("openai") {
                recs.push(Recommendation::new(
                    RecommendationType::General,
                    "Enable GPT-Specific Settings",
                    "GPT model detected — consider enabling GPT-optimized settings.",
                    vec!["Model preference set to GPT".to_string()],
                    RecommendationConfidence::Medium(0.70),
                    "rule-gpt-detect",
                    Some("model_gpt_optimized".to_string()),
                    Some("true".to_string()),
                    intent_id,
                ));
            }
        }
        crate::intent_engine::IntentCommand::UpdateApprovalPreference { new_value, .. } => {
            if *new_value {
                recs.push(Recommendation::new(
                    RecommendationType::General,
                    "Enable Approval Automation",
                    "Auto-approve enabled — consider setting confidence thresholds for automatic approvals.",
                    vec!["Approval preference set to true".to_string()],
                    RecommendationConfidence::Medium(0.65),
                    "rule-auto-approve",
                    Some("auto_approve_threshold".to_string()),
                    Some("0.8".to_string()),
                    intent_id,
                ));
            }
        }
        _ => {}
    }

    recs
}

/// Generate recommendations based on intent type.
pub fn generate_from_intent_type(
    intent_type: &crate::intent_engine::IntentType,
    intent_id: &str,
    context: &RecommendationContext,
) -> Vec<Recommendation> {
    let mut recs = Vec::new();

    match intent_type {
        crate::intent_engine::IntentType::Preference => {
            recs.push(Recommendation::new(
                RecommendationType::General,
                "Review Preference Impact",
                "Preference changes affect the entire session — consider reviewing all pending changes before approval.",
                vec!["Preference intent detected".to_string()],
                RecommendationConfidence::Low(0.50),
                "rule-preference-review",
                None,
                None,
                intent_id,
            ));
        }
        crate::intent_engine::IntentType::Execution => {
            recs.push(Recommendation::new(
                RecommendationType::General,
                "Execution Preview",
                "Execution intents may have side effects — review the command before approval.",
                vec!["Execution intent detected".to_string()],
                RecommendationConfidence::Low(0.55),
                "rule-execution-review",
                None,
                None,
                intent_id,
            ));
        }
        crate::intent_engine::IntentType::Workflow => {
            recs.push(Recommendation::new(
                RecommendationType::Workflow,
                "Workflow Safety Check",
                "Workflow execution may affect multiple files — consider enabling dry-run mode.",
                vec!["Workflow intent detected".to_string()],
                RecommendationConfidence::Medium(0.65),
                "rule-workflow-safety",
                Some("workflow_dry_run".to_string()),
                Some("true".to_string()),
                intent_id,
            ));
        }
        crate::intent_engine::IntentType::Question => {
            recs.push(Recommendation::new(
                RecommendationType::General,
                "Related Documentation",
                "Questions may benefit from related documentation links.",
                vec!["Question intent detected".to_string()],
                RecommendationConfidence::Low(0.45),
                "rule-question-docs",
                None,
                None,
                intent_id,
            ));
        }
        crate::intent_engine::IntentType::Help => {
            recs.push(Recommendation::new(
                RecommendationType::General,
                "Quick Reference",
                "Help requests may benefit from a quick reference card.",
                vec!["Help intent detected".to_string()],
                RecommendationConfidence::Low(0.40),
                "rule-help-reference",
                None,
                None,
                intent_id,
            ));
        }
        _ => {}
    }

    recs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dark_theme_rule_matches() {
        let rules = all_rules();
        let dark_rule = rules.iter().find(|r| r.id == "rule-dark-theme");
        assert!(dark_rule.is_some(), "Dark theme rule should exist");
        let rule = dark_rule.unwrap();
        assert!(
            rule.matches("Enable dark theme"),
            "Rule should match 'Enable dark theme'"
        );
        assert!(rule.matches("dark theme"), "Rule should match 'dark theme'");
    }

    #[test]
    fn test_vim_rule_matches() {
        let rules = all_rules();
        let vim_rule = rules.iter().find(|r| r.id == "rule-vim-mode");
        assert!(vim_rule.is_some(), "Vim mode rule should exist");
        let rule = vim_rule.unwrap();
        assert!(
            rule.matches("Enable vim mode"),
            "Rule should match 'Enable vim mode'"
        );
        assert!(rule.matches("use vim"), "Rule should match 'use vim'");
    }

    #[test]
    fn test_generate_from_rules_dark_theme() {
        let context = RecommendationContext::new();
        let recs = generate_from_rules("Enable dark theme", "plan-1", &context);
        assert!(
            !recs.is_empty(),
            "Should generate recommendations for dark theme"
        );
    }

    #[test]
    fn test_generate_from_rules_vim() {
        let context = RecommendationContext::new();
        let recs = generate_from_rules("Enable vim mode", "plan-1", &context);
        assert!(
            !recs.is_empty(),
            "Should generate recommendations for vim mode"
        );
    }
}
