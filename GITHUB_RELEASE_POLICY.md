# GitHub Release Policy

This repository uses trunk-based development with squash merges.

## Rules

- `main` is the only long-lived branch. It changes only through pull
  requests. Direct pushes are blocked.
- Every pull request is merged with **Squash and Merge**. One pull request
  becomes one commit on `main`.
- Pull requests are merged one at a time. The next branch starts from the
  current `main`.
- A pull request must pass the `just check` status check before it merges.
  The check runs the `check` recipe from the `Justfile`
  ([just](https://github.com/casey/just) command runner): tests, Clippy,
  rustfmt and an end-to-end run of the hooks.

## Pull request text

The pull request title and description become the commit message on `main`.
Keep them short.

- Title: one line, sentence case, imperative mood, no prefix such as `feat:`,
  no trailing period.
- Description: what changed and why, in a few lines of plain English.
  No boilerplate.

## Why

Squash merging keeps `main` linear and readable. Each commit on `main` is one
finished change with a description that explains it. Work-in-progress commits
stay in the pull request instead of the history.
