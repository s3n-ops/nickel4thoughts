//! The BLUF principle, expressed as rules.
//!
//! Bottom Line Up Front means: the conclusion comes first, on its own, stated
//! plainly. Everything below it is support the reader may skip. The rules here
//! are the mechanically checkable part of that. What cannot be checked is
//! whether the stated bottom line is in fact the most important one; that stays
//! the writer's judgement.
//!
//! Every rule rejects a specific way of burying the conclusion. Each one exists
//! because it is a real failure mode, not because it is easy to test.
//!
//! The rules come in two layers. The universal ones need no knowledge of the
//! language: word budgets, sentence length, dashes, placeholders, a closing
//! question mark. The phrase rules recognise preambles, hedging and filler by
//! their wording, so their words come from the language packs (see
//! `src/config.rs`). This module fixes what each category means and what its
//! violation says; a pack only supplies the phrases.
//!
//! The violation texts are part of the user interface. Claude reacts to their
//! wording, so they name the rule, the phrase found and the fix.

use crate::config::PhraseLists;
use crate::prose::{first_prose_line, prose_word_count, re, strip_markdown, word_boundaries};
use regex::{Regex, RegexBuilder};

/// A rule violation, addressed to the writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub message: String,
    pub notes: Vec<String>,
}

impl Violation {
    fn new(message: &str) -> Self {
        Violation {
            message: message.to_string(),
            notes: Vec::new(),
        }
    }

    fn with_note(message: String, note: String) -> Self {
        Violation {
            message,
            notes: vec![note],
        }
    }
}

pub type Check = Result<(), Violation>;

// --- the phrase layer ----------------------------------------------------

/// The upper bound on the compiled size of one category's pattern. Phrases
/// are escaped literals, so matching runs in linear time whatever a pack
/// holds; this bounds the memory of building the pattern.
const PATTERN_SIZE_LIMIT: usize = 4 << 20;

/// The phrase lists of one language, compiled. One pattern per category, built
/// from escaped literals, so a pack can neither break a pattern nor make
/// matching slow.
#[derive(Debug, Default)]
pub struct Phrases {
    preamble: Option<Regex>,
    question_comment: Option<Regex>,
    summary: Option<Regex>,
    hedging: Option<Regex>,
    forward_reference: Option<Regex>,
    meta: Option<Regex>,
    apology: Option<Regex>,
    filler: Option<Regex>,
    spelling: Option<Regex>,
}

/// Builds one category's pattern.
///
/// Every phrase matches case-insensitively. Whitespace inside a phrase matches
/// any run of whitespace, so a line break does not hide it. The pattern only
/// finds candidates; whether a candidate is a whole word is decided by
/// `find_phrase`, with the same word boundaries the word count uses.
///
/// The phrase itself is captured, so the violation can name it.
///
/// The longest phrase comes first. The regex takes the first alternative that
/// matches at a position, and `find_phrase` rejects it there if it ends inside
/// a word; a longer phrase starting at the same position, such as `waeren`
/// after `waere`, would then never be tried.
fn compile(phrases: &[String], anchored: bool) -> Result<Option<Regex>, regex::Error> {
    if phrases.is_empty() {
        return Ok(None);
    }
    let mut phrases: Vec<&String> = phrases.iter().collect();
    phrases.sort_by_key(|p| std::cmp::Reverse(p.chars().count()));
    let alternatives = phrases
        .iter()
        .map(|p| {
            p.split_whitespace()
                .map(regex::escape)
                .collect::<Vec<_>>()
                .join(r"\s+")
        })
        .collect::<Vec<_>>()
        .join("|");
    let lead = if anchored { r"^\s*" } else { "" };
    RegexBuilder::new(&format!(r"{lead}(?P<phrase>{alternatives})"))
        .case_insensitive(true)
        .size_limit(PATTERN_SIZE_LIMIT)
        .build()
        .map(Some)
}

/// Whether a list compiles within `PATTERN_SIZE_LIMIT`. The size depends on
/// the script as much as on the length: case folding gives a Greek letter
/// more variants than a Latin one. The config checks every user list with
/// this, so a list that is too large is reported instead of disabling its
/// rule. The anchored form is the larger one, so it decides for both.
pub fn fits(phrases: &[String]) -> bool {
    compile(phrases, true).is_ok()
}

impl Phrases {
    /// Compiles the lists of one or more languages. Several languages are
    /// merged list by list, which the tests use to check both shipped packs at
    /// once.
    pub fn compile(lists: &[&PhraseLists]) -> Phrases {
        // The user's lists passed `fits` when the config was loaded, and the
        // shipped ones are covered by the tests, so a failure here cannot
        // occur outside tests that merge several languages.
        let build = |list: Vec<String>, anchored| compile(&list, anchored).ok().flatten();
        let gather = |pick: fn(&PhraseLists) -> &Option<Vec<String>>| -> Vec<String> {
            lists
                .iter()
                .flat_map(|l| pick(l).iter().flatten().cloned())
                .collect()
        };
        Phrases {
            preamble: build(gather(|l| &l.preamble), true),
            question_comment: build(gather(|l| &l.question_comment), true),
            summary: build(gather(|l| &l.summary), true),
            hedging: build(gather(|l| &l.hedging), false),
            forward_reference: build(gather(|l| &l.forward_reference), false),
            meta: build(gather(|l| &l.meta), false),
            apology: build(gather(|l| &l.apology), false),
            filler: build(gather(|l| &l.filler), false),
            spelling: build(gather(|l| &l.spelling), false),
        }
    }

