//! UserPromptSubmit hook: prepends the answer format rule to every prompt.
//!
//! Stdout enters the context directly before the user's message, the position
//! where a formatting rule does not get diluted by a long system prompt.
//!
//! This text is re-sent on every prompt, so it carries only what cannot be
//! checked after the fact: what shape the answer takes and when the rule
//! applies. The prose rules are left out on purpose. The Stop hook enforces
//! them and its error messages name both the violation and the fix, so
//! repeating them here would buy nothing and cost tokens on every turn.

use crate::config::{self, Config, ConfigError};
use crate::render;
use std::path::PathBuf;

/// Resolves the schema directory relative to the binary.
///
/// The hook is registered with an absolute path to the binary, and Claude Code
/// does not expand `${CLAUDE_PROJECT_DIR}` in the exec form. Deriving the path
/// from the executable keeps the registration to one absolute path.
///
/// N4T_SCHEMA_DIR overrides the search, which is what makes an installed copy
/// relocatable without a rebuild.
pub fn schema_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("N4T_SCHEMA_DIR") {
        return canonical(PathBuf::from(dir));
    }

    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("n4t"));
    let dir = exe.parent().unwrap_or(&exe).to_path_buf();

    let candidates = [
        // n4t next to schema/, as in a copy unpacked into one directory
        dir.join("schema"),
        // bin/n4t with share/n4t/schema beside it, the layout `just install`
        // produces
        dir.join("../share/n4t/schema"),
        // A build from the repository, including the test binaries. The build
        // directory may live outside the repository (see .cargo/config.toml),
        // so the repository is located at compile time, not relative to the
        // binary.
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema"),
    ];

    for candidate in candidates {
        if candidate.join("render.ncl").is_file() {
            return canonical(candidate);
        }
    }

    canonical(dir.join("schema"))
}

fn canonical(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

/// The rule text, with the schema path and the limits filled in.
pub fn rule_text(schema_dir: &std::path::Path, limits: &Config) -> String {
    let b = &limits.budgets;
    let languages = render::languages(limits)
        .iter()
        .map(|l| format!("'{l}"))
        .collect::<Vec<_>>()
        .join(" | ");
    format!(
        r#"Strictly adhere to the Bottom Line Up Front (BLUF) principle. Cut out irrelevant information.

Format the FINAL answer of a turn as a Nickel record in a nickel fenced block:

```nickel
let {{ render, .. }} = import "{schema}/render.ncl" in
render {{
  bottom_line = "...",       # required, max {bottom_line} words, must stand alone
  supporting_detail = "...", # optional, max {supporting_detail} words
  evidence = ["..."],        # optional, max {evidence} words per entry, facts only
  caveats = ["..."],         # optional, max {caveats} words per entry
  next_action = "...",       # optional, max {next_action} words
  confidence = 'verified,    # 'verified | 'high | 'medium | 'low
  language = '{default},            # {languages}, the language of the answer, default '{default}
}}
```

Only the final answer of a turn. Text before a tool call stays plain prose.

The record is the whole answer: a hook replaces it on screen with the rendered
Markdown, so prose beside it is a preamble and anything outside it is lost.
Keep only the optional fields you have something to say in, and fill them; a
placeholder is not an answer. Fields hold Markdown, m%"..."% for several lines.
To write that delimiter itself inside a field, open the field with more percent
signs than the text contains, so m%%"..."%% holds a literal m%"..."%. Backticks
do not help, since Nickel parses before any Markdown is read.

A Stop hook validates the record and names any violation. The rules it applies:
BLUF on `bottom_line`, no filler or apologies anywhere, no em-dash and no
en-dash between clauses, {sentence} words per sentence, and the budgets above.
"#,
        schema = schema_dir.display(),
        bottom_line = b.bottom_line,
        supporting_detail = b.supporting_detail,
        evidence = b.evidence,
        caveats = b.caveats,
        next_action = b.next_action,
        sentence = limits.sentence_length,
        default = limits.default_language,
    )
}

/// One line after the rule when the config file is broken. The Stop hook
/// falls back to the defaults too, so the rule stays consistent; the note is
/// how the user learns that their file is not in effect.
fn config_note(err: &ConfigError) -> String {
    format!("\nThe n4t config is invalid, so the default limits apply: {err}\n")
}

/// Plain text on stdout is inserted into the context as a system reminder when
/// the hook exits 0. The JSON form with `hookSpecificOutput.additionalContext`
/// exists too; v1 uses plain text and it works, so this does the same.
pub fn run() -> i32 {
    let (limits, note) = match config::load() {
        Ok(limits) => (limits, String::new()),
        Err(err) => (Config::default(), config_note(&err)),
    };
    print!("{}{note}", rule_text(&schema_dir(), &limits));
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn rule_text_carries_the_schema_path() {
        let text = rule_text(Path::new("/opt/n4t/schema"), &Config::default());
        assert!(text.contains("import \"/opt/n4t/schema/render.ncl\""));
    }

    #[test]
    fn rule_text_keeps_the_backtick_quoted_field_names() {
        // v1 built this text with a bash heredoc, where an unescaped backtick
        // turned prose into command substitution. That failure mode is gone in
        // Rust, but the assertion is cheap and the text is what Claude reads.
        let text = rule_text(Path::new("/s"), &Config::default());
        assert!(text.contains("`bottom_line`"));
    }

    #[test]
    fn rule_text_names_every_field() {
        let text = rule_text(Path::new("/s"), &Config::default());
        for field in [
            "bottom_line",
            "supporting_detail",
            "evidence",
            "caveats",
            "next_action",
            "confidence",
            "language",
        ] {
            assert!(text.contains(field), "missing {field}");
        }
    }

    #[test]
    fn rule_text_offers_the_configured_languages() {
        let limits = Config {
            default_language: "fr".to_string(),
            languages: [("fr".to_string(), Default::default())].into(),
            ..Config::default()
        };
        let text = rule_text(Path::new("/s"), &limits);
        assert!(text.contains("'en | 'fr"), "{text}");
        assert!(text.contains("default 'fr"), "{text}");
    }

    #[test]
    fn rule_text_carries_the_configured_limits() {
        let mut limits = Config::default();
        limits.budgets.bottom_line = 123;
        limits.sentence_length = 17;
        let text = rule_text(Path::new("/s"), &limits);
        assert!(text.contains("max 123 words"), "{text}");
        assert!(text.contains("17 words per sentence"), "{text}");
    }
}
