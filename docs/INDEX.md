# Technical Documentation — Find by Task

Use this as a route map, not a reading list. Open the owning contract for the
changed behavior; follow its source/test references only as needed.

| Task | Current owner |
| --- | --- |
| Module ownership, IPC/native boundaries | [Architecture](architecture.md) |
| Native scheduling, admission, timing, cleanup, retirement | [Playback engine](playback-engine.md) |
| Windows tests, timing evidence and authorization | [Playback verification](playback-verification.md) |
| Desktop behavior, layout and preparation UI | [Workbench](desktop-workbench.md) |
| Desktop frontend stack and type boundary | [Type boundary](desktop-type-boundary.md) |
| App commands, browser runners and worktree isolation | [JavaScript tooling](desktop-tooling.md) |
| Visual UI work | [Visual reference](ui/README.md) (check capture SHA) |
| Library identity and membership | [Library V1](library-v1-contract.md) |
| Built-in songs and provenance | [Manifest](../builtin-songs/manifest.json), [curation](song-curation/README.md) |
| Sheet parsing, source compatibility | [Source decoding](song-source-decoding.md) |
| Pitch mapping, tempo, schedule and risk | [Song planning](song-planning.md) |
| Security and forbidden capabilities | [Security policy](../SECURITY.md) |
| Distribution, update channels and package trust | [Distribution](distribution-and-update.md) |
| Tauri installer, packaging and package verification | [Tauri packaging](v4-tauri-packaging.md) |
| Release transaction and promotion authority | [Release authority](v4-release-authority.md) |
| Hosted execution, runner trust and recovery | [Release execution](v4-release-execution-topology.md) |
| Updater private-key inventory/custody/rotation | [Updater key custody](v4-updater-key-custody.md) |
| Authenticode provider and signer seam | [Authenticode](v4-authenticode-provider-seam.md) |
| CI lanes, local impact preview, verification evidence | [CI topology](ci-validation-topology.md) |
| Compiler pin, MSRV, production lint | [Rust toolchain policy](rust-toolchain-policy.md) |
| Contribution and review practice | [Contributing](../CONTRIBUTING.md) |

[HISTORY.md](HISTORY.md) links revision-bound decisions and evidence, not live
work orders or current qualification. Published user-facing notes live in
[GitHub Releases](https://github.com/pumni/Sky-Auto-Player/releases); unreleased
candidates require source-tagged `docs/releases/{tag}.md` before preflight per
the release authority.

When code/tests and an owning contract disagree, investigate both and report
uncertainty; neither old implementation nor historical instructions override
current safety and release requirements.