    /// No phrase rules at all, for a language without a pack.
    pub fn none() -> Phrases {
        Phrases::default()
    }
}

/// Rejects `text` when `pattern` finds a phrase in it, naming that phrase.
fn reject(pattern: Option<&Regex>, text: &str, message: impl Fn(&str) -> String) -> Check {
    match pattern.and_then(|re| find_phrase(re, text)) {
        None => Ok(()),
        Some(found) => Err(Violation {
            message: message(found),
            notes: Vec::new(),
        }),
    }
}

/// The first candidate that starts and ends on a word boundary.
///
/// The boundaries come from the word segmenter, so a phrase is a whole word in
/// every script: `maybe` is not found in `maybeify`, and in Chinese `可能`
/// (maybe) is not found in `可能性` (possibility), which needs a dictionary to
/// tell apart. They are computed only once a candidate turns up, so text
/// without one costs no segmentation.
fn find_phrase<'t>(pattern: &Regex, text: &'t str) -> Option<&'t str> {
    let mut boundaries: Option<Vec<usize>> = None;
    let mut from = 0;
    while let Some(found) = pattern.captures_at(text, from)?.name("phrase") {
        let at = boundaries.get_or_insert_with(|| word_boundaries(text));
        if at.binary_search(&found.start()).is_ok() && at.binary_search(&found.end()).is_ok() {
            return Some(found.as_str());
        }
        // Not a whole word here. Look again one character further on, since a
        // candidate starting inside this one may still be a whole word.
        from = found.start() + text[found.start()..].chars().next()?.len_utf8();
    }
    None
}

/// A lead-in delays the conclusion by a clause. BLUF puts it in position one.
/// Only the opening words count, after headings and markup are gone.
pub fn no_preamble(phrases: &Phrases, value: &str) -> Check {
    let opening = first_prose_line(value);
    reject(phrases.preamble.as_ref(), &opening, |p| {
        format!("starts with the preamble \"{p}\"; put the conclusion in the first sentence")
    })?;
    reject(phrases.question_comment.as_ref(), &opening, |p| {
        format!("starts by commenting on the question (\"{p}\") instead of answering it")
    })?;
    reject(phrases.summary.as_ref(), &opening, |p| {
        format!("announces a summary with \"{p}\"; in BLUF the first sentence is the summary")
    })
}

/// Hedging turns a finding into a maybe. Uncertainty belongs in `confidence`
/// and `caveats`, where it is explicit and cannot blur the claim.
pub fn no_hedging(phrases: &Phrases, value: &str) -> Check {
    reject(phrases.hedging.as_ref(), &strip_markdown(value), |p| {
        format!(
            "hedges the claim with \"{p}\"; state it plainly and put the doubt in `confidence` or `caveats`"
        )
    })
}

/// The bottom line has to stand alone. A pointer forward breaks that.
pub fn self_contained(phrases: &Phrases, value: &str) -> Check {
    reject(
        phrases.forward_reference.as_ref(),
        &strip_markdown(value),
        |p| format!("refers to later sections with \"{p}\"; the bottom line must stand on its own"),
    )
}

/// Talking about the answer is not answering, and an apology is regret, not
/// content.
pub fn no_meta(phrases: &Phrases, value: &str) -> Check {
    let prose = strip_markdown(value);
    reject(phrases.meta.as_ref(), &prose, |p| {
        format!("comments on the answer itself (\"{p}\") instead of stating content")
    })?;
    reject(phrases.apology.as_ref(), &prose, |p| {
        format!("contains an apology (\"{p}\"); state the correction, not the regret")
    })
}

/// Filler survives deletion without loss.
pub fn no_filler(phrases: &Phrases, value: &str) -> Check {
    reject(phrases.filler.as_ref(), &strip_markdown(value), |p| {
        format!("contains the filler \"{p}\"; delete the phrase and the sentence still holds")
    })
}

/// A language written in a substitute spelling, such as German with `ue` for
/// `ü`. The pack lists the words that give it away.
pub fn no_substitute_spelling(phrases: &Phrases, value: &str) -> Check {
    reject(phrases.spelling.as_ref(), &strip_markdown(value), |p| {
        format!("writes \"{p}\" in a substitute spelling; use the letters of the language")
    })
}

// --- the universal layer -------------------------------------------------

// A bottom line that asks something has not concluded anything. The set
// covers the question marks of the major scripts: Latin, full-width CJK,
// Arabic, Greek (U+037E, which looks like a semicolon) and Ethiopic.
re!(QUESTION, r"[?\u{FF1F}\u{061F}\u{037E}\u{1367}]\s*$");

