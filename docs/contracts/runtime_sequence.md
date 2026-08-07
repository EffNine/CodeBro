# Runtime Sequence Diagrams

**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-05
**Owner:** CodeBro Engineering

---

## 1. Tool Registration Sequence

```
┌─────────────┐     ┌──────────────────┐     ┌─────────────────┐     ┌──────────────────┐
│   Main      │     │  ToolRegistry    │     │  ToolProvider   │     │   Tool (T)      │
└──────┬──────┘     └────────┬─────────┘     └────────┬────────┘     └────────┬─────────┘
       │                     │                        │                        │
       │  new()              │                        │                        │
       ├────────────────────>│                        │                        │
       │                     │── HashMap::new() ────>│                        │
       │                     │                        │                        │
       │  register(T)        │                        │                        │
       ├────────────────────>│                        │                        │
       │                     │  T.name()              │                        │
       │                     │<───────────────────────┼────────────────────────│
       │                     │  "read_file"           │                        │
       │                     │                        │                        │
       │                     │  T.description()       │                        │
       │                     │<───────────────────────┼────────────────────────│
       │                     │  "Read the contents..."│                        │
       │                     │                        │                        │
       │                     │  insert("read_file", T)│                        │
       │                     │  insert(metadata)      │                        │
       │                     │  lifecycle.register()  │                        │
       │                     │                        │                        │
       │  <Ok>               │                        │                        │
       │<────────────────────┤                        │                        │
```

---

## 2. Tool Discovery Sequence

```
┌─────────────┐     ┌──────────────────┐     ┌─────────────────┐
│  Main       │     │  ToolDiscovery   │     │  Provider (P)   │
└──────┬──────┘     └────────┬─────────┘     └────────┬────────┘
       │                     │                        │
       │  new()              │                        │
       ├────────────────────>│                        │
       │                     │                        │
       │  add_provider(P)    │                        │
       ├────────────────────>│                        │
       │                     │                        │
       │  discover()         │                        │
       ├────────────────────>│                        │
       │                     │  P.is_available()      │
       │                     │<───────────────────────┤
       │                     │  true                  │
       │                     │                        │
       │                     │  P.discover_tools()    │
       │                     │<───────────────────────┤
       │                     │  [ToolDef1, ToolDef2]  │
       │                     │                        │
       │  <DiscoveryResult>  │                        │
       │<────────────────────┤                        │
```

---

## 3. Tool Execution Sequence (with hooks and diagnostics)

```
┌──────────┐  ┌──────────────────┐  ┌─────────────┐  ┌────────────────┐  ┌─────────┐
│  Caller  │  │  ToolRegistry    │  │  HookManager│  │  DiagnosticCollector │  │  Tool  │
└────┬─────┘  └────────┬─────────┘  └──────┬──────┘  └────────┬─────────┘  └────┬────┘
     │                 │                   │                   │                 │
     │  execute(name,  │                   │                   │                 │
     │   args)         │                   │                   │                 │
     ├────────────────>│                   │                   │                 │
     │                 │                   │                   │                 │
     │                 │  is_active(name)? │                   │                 │
     │                 │<──────────────────┤                   │                 │
     │                 │  true             │                   │                 │
     │                 │                   │                   │                 │
     │                 │  check_permission │                   │                 │
     │                 │<──────────────────┤                   │                 │
     │                 │  Allowed          │                   │                 │
     │                 │                   │                   │                 │
     │                 │  before_execute() │                   │                 │
     │                 │<──────────────────┤                   │                 │
     │                 │  Ok               │                   │                 │
     │                 │                   │                   │                 │
     │                 │  tool.execute()   │                   │                 │
     │                 │───────────────────────────────────────>│                 │
     │                 │  Ok("output")     │                   │                 │
     │                 │<───────────────────────────────────────│                 │
     │                 │                   │                   │                 │
     │                 │  record_success() │                   │                 │
     │                 │────────────────────────────────────────────────────────>│
     │                 │                   │                   │                 │
     │                 │  after_execute()  │                   │                 │
     │                 │<──────────────────┤                   │                 │
     │                 │  Ok               │                   │                 │
     │                 │                   │                   │                 │
     │  <Ok("output")> │                   │                   │                 │
     │<────────────────┤                   │                   │                 │
```

