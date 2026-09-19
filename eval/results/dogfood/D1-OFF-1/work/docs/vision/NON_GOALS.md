# Non-Goals

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## Purpose

This document defines what CodeBro will never become. It exists to prevent scope drift and to give contributors a clear reference when evaluating feature requests. If a request aligns with any item below, the answer is **no** — unless it can be expressed as a Skill, MCP server, or Plugin.

These are not limitations born of impossibility. They are constraints chosen deliberately to protect CodeBro's identity as a focused, trustworthy engineering intelligence runtime.

---

## General Chatbot

**CodeBro will never be a general-purpose chatbot.**

Chatbots optimize for engagement, variety, and friendly conversation. CodeBro optimizes for correctness, traceability, and engineering efficiency. A chatbot asks "How can I help you?" CodeBro asks "What needs to change in the code?"

The difference is not surface-level. It is architectural. A chatbot's context window is the conversation. CodeBro's context window is the codebase.

---

## Personal Assistant

**CodeBro will never be a personal assistant.**

Calendar management, email, reminders, scheduling — these are personal assistant functions. They belong in tools designed for that purpose. CodeBro's purpose is engineering. Adding personal assistant capabilities would dilute the product and compete with tools that do that job better.

---

## Calendar or Scheduling Tool

**CodeBro will never manage calendars or schedules.**

Scheduling is a human decision supported by specialized tools. CodeBro does not know when the developer wants to meet, ship, or review. It knows when the tests pass and when the build succeeds. These are different domains.

---

## Email Client

**CodeBro will never read, write, or manage email.**

Email is a communication protocol with its own clients, security models, and workflows. CodeBro has no business in the developer's inbox. If a task requires email (e.g., "notify the team"), the developer sends the email. CodeBro does not.

---

## Web Browser

**CodeBro will never be a web browser or browsing agent.**

Browsing the web is a fundamentally different activity from engineering code. It requires rendering, navigation, and interaction with arbitrary web interfaces. CodeBro's tool system is designed for deterministic, reproducible operations on the local codebase. Web browsing is none of those things.

If a task requires web research, the developer copies the relevant content into the task input. CodeBro does not open browsers.

---

## Project Management Software

**CodeBro will never manage projects, sprints, or tasks.**

Project management is about coordination across people. CodeBro is a solo tool. It tracks tasks within a single engineering session — what was planned, what was executed, what was learned — but it does not manage work across a team or a timeline.

Issues, sprints, story points, and roadmaps belong in project management tools. CodeBro's "tasks" are engineering actions, not management artifacts.

---

## IDE Replacement

**CodeBro will never replace the IDE.**

IDEs provide code editing, refactoring, navigation, debugging, and autocomplete — all in a rich graphical interface. CodeBro is not an IDE. It is a task executor that operates on the codebase the IDE manages.

CodeBro complements the IDE. It reads what the IDE writes, proposes changes the IDE can review, and runs commands the IDE triggers. But it does not replace the editor, the debugger, or the compiler explorer.

Developers use CodeBro alongside their IDE, not instead of it.

---

## Deployment Platform

**CodeBro will never deploy code to production.**

Deployment is a human decision with irreversible consequences. CodeBro can build, test, and prepare changes. It can even generate the deployment command. But it will not execute deployment without explicit, per-task approval — and even then, deployment is recommended to happen through the developer's established CI/CD pipeline, not through CodeBro.

CodeBro ships code to the developer's review. The developer ships code to the world.

---

## Cloud Dashboard

**CodeBro will never be a web-accessible dashboard or cloud service.**

CodeBro runs locally. It has no web server, no cloud backend, no multi-user authentication. It is a terminal application that lives on the developer's machine. A web dashboard would require an entirely different architecture, security model, and deployment strategy.

If the developer wants a dashboard, they build one. CodeBro is the instrument, not the platform.

---

## Social Features

**CodeBro will never include social features.**

Leaderboards, shared sessions, community chat, public profiles — these are social features. CodeBro is a personal engineering tool. It tracks the developer's own work, their own memory, their own skills. There is no multi-user dimension.

If a developer wants to share a session or a skill, they use git or a file share. CodeBro does not facilitate social interaction.

---

## Summary

| Non-Goal | Why It Is Excluded |
|----------|-------------------|
| General chatbot | Different optimization target: engagement vs. correctness |
| Personal assistant | Different domain: life management vs. engineering |
| Calendar/scheduling | Human coordination, not code execution |
| Email client | Communication protocol, not engineering tool |
| Web browser | Non-deterministic, non-reproducible, different domain |
| Project management | Team coordination, not solo engineering |
| IDE replacement | Different interface paradigm, different scope |
| Deployment platform | Irreversible actions require human ownership |
| Cloud dashboard | Different architecture, different security model |
| Social features | Personal tool, not social platform |

Every non-goal is a boundary. Every boundary is a promise to the user: this tool will not become something else.
