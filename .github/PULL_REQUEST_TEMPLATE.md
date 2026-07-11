<!-- Keep PRs focused. Link the issue: Closes #___ -->

## What & why

## Checks

- [ ] `cargo fmt --all` clean
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo test` passes

## Security

- [ ] No secrets, tokens, or real account data added (code, tests, commits)

## Family / cli-common

- [ ] No shared/reusable behavior copied in that belongs in [cli-common](https://github.com/piekstra/cli-common) (`pk-cli-*`)
- [ ] Surface, DTO, or exit-code changes reflected in cli-common `DESIGN.md` / `conformance.md`
