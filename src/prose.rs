//! Markdown stripping and word counting.
//!
//! Fields hold Markdown. The prose rules must see prose, not markup, or they
//! fire on things that are not writing: a variable named `maybe`, a comment
//! reading "I think this is wrong", a path containing "siehe unten".
//!
//! Code spans and fenced blocks are removed entirely, since their wording is
//! not the writer's prose. Structural markers are stripped but their text is
//! kept, because a list item is still a sentence that has to hold up.
//!
//! Ported from the v1 prototype. Every regex is compiled once at first use
//! instead of per field, which is where the 22 ms of Nickel contract time went.

use icu_segmenter::options::WordBreakInvariantOptions;
use icu_segmenter::{WordSegmenter, WordSegmenterBorrowed};
use regex::Regex;
use std::sync::LazyLock;

macro_rules! re {
    ($name:ident, $pattern:literal) => {
        static $name: LazyLock<Regex> =
            LazyLock::new(|| Regex::new($pattern).expect("static regex"));
    };
}

re!(CODE_SPAN, r"`[^`]*`");
// Images and links: keep the label, drop the target.
re!(LINK, r"!?\[([^\]]*)\]\([^)]*\)");
re!(HEADING, r"(?m)^\s*#{1,6}\s+");
re!(LIST_MARKER, r"(?m)^\s*([-*+]|\d+\.)\s+");
re!(QUOTE, r"(?m)^\s*>\s?");
// Table rows: separators and pipes carry no prose.
re!(TABLE_RULE, r"(?m)^\s*\|[-: |]*\|\s*$");
re!(PIPE, r"\|");
re!(EMPHASIS, r"(\*\*|__|\*|_|~~)");
re!(HEADING_LINE, r"(?m)^\s*#{1,6}\s+.*$");
// Latin terminators end a sentence only before whitespace or at the end, so
// "3.5" and "v1.2.3" do not split it. An abbreviation before a space, such as
// "e.g. this", does split it, which only makes the sentence shorter. The
// terminators of CJK, Devanagari, Arabic, Urdu, Ethiopic, Burmese and Khmer
// scripts are followed by no space, so they end a sentence on their own.
re!(
    SENTENCE_END,
    r"[.!?]+(\s+|$)|[\u{3002}\u{FF01}\u{FF1F}\u{0964}\u{0965}\u{061F}\u{06D4}\u{1362}\u{1367}\u{104A}\u{104B}\u{17D4}\u{17D5}]+\s*"
);
re!(SPACE_RUN, r"\s+");

/// The Thai and Lao blocks.
fn is_thai_or_lao(c: char) -> bool {
    matches!(c, '\u{0E00}'..='\u{0EFF}')
}

/// Thai and Lao have no full stop. A space ends a sentence or a clause; inside
/// a sentence words run together without one. Every run of whitespace between
/// two Thai or Lao characters becomes a line break.
///
/// A single pattern over character, space and character cannot do this: its
/// matches do not overlap, so a one-character sentence such as `ณ` would use
/// up the character the next space needs.
fn break_at_space_stops(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for run in SPACE_RUN.find_iter(s) {
        let before = s[..run.start()].chars().next_back();
        let after = s[run.end()..].chars().next();
        if before.is_some_and(is_thai_or_lao) && after.is_some_and(is_thai_or_lao) {
            out.push_str(&s[last..run.start()]);
            out.push('\n');
            last = run.end();
        }
    }
    out.push_str(&s[last..]);
    out
}

/// A fence line: its marker (a backtick or a tilde), the length of the marker
/// run, and the info string after it. `None` for a line that is no fence.
///
/// Up to three spaces of indentation are allowed, as in CommonMark. A deeper
/// indent is an indented code block, which this stripper leaves alone.
fn fence(line: &str) -> Option<(char, usize, &str)> {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return None;
    }
    let mark = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    // Both markers are ASCII, so the count is also a byte offset.
    let len = rest.chars().take_while(|c| *c == mark).count();
    (len >= 3).then(|| (mark, len, &rest[len..]))
}

