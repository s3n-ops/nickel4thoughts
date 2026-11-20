//! Renders a validated answer record as Markdown.
//!
//! The record is the answer's source form. This is what the reader sees.

use crate::answer::{Answer, Confidence};
use crate::config::Config;

/// The language whose headings are built in, as the fallback for every other.
pub const BUILTIN_LANGUAGE: &str = "en";

/// Headings and the confidence label follow the language of the answer, not
/// the language of the schema. They borrow from the built-in English table or
/// from the config, whichever supplies the text.
#[derive(Clone, Copy)]
struct Labels<'a> {
    bottom_line: &'a str,
    supporting_detail: &'a str,
    evidence: &'a str,
    caveats: &'a str,
    next_action: &'a str,
    confidence: &'a str,
    verified: &'a str,
    high: &'a str,
    medium: &'a str,
    low: &'a str,
}

const EN: Labels<'static> = Labels {
    bottom_line: "Bottom line",
    supporting_detail: "Details",
    evidence: "Evidence",
    caveats: "Caveats",
    next_action: "Next step",
    confidence: "Confidence",
    verified: "executed and observed",
    high: "high",
    medium: "medium",
    low: "low",
};

impl<'a> Labels<'a> {
    /// The English table with the pack's labels for `language` laid over it.
    /// A label the pack leaves out, and every label of a language without a
    /// pack, stays English rather than failing, since the reader still gets
    /// every section.
    fn resolve(language: &str, config: &'a Config) -> Labels<'a> {
        let mut l: Labels<'a> = EN;
        let Some(o) = config.language(language).map(|lang| &lang.labels) else {
            return l;
        };
        let pick = |own: &'a Option<String>, base: &'a str| own.as_deref().unwrap_or(base);
        l.bottom_line = pick(&o.bottom_line, l.bottom_line);
        l.supporting_detail = pick(&o.supporting_detail, l.supporting_detail);
        l.evidence = pick(&o.evidence, l.evidence);
        l.caveats = pick(&o.caveats, l.caveats);
        l.next_action = pick(&o.next_action, l.next_action);
        l.confidence = pick(&o.confidence, l.confidence);
        l.verified = pick(&o.verified, l.verified);
        l.high = pick(&o.high, l.high);
        l.medium = pick(&o.medium, l.medium);
        l.low = pick(&o.low, l.low);
        l
    }

    fn confidence_label(&self, confidence: Confidence) -> &'a str {
        match confidence {
            Confidence::Verified => self.verified,
            Confidence::High => self.high,
            Confidence::Medium => self.medium,
            Confidence::Low => self.low,
        }
    }
}

/// Every language n4t knows: English and every language with a pack, sorted
/// and without duplicates. The injected rule lists them.
pub fn languages(config: &Config) -> Vec<&str> {
    let mut all: Vec<&str> = std::iter::once(BUILTIN_LANGUAGE)
        .chain(config.languages.keys().map(String::as_str))
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}

/// Entries may hold Markdown, including multiple lines. Continuation lines are
/// indented so they stay inside the bullet instead of ending the list.
fn bullet_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- {}", item.replace('\n', "\n  ")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Sections are dropped when empty, so the output has no hollow headings.
fn push_section(sections: &mut Vec<String>, title: &str, body: &str) {
    if !body.is_empty() {
        sections.push(format!("## {title}\n\n{body}"));
    }
}

/// Renders the record as Markdown.
///
/// Two leading newlines, so one blank line separates the first heading from
/// whatever precedes the rendering on screen. Prose emitted before the block
/// ends with its own newline, which a single one here only completes; the
/// heading then sits directly under the last sentence instead of apart from
/// it. The pair is harmless when nothing precedes, since a terminal collapses
/// leading vertical space.
pub fn render(answer: &Answer, config: &Config) -> String {
    let l = Labels::resolve(answer.language(config), config);
    let mut sections = Vec::new();

    push_section(&mut sections, l.bottom_line, &answer.bottom_line);

    if let Some(detail) = &answer.supporting_detail {
        push_section(&mut sections, l.supporting_detail, detail);
    }

    if !answer.evidence.is_empty() {
        push_section(&mut sections, l.evidence, &bullet_list(&answer.evidence));
    }

    if !answer.caveats.is_empty() {
        push_section(&mut sections, l.caveats, &bullet_list(&answer.caveats));
    }

    if let Some(next) = &answer.next_action {
        push_section(&mut sections, l.next_action, next);
    }

    sections.push(format!(
        "*{}: {}*",
        l.confidence,
        l.confidence_label(answer.confidence)
    ));

    format!("\n\n{}", sections.join("\n\n"))
}