pub fn is_statement(value: &str) -> Check {
    if QUESTION.is_match(&strip_markdown(value)) {
        Err(Violation::new(
            "ends in a question; the bottom line must assert something",
        ))
    } else {
        Ok(())
    }
}

// Em- and en-dashes used to join clauses. The house style calls for a comma, a
// colon, parentheses, or a shorter sentence.
//
// Only the clause-joining use is rejected, which is the one that reads as
// machine prose. Two legitimate uses stay allowed, so no exclude list is
// needed:
//
//   ranges and relations   "Zeilen 10-20", "Client-Server"  (no spaces around)
//   quoted foreign text    inside `code` or a fenced block  (stripped earlier)
//
// The distinction is the surrounding whitespace: a range binds tight, a clause
// separator floats. An em-dash is rejected either way.
re!(EM_DASH, "\u{2014}");
re!(CLAUSE_EN_DASH, r"\s\u{2013}|\u{2013}\s");

pub fn no_clause_dashes(value: &str) -> Check {
    let prose = strip_markdown(value);
    if EM_DASH.is_match(&prose) {
        return Err(Violation::new(
            "contains an em-dash; use a comma, a colon, parentheses, or two sentences",
        ));
    }
    if CLAUSE_EN_DASH.is_match(&prose) {
        return Err(Violation::new(
            "joins clauses with an en-dash; use a comma, a colon, parentheses, or two sentences",
        ));
    }
    Ok(())
}

/// Long sentences hide the point. The limit is generous on purpose: it catches
/// runaway constructions, not ordinary prose.
pub fn sentence_length(limit: usize, value: &str) -> Check {
    match crate::prose::sentence_word_counts(value)
        .into_iter()
        .find(|&n| n > limit)
    {
        None => Ok(()),
        Some(n) => Err(Violation::with_note(
            format!("a sentence runs {n} words, limit is {limit}"),
            "Split it. Long sentences bury the point.".to_string(),
        )),
    }
}

/// Enforces a prose word budget. Budgets count prose, not markup: a table or a
/// code block should not consume the allowance, since the reader's effort
/// scales with the words.
pub fn max_words(limit: usize, value: &str) -> Check {
    let n = prose_word_count(value);
    if n <= limit {
        Ok(())
    } else {
        Err(Violation::with_note(
            format!("{n} words, limit is {limit}"),
            format!("Cut {} of them.", n - limit),
        ))
    }
}

/// A field that must not be empty or whitespace only. This looks at the raw
/// text, not the prose: a field holding nothing but a code block has content,
/// and one holding only "..." is left to the placeholder rule.
pub fn non_empty(value: &str) -> Check {
    if value.trim().is_empty() {
        Err(Violation::new("must not be empty"))
    } else {
        Ok(())
    }
}

re!(PLACEHOLDER, r"^[\s.\u{2026}_-]*$");

/// The template shipped by the prompt hook uses "..." as a placeholder. Copying
/// it verbatim satisfies every other rule: it is one word, it hedges nothing,
/// it asks nothing. This rejects the unfilled template.
pub fn filled_in(value: &str) -> Check {
    if PLACEHOLDER.is_match(value) {
        Err(Violation::with_note(
            "is still the placeholder".to_string(),
            "Write the actual content, or drop the field.".to_string(),
        ))
    } else {
        Ok(())
    }
}

/// The full BLUF rule set. Applies to the bottom line, where burying the point
/// does the most damage. `sentence_limit` is the configured sentence length.
pub fn bluf(phrases: &Phrases, sentence_limit: usize, value: &str) -> Check {
    no_preamble(phrases, value)?;
    is_statement(value)?;
    self_contained(phrases, value)?;
    no_hedging(phrases, value)?;
    no_meta(phrases, value)?;
    no_filler(phrases, value)?;
    no_clause_dashes(value)?;
    no_substitute_spelling(phrases, value)?;
    sentence_length(sentence_limit, value)
}

/// Supporting prose may qualify and reference, but must not waffle or apologise.
pub fn prose_rules(phrases: &Phrases, sentence_limit: usize, value: &str) -> Check {
    no_meta(phrases, value)?;
    no_filler(phrases, value)?;
    no_clause_dashes(value)?;
    no_substitute_spelling(phrases, value)?;
    sentence_length(sentence_limit, value)
}

/// Evidence entries state facts, so hedging is out. Filler, apologies and the
/// punctuation rules apply as everywhere else.
pub fn evidence_rules(phrases: &Phrases, value: &str) -> Check {
    no_hedging(phrases, value)?;
    no_meta(phrases, value)?;
    no_filler(phrases, value)?;
    no_clause_dashes(value)?;
    no_substitute_spelling(phrases, value)
}

/// Caveats may hedge by nature. Filler, apologies and the punctuation rules
/// apply as everywhere else.
pub fn caveat_rules(phrases: &Phrases, value: &str) -> Check {
    no_meta(phrases, value)?;
    no_filler(phrases, value)?;
    no_clause_dashes(value)?;
    no_substitute_spelling(phrases, value)
}
