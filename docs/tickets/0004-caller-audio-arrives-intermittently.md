# 0004 — The caller was talking over the greeting

**Status**: Fixed 2026-08-15 · **Found by**: the evaluation harness · **Area**: eval clients

## What was happening

The caller's audio reached the segmentation policy only sometimes: the loudest
frame in a second would be 9, on a clip whose frames peak at 16823.

It was not transport. The service drops inbound frames outright while its own
turn is pending (`InputGateMode::Closed` and `OutputTurnPending` both `continue`
without pushing), and both eval clients started speaking the moment they saw the
bot's track — which is during the greeting, not after it. Whether a clip was
heard came down to whether it happened to land in the barge-in path.

## The fix

The live solver now waits for the greeting to finish before speaking: it
subscribes to the bot's audio and starts the clip once that has been quiet for
700 ms. That is what a caller does, and it also keeps the measurement about an
ordinary turn rather than a barge-in, which is a different thing that would want
its own clips.

`crates/probe` starts speaking immediately for the same reason and has the same
blind spot; its `perceived_response_ms` has been 0.0 throughout, because what it
heard was the greeting rather than an answer. Worth the same treatment.

## What it unblocked

The first complete live evaluation, and with it ticket 0001's answer.
