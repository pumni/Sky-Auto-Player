# Manifest

| File | Purpose |
|---|---|
| README.md | entry point and key decisions |
| 00_BASELINE_AND_SCOPE.md | scope, versions, definition of full migration |
| 01_CURRENT_DISPATCH_FLOW.md | current end-to-end flow analysis |
| 02_BEHAVIORAL_INVARIANTS.md | non-negotiable contracts |
| 03_TARGET_RUST_ARCHITECTURE.md | final native worker architecture |
| 04_PYO3_CONTRACT.md | free-threaded PyO3 API and adapter |
| 05_RUST_CRATE_AND_DATA_DESIGN.md | workspace/data/ownership design |
| 06_WIN32_SENDINPUT_PORT.md | exact Win32 port/retry/wait rules |
| 07_MIGRATION_PHASES.md | incremental PR sequence |
| 08_TEST_AND_BENCHMARK_PLAN.md | differential, fault and perf gates |
| 09_BUILD_PACKAGING.md | Maturin/uv/PyInstaller/CI |
| 10_TELEMETRY_UI_INTEGRATION.md | schema/snapshot/UI mapping |
| 11_RISKS_ROLLBACK.md | risk register and fallback |
| 12_AI_CODING_RUNBOOK.md | prompts and agent workflow |
| 13_DEFINITION_OF_DONE.md | final acceptance checklist |
| references/* | source map, version baseline and diagrams |
| templates/* | starting scaffold, intentionally incomplete |
