# Agent Instructions

## Language Policy

**Always** write code and code comments in English. Communicate with the user in their preferred language, but **never** write code or comments in the user's language. If the user's language distinguishes between formal and informal pronouns, always use the informal form.

## Language Usage

Role model: News language.

- Information first.
- Short sentences, strong verbs.
- Be transparent, accountable, reliable and authentic.
- State facts in the indicative mood and uncertainties in the subjunctive mood.
  - *Correct (Fact):* "The file `config.toml` was modified."
  - *Correct (Uncertainty):* "This could potentially cause a race condition if..."
  - *Incorrect:* "I think I have successfully modified the file." (Do not guess about facts).
- **Avoid all-caps emphasis:** Do not use capitalization to emphasize rules, conditions, or importance (e.g., instead of "ALWAYS", use "**always**"). Use Markdown formatting for emphasis instead.
- **No dashes for clauses:** Do not use em-dashes (—) or en-dashes (–) to connect clauses or thoughts. Use commas, colons, or parentheses instead, or write shorter sentences.

## Admitting Mistakes

Admit mistakes immediately and without sugarcoating.
Do not call a mistake "confusion".

## Versioning

n4t follows [Semantic Versioning](https://semver.org). The version lives in
`Cargo.toml`. The public interface is the CLI (subcommands, arguments, exit
codes), the hook input and output, the record schema in `schema/`, the config
format and the language pack format.

Pick the level from the largest change since the last version tag:

| Change | Below 1.0.0 | From 1.0.0 |
|---|---|---|
| Breaks the public interface | minor | major |
| New feature, backward compatible | minor | minor |
| Bug fix, performance, internal change of the binary | patch | patch |
| Docs, tests, CI, tooling only | none | none |

When a change is ready for a release, propose the level with a one-line
reason and wait for the maintainer to confirm. Then raise it with
`cargo set-version --bump <level>`, which updates `Cargo.toml` and
`Cargo.lock`, and commit that alone as `Raise version to X.Y.Z`.

Version tags and releases belong to the maintainer. **Never** create, move,
delete or push a `v*` tag, and never create a release.

## Git Commits

Commit under the user's own credentials. **Never** add an AI signature, a
`Co-Authored-By` trailer, or any mention of an AI agent or tool.

The project discloses its use of AI tools once, in the README. Repeating that
in every commit adds no information about the change itself, and the history
should stay focused on what changed and why.

Before staging, check the diff for secrets. If you find one, name what and
where, never the value, and stop.

The message:

- **Sentence case.** First letter capitalized, standard English
  capitalization, no trailing period.
- **Imperative mood.** "Make lock acquisition atomic", not "Makes" or "Made".
- **No type or topic prefix.** No `feat:`, `fix:`, `refactor:`, and no
  colon-separated prefix of any kind. Plain sentences only.
- **No emojis and no colons.**
- **Plain language.** Write for the people who read the history: clear,
  specific wording without filler or generated-sounding phrases.
- **Short.** One line preferred, at most three. Add lines only when the
  subject cannot carry the change: the subject line, a blank line, then one or
  two short lines of plain text. No bullet points.

Good: `Make lock acquisition atomic with O_EXCL`, `Validate config profiles on
load`, `Restrict mount targets to configured base path`.

Bad: `refactor: deduplicate orchestrator logic`, `fix: resolve race condition
in lock acquisition`, `harden for production: atomic lock, config validation,
tests`.

Commit what the working tree holds. If it carries several unrelated changes,
say so and propose separate commits rather than inventing one message that
covers everything.

Branching, pull requests and releases follow `GITHUB_RELEASE_POLICY.md` in the
repository root. Read it before the first commit of a session. It covers the
workflow only and never overrides the rules on secrets.

## Secrets

Credentials, tokens, keys, passwords, connection strings, personal data: never printed, not in a response, log, commit message, error, or comment. Refer to them by name or path.

Do not read `.env` files, keyrings, SSH keys, or cloud config unless that is exactly what was asked. Do not write a secret outside a designated secret store, never into version control, never into an outward call such as an API request, web search, or subagent prompt. Check the diff before staging.

Found one incidentally: say what and where, never the value. Ambiguous case: treat it as a secret.
