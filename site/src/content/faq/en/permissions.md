---
key: 'permissions'
locale: 'en'
order: 12
category: 'General'
question: 'Does Sky Auto Player need admin rights?'
---

The canonical v4 app uses a current-user Tauri NSIS installer and does not require administrator
access for normal use. Uninstall it through Windows **Installed apps**. Updates use the official
Tauri updater through the Rust-owned `UpdateService`.
