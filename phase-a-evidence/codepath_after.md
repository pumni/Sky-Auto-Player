# Phase A code path — candidate

The worker now calls the trusted prepared target-aware sender directly. The
sender resolves the prepared packet pointer, length, and Windows INPUT size
before its QPC loop; the first sample `>= physical_target_qpc` is the
authoritative `pre_call_qpc`; it applies only the Down cutoff predicate; it
performs at most one Windows `SendInput`; and it samples completion QPC after
that syscall.

Up-only packets have no Down cutoff. Deadline equality is allowed. A Down
that crosses after the cutoff returns `DeadlineMissedBeforeSend` with zero
send attempts. Ownership, recovery, telemetry, and calibration policy remain
outside this Phase A change. Test-only seams preserve deterministic existing
tests and are not production paths.
