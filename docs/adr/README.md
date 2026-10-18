# ADR — decision records

Short records. One file per decision.
Status is Proposed until the lead accepts.
Every record names a Compliance check path that exists; `node --test docs/adr/adr.test.mjs` enforces it.

## Template

- Title
- Status: Proposed | Accepted | Superseded
- Context: what is true today
- Decision: what is proposed
- Consequences: what changes
- Compliance: the check that fails if broken
- Advice: who was asked

## Index

| ID | Title | Status |
|---|---|---|
| 0001 | Document engine-endpoint -> engine-net edge | Proposed |
| 0002 | Two finish files | Proposed |
| 0003 | Netcode path for the duel | Proposed |
| 0004 | Handles and residency | Proposed |
| 0005 | Physics bodies off in the shipped app | Proposed |
| 0006 | Single-threaded sim until RC36 parity | Proposed |
| 0007 | No Bevy viewer and no Bevy dependency | Proposed |
| 0008 | Frozen scope until the first real frame | Proposed |
| 0009 | Donor code may be ported, deny.toml is Cargo only | Proposed |
| 0010 | Parry window | Accepted |
| 0011 | Audio parked, no owner | Proposed |
