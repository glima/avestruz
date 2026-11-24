# Contributing to Avestruz

What a change has to pass, and how its commit is written. How the code
is laid out, and why, is in the guide: `just doc-open`, then
`guide::architecture` and `guide::decisions`.

## Before a change goes in

```sh
just check   # formatting, clippy, unused dependencies, commit messages, the docs and their diagrams, licences, advisories
just test    # the tests
```

Both run on GitHub for every push to `main` and every pull request
(`.github/workflows/ci.yml`), with every tool `just check` would skip
installed. What each step of `just check` holds to is listed in
`guide::testing`.

## Commit messages

**[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) are
required.** The history is uniform and stays that way:

```
type(scope): imperative summary, lowercase, no trailing period

Body explaining what was wrong and why this is the fix, wrapped at 72
columns. Findings and measurements belong here — they are the part that
survives.

BREAKING CHANGE: only when a user has to do something differently.
```

- **Types**: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`,
  `ci`, `chore`, `style`, plus `security`, `deps`, `packaging`, `license` and
  `release`. That list is [`cog.toml`](./cog.toml), and it is checked: `just
  check` runs `cog check --from-latest-tag`, so an unknown type or a
  non-conventional subject fails the build. Each type also has a section in
  [`cliff.toml`](./cliff.toml), so anything that passes the check appears in
  the release notes. A `!` before the colon marks a breaking change, pairs
  with a `BREAKING CHANGE:` trailer, and earns its own section.
- **Scopes** are the area touched, not the file: `auth`, `feed`, `thread`,
  `compose`, `keys`, `video`, `media`, `relays`, `zap`, `ui`, `deps`,
  `audit`, `i18n`.
- **Bodies matter more than summaries.** Say what the behaviour was, what it
  is now, and how you know — an error message, a measured number, a relay's
  answer. "Fix bug" tells the next reader nothing; the commit is where the
  evidence lives.
- **One concern per commit.** Formatting churn goes in its own `style:`
  commit so it doesn't bury a fix.

## Releases

A tag `vX.Y.Z` on `main` is a release: `.github/workflows/release.yml`
builds the packages and writes the release notes from the commits since
the release before, a section for each type of commit.
