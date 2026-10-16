# Evidence pack example

One real claim from this tree, bundled with the checklist only. Builder time under 15 min. A second seat replays it by rerunning the command line and matching the SHA.

## Claim

`tools/fnv1a64.test.mjs` passes: 7 pass, 0 fail.

## The 5 parts, filled

1. **Command:** `node --test tools/fnv1a64.test.mjs` (pwsh, workdir `C:\engine2040`, no flags, no seed).
2. **Exit:** `0`. **SHA:** `f76335b32761e9b941a552ee7abb55394f2cd7562eb7cc797d2c32f146d331bb` (SHA-256 of the exact output bytes, 15 lines).
3. **Excerpt** (the lines that prove the verdict):
   `pass 7`
   `fail 0`
4. **Verdict:** `PASS 7 pass 0 fail`.
5. **Replay:** rerun the command line above in the same workdir; the run passes with exit 0 and the output SHA matches when output bytes are identical.

## Report line

`EVIDENCE: node --test tools/fnv1a64.test.mjs | exit 0 | sha f76335b3…46d331bb | PASS 7 pass 0 fail`
