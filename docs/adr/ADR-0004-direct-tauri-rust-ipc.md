# ADR-0004: Direct Tauri Commands and Channels; Remove Internal Python RPC

Status: accepted

Date: 2026-08-31

## Context

The frontend already communicates with Rust through Tauri `invoke` and
`Channel`. Rust currently forwards many commands through a second framed JSON
RPC layer to the Python Core child process.

## Decision

Keep the frontend-visible Tauri command and event contract stable. Replace
internal `CoreSupervisor.request(method, json)` calls with direct typed Rust
application-service calls as each behavior reaches parity. Use a Rust bounded
event hub feeding the existing Tauri `Channel<UiEvent>`.

## Event policy

- lifecycle/state events remain ordered and are not silently dropped;
- telemetry snapshots are latest-wins/coalesced by session or operation;
- fatal queue conditions fail closed and trigger physical cleanup.

## Required retention

Strict WebView DTO validation, cancellation/timeouts where still needed,
emergency release, controlled shutdown, and the startup updater guard remain
explicit responsibilities after the internal RPC layer is removed.

This ADR does not authorize changes to the realtime scheduler or gameplay input
mechanism.