/// Removes fenced blocks, keeping the prose around them.
///
/// This is a scanner rather than a regex, because the fences have to be paired
/// by length: a block opened with four backticks may contain a three-backtick
/// fence, and a closer only closes an opener that is at least as long. A lazy
/// ```` ```.*?``` ```` pattern pairs the opener with the inner fence instead,
/// which leaves the rest of the block as prose and can swallow real prose that
/// follows it.
///
/// An opener that is never closed runs to the end of the text, so an
/// interrupted message does not leak its half-written block into the prose
/// rules.
///
/// A backtick run whose info string holds another backtick is no opener, as
/// in CommonMark. It is inline code (```` ```x``` ````), and treating it as an
/// opener would strip every line after it from the prose rules.
fn strip_fenced(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut open: Option<(char, usize)> = None;
    for line in s.lines() {
        match (open, fence(line)) {
            // Inside a block: only a fence of the same marker, at least as
            // long as the opener and carrying no info string, closes it.
            (Some((mark, opener)), Some((m, len, info))) => {
                if m == mark && len >= opener && info.trim().is_empty() {
                    open = None;
                }
            }
            (Some(_), None) => {}
            (None, Some((mark, len, info))) if !(mark == '`' && info.contains('`')) => {
                open = Some((mark, len));
                // An empty line in place of the block, so the prose before and
                // after it stay separate paragraphs.
                out.push('\n');
            }
            (None, _) => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

// A line that starts a Markdown block of its own: a list item, a heading, a
// quote or a table row. Its line break ends the sentence before it.
re!(BLOCK_START, r"^\s*([-*+]\s|\d+\.\s|#{1,6}\s|>|\|)");

/// Joins soft-wrapped lines into one line per paragraph or block.
///
/// A long sentence wrapped over several lines of a field is still one
/// sentence. A blank line and the start of a new block end it.
fn join_soft_breaks(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut joinable = false;
    for line in s.lines() {
        if line.trim().is_empty() {
            out.push('\n');
            joinable = false;
            continue;
        }
        if joinable && !BLOCK_START.is_match(line) {
            out.push(' ');
        } else if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
        joinable = true;
    }
    out
}

/// Removes Markdown markup, keeping the prose inside it.
pub fn strip_markdown(s: &str) -> String {
    let s = strip_fenced(s);
    let s = CODE_SPAN.replace_all(&s, " ");
    let s = LINK.replace_all(&s, " ");
    let s = HEADING.replace_all(&s, " ");
    let s = LIST_MARKER.replace_all(&s, " ");
    let s = QUOTE.replace_all(&s, " ");
    let s = TABLE_RULE.replace_all(&s, " ");
    let s = PIPE.replace_all(&s, " ");
    EMPHASIS.replace_all(&s, "").into_owned()
}

/// The word segmenter of ICU4X: Unicode word boundaries (UAX #29), plus
/// dictionaries for Chinese and Japanese and LSTM models for Thai, Lao, Khmer
/// and Burmese, which put no spaces between words. Its data is compiled into
/// the binary, so building it costs microseconds and no file access.
static WORDS: LazyLock<WordSegmenterBorrowed<'static>> =
    LazyLock::new(|| WordSegmenter::new_auto(WordBreakInvariantOptions::default()));

/// Counts the words in the raw string, the way a reader would in that
/// language. Punctuation and whitespace between words do not count.
///
/// A segment counts when it holds a letter or a digit. The segmenter's own
/// word type is not used: ICU4X 2.4 marks a word that ends in a combining
/// vowel sign as no word, which drops `है` and `चलता` from Hindi and would do
/// the same to every Indic script.
pub fn word_count(s: &str) -> usize {
    let mut start = 0;
    let mut words = 0;
    for end in WORDS.segment_str(s) {
        if s[start..end].chars().any(char::is_alphanumeric) {
            words += 1;
        }
        start = end;
    }
    words
}

/// The positions between words, as byte offsets in ascending order, the start
/// and the end of the text included.
pub fn word_boundaries(s: &str) -> Vec<usize> {
    WORDS.segment_str(s).collect()
}

/// Counts prose words only, so a table or a code block does not inflate the
/// budget with markup.
pub fn prose_word_count(s: &str) -> usize {
    word_count(&strip_markdown(s))
}

/// The first line that actually carries prose.
///
/// Anchored rules apply to the opening words. Stripping a heading leaves blank
/// lines in front of it, which would otherwise let a preamble slip past a `^`
/// anchor. Headings are labels, not prose: dropping the whole line keeps a
/// preamble underneath a heading from hiding behind it.
pub fn first_prose_line(s: &str) -> String {
    let without_headings = HEADING_LINE.replace_all(s, "");
    strip_markdown(&without_headings)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Splits prose into sentences and returns the word count of each.
///
/// The terminator is dropped rather than preserved; word counts are
/// unaffected. Fenced blocks go first, otherwise a fenced block counts as one
/// giant sentence. Soft line breaks are joined before the rest of the markup
/// is stripped, since list markers and headings mark where a block begins.
pub fn sentence_word_counts(s: &str) -> Vec<usize> {
    let prose = strip_markdown(&join_soft_breaks(&strip_fenced(s)));
    let prose = break_at_space_stops(&prose);
    SENTENCE_END
        .replace_all(&prose, "\n")
        .split('\n')
        .map(word_count)
        .collect()
}
