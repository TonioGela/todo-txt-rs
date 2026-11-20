# todo-txt-rs

An inline terminal UI for [todo.txt](https://github.com/todotxt/todo.txt). It reads and writes the
same files as `todo.sh`, filters by project, context and priority, and keeps the todo directory
in sync with git.

## Using it

- `←`/`→` (or `h`/`l`): change the value of the focused filter
- `tab` / `shift-tab`: focus Project, Context or Priority
- `↑`/`↓` (or `j`/`k`): move through the list
- `/`: filter the list by text (`enter` keeps it, `esc` clears it)
- `a`: add a todo, `e`: edit the selected one
- `x`: complete the selected todo, or restore a done one
- `d`: show or hide done items
- `q` (or `esc` with no filter): quit

The prompt on the bottom line takes the usual arrow, `home`, `end`, `backspace` and `delete` keys.

Filters only show the axes that have values. Project, Context and Priority tabs appear once the
file has a `+project`, `@context` or `(A)` priority.

## Where it reads and writes

In order of precedence:

1. Environment variables `TODO_DIR`, `TODO_FILE` and `DONE_FILE`
2. `~/.todo/config` or `~/.todo.cfg`, the files `todo.sh` uses
3. `~/.todo`, the default directory of `todo.sh`

If the todo file lives in a git repository, the app runs `git pull --rebase --autostash` when it
starts. Every change is committed on its own (`todo: add …`, `todo: done …`, and so on), and the
commit is pushed straight away. Errors appear on the bottom line and clear on the next key press.

## Building with Nix

The build uses the `nixpkgs` pinned in `npins/`, which is the same `nixos-26.05` pin as in your
global npins. No flakes are needed.

```sh
nix-build              # builds ./result/bin/todo-txt-rs
./result/bin/todo-txt-rs
```

### Install

Install into your user profile:

```sh
nix-env -f . -i
```

Remove it again with `nix-env -e todo-txt-rs`.

### Updating nixpkgs

```sh
npins update nixpkgs
```

This moves the pin forward. Commit the changed `npins/sources.json` to keep the build reproducible.

### Without Nix

```sh
cargo build --release   # binary at target/release/todo-txt-rs
```

## Releases

`.github/workflows/build.yml` builds and tests on Linux, macOS (both through Nix) and Windows
(through cargo) for every push and pull request. Pushing a tag such as `v0.1.0` also publishes a
GitHub release with the three binaries. The tag must match the version in `Cargo.toml`.

```sh
git tag v0.1.0 && git push origin v0.1.0
```

## Development

```sh
cargo test
```

## License

MIT, see [LICENSE](LICENSE).
