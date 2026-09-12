# Exact-head Windows regression and cutoff sweep

This is the official 17-case Windows runner started from implementation head
`b8bbe42d84837c440dd2e51cc90b490298aaa760` with a clean tracked source tree.
The runner used the production dispatch path and the project-owned
`ReceiveOnly` sink plus inert focus probe. It did not target Sky.

The run passed the 12 default scenarios and the `timing-margin-sweep` case at
500 µs. The next case, at 1,000 µs, failed qualification because its single
observed Up-completion-to-next-Down-pre-call interval was `15.1804 ms`, below
the fixed `16.667 ms` floor. It collected one release-gap sample, so it is not
a rate estimate. The runner correctly stopped there; the 2,000 and 5,000 µs
sweep cases were not executed in this run.

The full native reports, per-invocation sequence ledger, ready records, raw
event logs, runner output, and summary are preserved in this directory. The
13th default scenario (`focus-loss`) was run separately on the same exact head;
see the sibling `input-reliability-focus-loss-20260912T235844-465b29a6/`
directory.
