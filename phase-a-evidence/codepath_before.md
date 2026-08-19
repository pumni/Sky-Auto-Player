# Phase A code path — baseline

The baseline worker performed the target-crossing QPC spin in the player
dispatch layer. After the crossing it handed the prepared packet to the
tracked sender, which performed the prepared-packet cutoff/SendInput attempt
and completion timestamping. The target-crossing timestamp and the trusted
SendInput payload/call were therefore split across layers.

The baseline used the same target, 700 us spin policy, 2,000 us guard, and
500 us Down grace policy. No calibration behavior was part of this baseline.
