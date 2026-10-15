# Security policy

## Supported

| track | status |
|---|---|
| `master` HEAD | supported |
| anything else | unsupported, update first |

## Disclosure

Do not open a public issue with secrets or exploit detail.
Contact the repo owner through a private channel with:
affected track, repro steps, log excerpt with secrets masked.

## Ground rules

- Never commit secrets: tokens, keys, model weights, private data.
- `deny.toml` gates dependency licenses in CI; keep the window-feature graph clean.
- Determinism goldens (`final_hash=d57b5c8ae3096614`) are consensus-critical: report hash breaks privately first.
