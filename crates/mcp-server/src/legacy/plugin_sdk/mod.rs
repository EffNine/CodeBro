#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Plugin SDK Foundation for CodeBro.
//!
//! The ONLY approved extension mechanism. Plugins extend capabilities without
//! modifying core engines. Core remains stable; plugins are additive.
//!
//! # Architecture
//!
//! ```text
//! Plugin SDK
//!   ├─ types        — PluginId, Manifest, Capability, Hook, Permission
//!   ├─ plugin       — Plugin trait, PluginState, PluginError
//!   ├─ registry     — PluginRegistry: discover, validate, register
//!   ├─ loader       — PluginLoader: load from sources
//!   ├─ lifecycle    — PluginLifecycle: discover → validate → load → init → run → shutdown
//!   ├─ capabilities — CapabilityModel: declare, check, enforce
//!   ├─ hooks        — HookSystem: register, dispatch, order
//!   ├─ sandbox      — PluginSandbox: isolation, permission checks
//!   └─ diagnostics  — PluginDiagnostics: health, metrics, audit
//! ```
//!
//! # Design Rules
//!
//! - **Core is stable**: No plugin can modify core memory directly.
//! - **Plugins are isolated**: All interactions go through approved SDK interfaces.
//! - **Lifecycle is deterministic**: discover → validate → load → init → register → run → shutdown.
//! - **Capabilities are declared**: Every plugin declares what it provides.
//! - **Security first**: Permissions, sandboxing, and approval gate enforcement.
//!
//! # Thread Safety
//!
//! All public types implement `Send + Sync + Clone`.

pub mod capabilities;
pub mod diagnostics;
pub mod hooks;
pub mod lifecycle;
pub mod loader;
pub mod plugin;
pub mod registry;
pub mod sandbox;
pub mod types;

pub use capabilities::{Capability, CapabilityModel};
pub use diagnostics::PluginDiagnostics;
pub use hooks::{Hook, HookDispatcher, HookOrder};
pub use lifecycle::PluginLifecycle;
pub use loader::PluginLoader;
pub use plugin::{Plugin, PluginError, PluginState};
pub use registry::PluginRegistry;
pub use sandbox::PluginSandbox;
pub use types::{
    Author, CodeBroVersion, HookPhase, License, Permission, PermissionLevel, PluginId,
    PluginManifest, PluginSource, PluginVersion, RequiredSdkVersion, SecurityDomain,
    SupportedVersionRange,
};
