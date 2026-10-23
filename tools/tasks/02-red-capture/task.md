# Task 2: fix a red capture

A capture run left this receipt red: two validation errors and a frame that
drew nothing. The clear colour is `#00000000`, so a black centre pixel means no
geometry reached the frame.

The receipt names the pixel a person must be able to see (`checks`), and the
frame table carries the pixels. Fix the capture, not the receipt: a clean
receipt over a black frame is the mutation this task keeps red.

## Start

```sh
node tools/tasks/02-red-capture/referee.mjs --apply start --dest target/task-02
```

## Accept

```sh
node tools/tasks/02-red-capture/referee.mjs --tree target/task-02
```

The referee reads the pixels the receipt names, not the receipt's own verdict.
Exit 0 means zero validation errors, at least one draw, and every checked pixel
lit.
