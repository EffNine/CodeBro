# Plugin SDK Architecture Report — P9.3

**Date:** 2026-08-06
**Version:** CodeBro v1.0.0
**Phase:** P9.3 Plugin SDK Foundation

## Executive Summary

The Plugin SDK Foundation has been implemented as `src/plugin_sdk/`. It provides the ONLY approved extension mechanism for CodeBro. Core engines remain stable; plugins extend capabilities through declared interfaces.

## Architecture

```
src/plugin_sdk/
├── mod.rs          — Module root, public re-exports
├── types.rs        — Core types: PluginId, Manifest, Permission, HookPhase, etc.
├── plugin.rs       — Plugin trait, PluginState, PluginError, NoOpPlugin
├── registry.rs     — PluginRegistry: discover, validate, register, dependency resolution
├── loader.rs       — PluginLoader: discover from paths, validate manifests
├── lifecycle.rs    — PluginLifecycle: deterministic state machine
├── capabilities.rs — CapabilityModel: declare, check, enforce plugin capabilities
├── hooks.rs        — HookDispatcher: register, dispatch, order plugin hooks
├── sandbox.rs      — PluginSandbox: isolation, permission checks, violation tracking
└── diagnostics.rs  — PluginDiagnostics: health, metrics, audit
```

## Module Responsibilities

| Module | Responsibility |
|--------|---------------|
| `types` | All core data types: `PluginId`, `PluginManifest`, `Permission`, `SecurityDomain`, `HookPhase`, `PluginVersion`, `RequiredSdkVersion` |
| `plugin` | `Plugin` trait (init, on_hook, shutdown, clone_box), `PluginState` state machine, `PluginError` |
| `registry` | `PluginRegistry`: plugin storage, dependency graph, topological ordering, state management |
| `loader` | `PluginLoader`: discover from directories, validate manifests, load internal plugins |
| `lifecycle` | `PluginLifecycle`: deterministic run_lifecycle (discover→validate→load→init→register→run→shutdown) |
| `capabilities` | `CapabilityModel`: register capabilities, check plugin capabilities, track providers |
| `hooks` | `HookDispatcher`: register hooks, dispatch by phase, order by priority, check blockers |
| `sandbox` | `PluginSandbox`: policy enforcement, violation tracking, security domain checks |
| `diagnostics` | `PluginDiagnostics`: plugin health tracking, error recording, summary reporting |

## Plugin Lifecycle

```
Discover → Validate → Load → Init → Register → Run → Shutdown
```

1. **Discover**: `PluginLoader` scans search paths for plugin manifests (plugin.json / plugin.toml)
2. **Validate**: Manifest validation (non-empty name/description, valid version)
3. **Load**: Register plugin in `PluginRegistry`, set state to `Loaded`
4. **Init**: Call `plugin.init()` for each plugin in dependency order
5. **Register**: Set state to `Active`, plugin is now operational
6. **Run**: Hooks dispatch on pipeline events via `HookDispatcher`
7. **Shutdown**: Reverse-order shutdown, release all resources

## Security Model

- **No direct memory access**: Plugins cannot modify core memory
- **Approval gate enforcement**: `Sandbox` blocks approval bypass attempts
- **Validation enforcement**: `Sandbox` blocks validation bypass attempts
- **Deterministic behavior**: `Sandbox` blocks changes to deterministic behavior
- **Permission domains**: Each plugin declares which `SecurityDomain`s it needs
- **Capability model**: Plugins can only use declared capabilities

## Compatibility

The SDK supports future extension without redesign:
- **Marketplace plugins**: `PluginSource::Remote(url)`
- **Remote plugins**: Download + validate + load
- **Enterprise plugins**: Custom `SecurityDomain` + policy
- **AI plugins**: `PluginSource::AiGenerated` + enhanced validation
- **Internal plugins**: `PluginSource::Internal` with compile-time guarantees

## Test Coverage

61 new tests covering all modules:
- types: 11 tests
- plugin: 11 tests
- registry: 11 tests
- loader: 6 tests
- lifecycle: 5 tests
- capabilities: 6 tests
- hooks: 6 tests
- sandbox: 6 tests
- diagnostics: 5 tests

All 1,553 tests pass. Zero regressions.
