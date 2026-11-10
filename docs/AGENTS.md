# Documentation Context

Use [INDEX.md](INDEX.md) when a task needs a current technical contract.

Historical ADRs, release records, and measurements preserve the decisions,
source revisions, and conditions they actually describe; they are not
instructions for current HEAD. Current playback architecture and qualification
contracts are defined in [playback-engine.md](playback-engine.md) and
[playback-verification.md](playback-verification.md). Archived reports, traces,
and retired ADRs may be retrieved via [HISTORY.md](HISTORY.md) at immutable Git
commits instead of keeping copies in the active tree. For visual UI tasks,
see [ui/README.md](ui/README.md).

Keep one-off screenshots, traces, and run logs out of tracked docs by default;
use ignored test-results or CI artifacts. Commit durable evidence only when a
reproducible test, release qualification, or other explicit consumer requires it.

For active contracts, resolve drift using current source, direct tests, and
the applicable security, timing, or release requirements.
