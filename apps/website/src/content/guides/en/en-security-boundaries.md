---
key: security-boundaries
locale: en
slug: security-boundaries
title: Security Boundaries
description: >-
  A short overview of Sky Auto Player's input boundary and the canonical security policy.
summary: >-
  Gameplay input uses Windows SendInput. The full security, release-trust, audit, and
  vulnerability-reporting rules are maintained in SECURITY.md.
category: technical-safety
order: 1
published: '2026-08-08'
updated: '2026-11-18'
draft: false
related:
  - how-it-works
  - windows-setup
  - timing-engine
evidence:
  - category: security
    label: Canonical security policy
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md
  - category: security
    label: Static security audit
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/rust/xtask/src/audits/security.rs
---

## At a glance

Sky Auto Player sends gameplay keystrokes through the public Windows
SendInput API. It does not read or modify game memory or files, inject code,
install keyboard/input hooks, attach a debugger, or bypass anti-cheat.

The audited foreground event callback only provides a hint to recheck the
current target. It cannot authorize a key send.

## Canonical policy

[SECURITY.md](https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md)
is the authoritative source for the complete security boundary, security
audit, updater and release trust, out-of-scope behavior, and vulnerability
reporting. Use that policy when reviewing implementation or reporting an issue.

Automated playback may conflict with Sky's Terms of Service. Sky Auto Player is
an unofficial community project; consult the canonical policy for the complete
notice and scope.
