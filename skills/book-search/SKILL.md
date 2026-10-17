---
name: book-search
description: Use before designing or fixing an engine subsystem, a Rust concurrency or unsafe question, or a test strategy. Search the game-engine and Rust books, then cite book id and heading in the packet or commit.
version: 0.1.0
author: skillworks
---

# Book search

Use before you design or fix an engine subsystem, answer a Rust concurrency or unsafe question, or write a test strategy. Cite each hit as `[id | heading]` in the packet or commit. Run from the repo root:

- `node tools/books.mjs search "<query>" [--book <id>[,<id>]] [--k 5]`: each hit prints `[id | heading] score` and up to 700 characters. The first search builds the index (about 4 s, 33 MB in `target/books/index.json`); later ones take under 1 s.
- `node tools/books.mjs toc <id>` lists headings. `node tools/books.mjs list` shows ids, language and size.

Example queries (topic: query, where it lands):

- render culling: `culling frustum` (found2 9.4 Frustum Culling)
- animation blending: `animation blending` (geaen 11.6)
- game loop timing: `game loop timing` (weak: see the geaen rule)
- rollback netcode: not in these books; try `networked multiplayer game loop` (geaen 7.7)
- memory ordering: `memory ordering acquire release` (bos, Russian)
- playtest bug report: `playtest bug report` (testing, "Why bug reports matter")

Books (id: title):

- geaen: Gregory, Game Engine Architecture, 2nd ed (English)
- gearus: Gregory, Game Engine Architecture, 3rd ed (Russian; adds a concurrency chapter)
- found1: Lengyel, Foundations of Game Engine Development, vol 1 Mathematics
- found2: Lengyel, Foundations of Game Engine Development, vol 2 Rendering
- gems1, gems2, gems3: Game Engine Gems 1, 2, 3
- trpl: The Rust Programming Language, 3rd ed
- progrust: Programming Rust, 3rd ed
- idiomatic: Idiomatic Rust
- bos: Rust Atomics and Locks (Russian)
- transition: Refactoring to Rust (Russian)
- rustwasm: Game Development with Rust and WebAssembly
- handsrust: Wolverson, Hands-on Rust (roguelike with ECS, legion)
- testing: Modern Game Testing
- gogame: Deep Learning and the Game of Go (Russian)
- dushkin: agentic and multi-agent design patterns (Russian)

Rules:

- Quote at most 2 short lines from a hit; cite the rest as `[id | heading]`.
- `(no heading)` means no heading was detected above that text; cite the book id only.
- geaen lost word spaces in about a quarter of its long lines ("Onecommonoptimization"). If a hit is missing, Grep `C:\empire\lab\skillworks\work\b1-<id>\full_text.txt` for the word with spaces removed.
- No stemming: a Russian query needs the word form the text uses.

source: tools/books.mjs; books in C:\empire\lab\skillworks\work