---

## 4. Streaming Execution Sequence

```
┌──────────┐  ┌──────────────────┐  ┌─────────────────┐
│  Caller  │  │  AsyncTool       │  │  Output Source  │
└────┬─────┘  └────────┬─────────┘  └────────┬────────┘
     │                 │                     │
     │  execute_stream│                     │
     │  (args, ctx)    │                     │
     ├────────────────>│                     │
     │                 │                     │
     │                 │  [spawn producer]   │
     │                 │────────────────────>│
     │                 │                     │
     │                 │  StreamResult       │
     │                 │  (with channel)     │
     │                 │<────────────────────┤
     │                 │                     │
     │  <StreamResult> │                     │
     │<────────────────┤                     │
     │                 │                     │
     │  collect()      │                     │
     ├────────────────>│                     │
     │                 │  next()             │
     │                 │<────────────────────┤
     │  Some(chunk1)   │                     │
     │<────────────────┤                     │
     │                 │  ...                │
     │  Some(chunkN)   │                     │
     │<────────────────┤                     │
     │                 │  None               │
     │<────────────────┤                     │
     │                 │                     │
     │  <Ok("output")> │                     │
     │<────────────────┤                     │
```

---

## 5. Lifecycle Transition Sequence

```
┌─────────────┐  ┌──────────────────┐
│  Manager    │  │  ToolLifecycle   │
└──────┬──────┘  └────────┬─────────┘
       │                  │
       │  register(name)  │
       ├─────────────────>│
       │                  │  transition(Unregistered → Registered)
       │                  │<─ Ok(Registered) ─
       │  <Ok>           │
       │<─────────────────┤
       │                  │
       │  enable(name)    │
       ├─────────────────>│
       │                  │  transition(Registered → Enabled)
       │                  │<─ Ok(Enabled) ─
       │  <Ok>           │
       │<─────────────────┤
       │                  │
       │  disable(name)   │
       ├─────────────────>│
       │                  │  transition(Enabled → Disabled)
       │                  │<─ Ok(Disabled) ─
       │  <Ok>           │
       │<─────────────────┤
```

---

## 6. Permission Check Sequence

```
┌──────────┐  ┌──────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│ Registry │  │  HookManager     │  │  ToolHooks      │  │  PermissionHook │
└────┬─────┘  └────────┬─────────┘  └────────┬────────┘  └────────┬────────┘
     │                 │                     │                     │
     │  check_perm(ctx)│                     │                     │
     ├────────────────>│                     │                     │
     │                 │  get_tool_hooks()   │                     │
     │                 │<────────────────────┤                     │
     │                 │  ToolHooks { perm } │                     │
     │                 │                     │                     │
     │                 │  hooks.check_perm() │                     │
     │                 │────────────────────>│                     │
     │                 │                     │  hook.check(ctx)    │
     │                 │                     │────────────────────>│
     │                 │                     │                     │  Decision
     │                 │                     │<────────────────────┤
     │                 │  Allowed/Ask/Denied │                     │
     │                 │<────────────────────┤                     │
     │  Decision       │                     │                     │
     │<────────────────┤                     │                     │
```

---

## 7. Diagnostic Recording Sequence

```
┌──────────┐  ┌──────────────────┐  ┌─────────────────────┐
│ Registry │  │  DiagnosticMgr   │  │  ToolDiagnostics    │
└────┬─────┘  └────────┬─────────┘  └─────────────────────┘
     │                 │
     │  record_success │
     │  (name, dur,    │
     │   exec_id, code)│
     ├────────────────>│
     │                 │  get_or_create(name)
     │                 │<──────────────────────┐
     │                 │  ToolDiagnostics      │
     │                 │──────────────────────>│
     │                 │                       │
     │                 │  diag.record_success()│
     │                 │<──────────────────────┤
     │                 │                       │
     │  <Ok>          │                       │
     │<────────────────┤                       │
```
