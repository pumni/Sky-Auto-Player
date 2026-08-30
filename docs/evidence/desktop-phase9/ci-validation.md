# Phase 9 CI validation-layer evidence

The CI classifier has three independent decisions:

| Changed-path class | Static/security | Windows/code | Portable package |
| --- | ---: | ---: | ---: |
| `README.md`, `docs/**`, `site/**` | run | skip | skip |
| `tests/**` | run | run | skip |
| `src/**`, `desktop/**`, `rust/**`, packaging inputs | run | run | run |
| `push` to `main` or manual full validation | run | run | run |

The representative docs/site-only input
`README.md`, `docs/evidence/desktop-phase9/README.md`, and
`site/src/pages/index.astro` produces:

```text
static_required=true
code_required=false
package_required=false
classification_reason=static/site/docs only
```

The required-gate job treats a skipped validation job as acceptable only when
the corresponding classifier output is `false`. A package-sensitive change
cannot skip the portable qualification job, while `push` and
`workflow_dispatch` always use `--full`.

## Timing comparison

The broad classifier used before this change caused the representative
docs/site-oriented Phase 9 run `33307710377` to start the heavy jobs. Its
packaged job took approximately 13m31s; the Windows compatibility job took
approximately 15m34s, and the portable build step itself took approximately
12m17s.

The classifier-only evaluation for the docs/site input above completes in
under one second locally and schedules no Windows or portable job. GitHub does
not report a runner duration for jobs that are skipped, so the exact
post-change GitHub wall-clock reduction is reported from the first
docs/site-only pull request run rather than guessed here. This is a scheduling
optimization only: full exact-package qualification remains mandatory for
package-sensitive pull requests, `main` pushes, releases, and manual full
validation.
