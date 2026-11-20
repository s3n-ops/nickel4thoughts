//! The answer record and its validation.
//!
//! Nickel parses the record and checks its structure; the prose rules and the
//! word budgets run here, with the phrase lists of the answer's language. The
//! checks a language pack writes in Nickel run last.

use crate::checks::{self, Text};
use crate::config::Config;
use crate::prose::strip_markdown;
use crate::rules::{self, Check, Phrases, Violation};
use serde::Deserialize;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Verified,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    /// The single most important statement. Comes first, always present.
    /// Answers the question on its own, without the fields below.
    pub bottom_line: String,

    /// Reasoning, evidence, mechanism. Only what supports the bottom line.
    #[serde(default)]
    pub supporting_detail: Option<String>,

    /// What was actually run or observed, as opposed to inferred.
    #[serde(default)]
    pub evidence: Vec<String>,

    /// Known limits of the answer.
    #[serde(default)]
    pub caveats: Vec<String>,

    /// Concrete next step for the reader, if one exists.
    #[serde(default)]
    pub next_action: Option<String>,

    /// Confidence in the bottom line. Forces an explicit stance.
    pub confidence: Confidence,

    /// Language of the answer, as the tag without its quote (`'fr` arrives
    /// as "fr"). Any tag is accepted. It picks the language pack, which
    /// supplies the headings and the phrase lists; without the field the
    /// configured default applies. A tag without a pack gets English headings
    /// and the universal rules only.
    #[serde(default)]
    pub language: Option<String>,
}

/// A violation together with the field it was found in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldViolation {
    pub field: String,
    pub violation: Violation,
}

impl fmt::Display for FieldViolation {
    /// Mirrors Nickel's own layout: a headline naming the field, then the rule
    /// on an indented continuation line, then the notes. Claude reacts to
    /// that shape, so the rules written in Rust keep it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "contract broken by the value of `{}`\n{}",
            self.field, self.violation.message
        )?;
        for note in &self.violation.notes {
            write!(f, "\n{note}")?;
        }
        Ok(())
    }
}

/// Runs one check and tags a violation with its field name.
fn field(name: &str, check: Check) -> Result<(), FieldViolation> {
    check.map_err(|violation| FieldViolation {
        field: name.to_string(),
        violation,
    })
}

/// Runs one check against an array entry, naming the index in the field.
fn entry(name: &str, index: usize, check: Check) -> Result<(), FieldViolation> {
    check.map_err(|violation| FieldViolation {
        field: format!("{name}[{index}]"),
        violation,
    })
}

impl Answer {
    /// The language tag the answer is judged and rendered in.
    pub fn language<'a>(&'a self, config: &'a Config) -> &'a str {
        self.language.as_deref().unwrap_or(&config.default_language)
    }

    /// Validates every field against the configured limits. The first
    /// violation wins, so the error names one concrete problem instead of a
    /// list the writer has to triage.
    pub fn validate(&self, config: &Config) -> Result<(), FieldViolation> {
        let language = config.language(self.language(config));
        let phrases = language.map_or_else(Phrases::none, |l| Phrases::compile(&[&l.phrases]));
        self.validate_rules(config, &phrases)?;

        // The pack's own checks see only answers that passed everything else,
        // so a writer fixes one problem at a time. A check that fails or runs
        // too long lets the answer through: a broken pack must not block every
        // turn.
        let Some(pack) = language.and_then(|l| l.checks.as_ref()) else {
            return Ok(());
        };
        match checks::run(pack, &self.prose()) {
            Ok(Some(hit)) => Err(FieldViolation {
                field: hit.field,
                violation: Violation {
                    message: hit.message,
                    notes: vec![format!(
                        "Reported by the check `{}` of the language pack.",
                        hit.check
                    )],
                },
            }),
            Ok(None) | Err(_) => Ok(()),
        }
    }

    /// Every prose field with its name, Markdown removed, in the order the
    /// rules visit them.
    fn prose(&self) -> Vec<Text> {
        let text = |field: String, value: &str| Text {
            field,
            text: strip_markdown(value),
        };
        let mut out = vec![text("bottom_line".to_string(), &self.bottom_line)];
        out.extend(
            self.supporting_detail
                .iter()
                .map(|d| text("supporting_detail".to_string(), d)),
        );
        out.extend(
            self.evidence
                .iter()
                .enumerate()
                .map(|(i, e)| text(format!("evidence[{i}]"), e)),
        );
        out.extend(
            self.caveats
                .iter()
                .enumerate()
                .map(|(i, c)| text(format!("caveats[{i}]"), c)),
        );
        out.extend(
            self.next_action
                .iter()
                .map(|n| text("next_action".to_string(), n)),
        );
        out
    }

    fn validate_rules(&self, config: &Config, phrases: &Phrases) -> Result<(), FieldViolation> {
        let budgets = &config.budgets;
        let sentence = config.sentence_length;

        field("bottom_line", rules::non_empty(&self.bottom_line))?;
        field("bottom_line", rules::filled_in(&self.bottom_line))?;
        field(
            "bottom_line",
            rules::bluf(phrases, sentence, &self.bottom_line),
        )?;
        field(
            "bottom_line",
            rules::max_words(budgets.bottom_line, &self.bottom_line),
        )?;

        if let Some(detail) = &self.supporting_detail {
            field("supporting_detail", rules::non_empty(detail))?;
            field("supporting_detail", rules::filled_in(detail))?;
            field(
                "supporting_detail",
                rules::prose_rules(phrases, sentence, detail),
            )?;
            field(
                "supporting_detail",
                rules::max_words(budgets.supporting_detail, detail),
            )?;
        }

        for (i, item) in self.evidence.iter().enumerate() {
            entry("evidence", i, rules::filled_in(item))?;
            entry("evidence", i, rules::evidence_rules(phrases, item))?;
            entry("evidence", i, rules::max_words(budgets.evidence, item))?;
        }

        for (i, item) in self.caveats.iter().enumerate() {
            entry("caveats", i, rules::filled_in(item))?;
            entry("caveats", i, rules::caveat_rules(phrases, item))?;
            entry("caveats", i, rules::max_words(budgets.caveats, item))?;
        }

        if let Some(next) = &self.next_action {
            field("next_action", rules::non_empty(next))?;
            field("next_action", rules::filled_in(next))?;
            field("next_action", rules::prose_rules(phrases, sentence, next))?;
            field("next_action", rules::max_words(budgets.next_action, next))?;
        }

        Ok(())
    }
}
