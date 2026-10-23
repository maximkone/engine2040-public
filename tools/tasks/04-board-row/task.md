# Task 4: add a board row

The board is a markdown file of rows. A row is one line, then its needs and its
Accept command:

```md
- [ ] EG50 agent: serve the frame verb over the door.
  needs: EG49. Accept: node tools/board-slice.mjs EG50.
```

Add the `EG50` row below `EG49`. The Accept command must name a tool that
exists in the tree. A row that needs an id nobody has, or accepts with a tool
nobody can run, is the mutation this task keeps red.

## Start

```sh
node tools/tasks/04-board-row/referee.mjs --apply start --dest target/task-04
```

## Accept

```sh
node tools/tasks/04-board-row/referee.mjs --tree target/task-04
```

Exit 0 needs exactly one unticked `EG50` row, a `needs:` id that exists on the
board, and an `Accept: node <path>` whose path is a file in the tree.
