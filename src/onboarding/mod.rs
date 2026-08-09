//! Onboarding Module
//!
//! Provides a guided first-run experience that requires only the user's API
//! key to get CodeBro working. All other configuration is auto-detected and
//! presented for approval.

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::capability_discovery::{CapabilityDiscovery, CapabilityScanner};
use crate::provider_manager::{ProviderId, WizardState};
use crate::workspace_discovery::{DiscoveryEngine, WorkspaceDiscovery};

// ─── Onboarding State ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum OnboardingStep {
    /// Check if this is a first run
    CheckConfig,
    /// Welcome screen
    Welcome,
    /// Enter API key
    EnterApiKey,
    /// Select provider
    SelectProvider,
    /// Auto-detect model
    DetectModel,
    /// Discover workspace
    DiscoverWorkspace,
    /// Review integrations
    ReviewIntegrations,
    /// Review capabilities
    ReviewCapabilities,
    /// Final confirmation
    Confirm,
    /// Complete
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingResult {
    pub provider_id: ProviderId,
    pub base_url: String,
    pub model: String,
    pub workspace_root: PathBuf,
    pub workspace_discovery: WorkspaceDiscovery,
    pub capability_discovery: CapabilityDiscovery,
    pub integrations_enabled: Vec<String>,
    pub completed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct OnboardingSession {
    pub step: OnboardingStep,
    pub wizard_state: WizardState,
    pub workspace_discovery: Option<WorkspaceDiscovery>,
    pub capability_discovery: Option<CapabilityDiscovery>,
    pub api_key: Option<String>,
    pub result: Option<OnboardingResult>,
    pub error: Option<String>,
}

impl OnboardingSession {
    pub fn new() -> Self {
        OnboardingSession {
            step: OnboardingStep::CheckConfig,
            wizard_state: WizardState::new(),
            workspace_discovery: None,
            capability_discovery: None,
            api_key: None,
            result: None,
            error: None,
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.step, OnboardingStep::Complete)
    }

    pub fn is_first_run(&self) -> bool {
        matches!(self.step, OnboardingStep::CheckConfig)
    }
}

impl Default for OnboardingSession {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Onboarding Manager ──────────────────────────────────────────────────────

pub struct OnboardingManager {
    pub config_dir: PathBuf,
    pub session: OnboardingSession,
}

impl OnboardingManager {
    pub fn new(config_dir: PathBuf) -> Self {
        OnboardingManager {
            config_dir,
            session: OnboardingSession::new(),
        }
    }

    /// Check if this is a first run (no config exists)
    pub fn check_first_run(&self) -> bool {
        let config_path = self.config_dir.join("config.toml");
        !config_path.exists()
    }

    /// Start the onboarding flow
    pub fn start(&mut self) {
        self.session.step = OnboardingStep::Welcome;
    }

    /// Advance to the next step
    pub fn next(&mut self) {
        self.session.step = match self.session.step {
            OnboardingStep::Welcome => OnboardingStep::EnterApiKey,
            OnboardingStep::EnterApiKey => OnboardingStep::SelectProvider,
            OnboardingStep::SelectProvider => OnboardingStep::DetectModel,
            OnboardingStep::DetectModel => OnboardingStep::DiscoverWorkspace,
            OnboardingStep::DiscoverWorkspace => OnboardingStep::ReviewIntegrations,
            OnboardingStep::ReviewIntegrations => OnboardingStep::ReviewCapabilities,
            OnboardingStep::ReviewCapabilities => OnboardingStep::Confirm,
            OnboardingStep::Confirm => OnboardingStep::Complete,
            _ => OnboardingStep::Complete,
        };
    }

    /// Go back to the previous step
    pub fn previous(&mut self) {
        self.session.step = match self.session.step {
            OnboardingStep::EnterApiKey => OnboardingStep::Welcome,
            OnboardingStep::SelectProvider => OnboardingStep::EnterApiKey,
            OnboardingStep::DetectModel => OnboardingStep::SelectProvider,
            OnboardingStep::DiscoverWorkspace => OnboardingStep::DetectModel,
            OnboardingStep::ReviewIntegrations => OnboardingStep::DiscoverWorkspace,
            OnboardingStep::ReviewCapabilities => OnboardingStep::ReviewIntegrations,
            OnboardingStep::Confirm => OnboardingStep::ReviewCapabilities,
            OnboardingStep::Complete => OnboardingStep::Confirm,
            _ => OnboardingStep::Welcome,
        };
    }

    /// Set the API key
    pub fn set_api_key(&mut self, key: &str) {
        self.session.api_key = Some(key.to_string());
        self.session.wizard_state.set_api_key(key);
    }

    /// Select a provider
    pub fn select_provider(&mut self, provider: &ProviderId) {
        self.session.wizard_state.select_provider(provider);
    }

    /// Discover the workspace
    pub async fn discover_workspace(&mut self, root: &PathBuf) {
        let engine = DiscoveryEngine::new(root.clone());
        let discovery = engine.discover();
        self.session.workspace_discovery = Some(discovery);
    }

    /// Discover capabilities
    pub async fn discover_capabilities(&mut self, root: &PathBuf) {
        let scanner = CapabilityScanner::new(root.clone());
        let discovery = scanner.scan();
        self.session.capability_discovery = Some(discovery);
    }

    /// Complete the onboarding and persist config
    pub async fn complete(&mut self, workspace_root: &PathBuf) -> Result<OnboardingResult> {
        let provider_id = self
            .session
            .wizard_state
            .selected_provider
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No provider selected"))?;

        let model = self
            .session
            .wizard_state
            .selected_model
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No model selected"))?;

        let api_key = self
            .session
            .api_key
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No API key provided"))?;

        let workspace_discovery = self
            .session
            .workspace_discovery
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Workspace not discovered"))?;

        let capability_discovery = self
            .session
            .capability_discovery
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Capabilities not discovered"))?;

        let integrations_enabled: Vec<String> = workspace_discovery
            .proposals
            .iter()
            .filter(|p| p.enabled)
            .map(|p| p.name.clone())
            .collect();

        // Persist the configuration
        self.persist_config(&provider_id, &model, &api_key)?;

        let result = OnboardingResult {
            provider_id: provider_id.clone(),
            base_url: provider_id.default_base_url(),
            model,
            workspace_root: workspace_root.clone(),
            workspace_discovery,
            capability_discovery,
            integrations_enabled,
            completed_at: chrono::Utc::now(),
        };

        self.session.result = Some(result.clone());
        self.session.step = OnboardingStep::Complete;

        Ok(result)
    }

    fn persist_config(&self, provider_id: &ProviderId, model: &str, api_key: &str) -> Result<()> {
        use std::fs;

        // Write the main config
        let config_content = format!(
            "# CodeBro Configuration\n\
             # Generated by onboarding wizard\n\n\
             provider = \"{}\"\n\
             base_url = \"{}\"\n\
             model = \"{}\"\n",
            provider_id.as_str(),
            provider_id.default_base_url(),
            model
        );

        let config_path = self.config_dir.join("config.toml");
        fs::create_dir_all(&self.config_dir)?;
        fs::write(&config_path, config_content)?;

        // Store API key in the secure credential store (never plaintext JSON).
        let mut store = crate::credentials::CredentialStore::new(self.config_dir.clone());
        store.set(provider_id.as_str(), api_key)?;

        // Migrate: remove the legacy plaintext key file if it exists.
        let legacy_key_path = self.config_dir.join(".api_key");
        if legacy_key_path.exists() {
            let _ = std::fs::remove_file(&legacy_key_path);
        }

        Ok(())
    }

    /// Get the current step display info
    pub fn step_info(&self) -> (&'static str, &'static str) {
        match self.session.step {
            OnboardingStep::CheckConfig => {
                ("Check Config", "Checking for existing configuration...")
            }
            OnboardingStep::Welcome => ("Welcome", "Welcome to CodeBro! Let's get you set up."),
            OnboardingStep::EnterApiKey => ("API Key", "Enter your provider API key."),
            OnboardingStep::SelectProvider => ("Provider", "Select your AI provider."),
            OnboardingStep::DetectModel => ("Model", "Detecting available models..."),
            OnboardingStep::DiscoverWorkspace => ("Workspace", "Discovering your workspace..."),
            OnboardingStep::ReviewIntegrations => {
                ("Integrations", "Review workspace integrations.")
            }
            OnboardingStep::ReviewCapabilities => {
                ("Capabilities", "Review available capabilities.")
            }
            OnboardingStep::Confirm => ("Confirm", "Review and confirm your settings."),
            OnboardingStep::Complete => ("Complete", "Onboarding complete! Welcome to CodeBro."),
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_onboarding_flow() {
        let mut manager = OnboardingManager::new(PathBuf::from("/tmp/test_configbro"));
        manager.start();
        assert_eq!(manager.session.step, OnboardingStep::Welcome);

        manager.next();
        assert_eq!(manager.session.step, OnboardingStep::EnterApiKey);

        manager.next();
        assert_eq!(manager.session.step, OnboardingStep::SelectProvider);

        manager.previous();
        assert_eq!(manager.session.step, OnboardingStep::EnterApiKey);
    }

    #[test]
    fn test_step_info() {
        let manager = OnboardingManager::new(PathBuf::from("/tmp"));
        let (title, desc) = manager.step_info();
        assert!(!title.is_empty());
        assert!(!desc.is_empty());
    }

    #[test]
    fn test_is_complete() {
        let mut manager = OnboardingManager::new(PathBuf::from("/tmp"));
        assert!(!manager.session.is_complete());
        manager.session.step = OnboardingStep::Complete;
        assert!(manager.session.is_complete());
    }
}
