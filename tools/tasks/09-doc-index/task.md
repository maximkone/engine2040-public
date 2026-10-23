# Task 9: add a doc and its index line

Every file under `docs/` needs a line in `docs/INDEX.md`: the path and one
purpose. Add `docs/verbs.md`, a short page for the door verbs, and its index
line.

A page nobody can find is the mutation this task keeps red: the doc exists, the
index does not mention it.

## Start

```sh
node tools/tasks/09-doc-index/referee.mjs --apply start --dest target/task-09
```

## Accept

```sh
node tools/tasks/09-doc-index/referee.mjs --tree target/task-09
```

Exit 0 needs `docs/verbs.md` with a `# ` title, an index line for it with a
purpose, and no doc in the tree missing from the index.
