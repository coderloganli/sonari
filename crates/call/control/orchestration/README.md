# call/orchestration

`call/orchestration` owns cross-owner call flow composition.

## Responsibilities

- own server-initiated turn request modeling
- build bot-speech items for explicit backend-triggered turn events
- define trigger-policy extension points for future server-initiated turns

## Non-responsibilities

- worker runtime fact collection
- audio queue policy, interruptibility, or flush semantics
- LiveKit details
- SQL or persistence adapters
- HTTP request parsing

## Current runtime scope

The runtime path currently uses this layer for:

- `call_started` server-initiated turn generation

Future trigger-policy scenarios may expand here, but worker/runtime must continue to report neutral
facts while orchestration owns the business interpretation of those facts.
