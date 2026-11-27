//! Extracting and evaluating a ```nickel block.

use crate::answer::{Answer, FieldViolation};
use crate::config::Config;
use crate::nickel::{eval_answer, line_col, EvalError};
use std::fmt::Write as _;

/// Why a block could not be turned into a valid answer.
#[derive(Debug)]
pub enum BlockError {
    /// Nickel could not evaluate the source.
    Eval(EvalError),
    /// The record evaluated but breaks a prose rule or a word budget.
    Rule(FieldViolation),
}

impl BlockError {
    /// The full text handed to Claude when a turn is blocked.
    ///
    /// A parse error is a different failure from a broken contract, and it
    /// carries no rule to name. The common cause is a field that quotes
    /// Nickel's own multiline terminator, which closes the field early and
    /// leaves the rest of the prose to be read as code. That cause is named
    /// only when the source actually shows it: an unconditional explanation
    /// sent writers escalating `m%` to `m%%` to `m%%%` against a defect that
    /// lived in the extractor.
    pub fn reason(&self, source: &str) -> String {
        match self {
            BlockError::Rule(violation) => violation.to_string(),
            BlockError::Eval(err) => {
                let mut out = err.diagnosis();
                if let EvalError::Parse { offset, .. } = err {
                    out.push_str("\n\nThis is a parse error, not a contract violation.");
                    if let Some(offset) = offset {
                        let (line, col) = line_col(source, *offset);
                        let culprit: String = source
                            .lines()
                            .nth(line - 1)
                            .unwrap_or("")
                            .chars()
                            .take(160)
                            .collect();
                        let _ = write!(
                            out,
                            " The block breaks at line {line}, column {col}:\n\n  {culprit}"
                        );
                    }
                    if let Some(hint) = parse_hint(source) {
                        out.push_str("\n\n");
                        out.push_str(hint);
                    }
                }
                out
            }
        }
    }
}

/// Names the cause of a parse error, when the source shows one.
///
/// Two causes are distinguishable by looking at the source. A field that quotes
/// the multiline terminator closes early; a bare fence left inside a field
/// means the block was cut short and the source is a fragment. When neither
/// shows, no explanation is offered: a confident diagnosis of the wrong cause
/// costs more turns than none at all.
fn parse_hint(source: &str) -> Option<&'static str> {
    let mut strings = StringScan::default();
    let mut fence_in_field = false;
    let mut early_close = false;

    for line in source.lines() {
        if strings.inside() && line.trim() == "```" {
            fence_in_field = true;
        }
        if let Some(rest) = strings.feed(line) {
            if !continues_as_code(rest) {
                early_close = true;
            }
        }
    }

    if fence_in_field {
        return Some(
            "A bare ``` line inside a field ends the block early, so the record arrives\n\
             half-written. Write code examples as indented blocks, four spaces, or tag the\n\
             fence with its language.",
        );
    }

    // An unclosed string at the end of the source is the same failure seen from
    // the other side: a field closed early, so a later terminator opened one
    // that never closes.
    if early_close || strings.inside() {
        return Some(
            "The cause is a field that quotes Nickel's multiline terminator, which\n\
             closes that field early. Backticks do not help, since the parser runs before\n\
             any Markdown is read. Open the field with more percent signs than the text\n\
             contains, so m%%\"...\"%% holds a literal m%\"...\"%.",
        );
    }

    None
}

/// Whether the text after a multiline terminator reads as code.
///
/// A field closes on its own line, on a line that ends the record, or before
/// a comment, an operator or the `in` of a `let`. Anything else is prose,
/// which means the terminator was part of the prose and closed the field in
/// the middle of it.
fn continues_as_code(rest: &str) -> bool {
    let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || ",}])".contains(c));
    rest.is_empty()
        || rest.starts_with('#')
        || rest.trim_end() == "in"
        || rest.starts_with(|c: char| "+|&@=!<>".contains(c))
}

/// Tracks whether a scan sits inside a Nickel multiline string.
///
/// A multiline string opens with `m` followed by a run of one or more `%` and
/// a `"`, and closes with a `"` followed by the same run of `%`. A shorter run
/// does not close it: `m%%"..."%%` is not closed by a plain `"%`, which is
/// exactly the escape a field uses to hold a literal `m%"`. A longer run is a
/// lexer error in Nickel, so the scanner treats it as a close and leaves the
/// error to the parser.
///
/// The scanner is deliberately small. Nickel's own lexer is not reachable from
/// here, and the extractor only has to answer one question: does this line's
/// content belong to a string. Single-quoted strings are ignored, since they
/// cannot span lines and therefore cannot hide a fence. A `#` comment ends the
/// scan of its line, so an opener quoted in a comment opens nothing.
#[derive(Default)]
struct StringScan {
    /// The length of the `%` run of the open multiline string, if any.
    open: Option<usize>,
}

impl StringScan {
    fn inside(&self) -> bool {
        self.open.is_some()
    }

    /// Feeds one line to the scanner.
    ///
    /// Returns what followed the terminator when a multiline string closed on
    /// this line, which `parse_hint` reads to tell an intended terminator from
    /// one that was quoted inside the prose.
    fn feed<'a>(&mut self, line: &'a str) -> Option<&'a str> {
        let bytes = line.as_bytes();
        let mut closed_at = None;
        let mut i = 0;
        while i < bytes.len() {
            match self.open {
                Some(run) => {
                    // Closing: a quote followed by at least `run` percent signs.
                    if bytes[i] == b'"' {
                        let percents = bytes[i + 1..].iter().take_while(|b| **b == b'%').count();
                        if percents >= run {
                            self.open = None;
                            i += 1 + run;
                            closed_at = Some(i);
                            continue;
                        }
                    }
                    i += 1;
                }
                None => {
                    if bytes[i] == b'#' {
                        break;
                    }
                    // Opening: `m`, a run of percent signs, then a quote. The
                    // `m` must not be part of a longer identifier.
                    if bytes[i] == b'm' && (i == 0 || !is_ident_byte(bytes[i - 1])) {
                        let percents = bytes[i + 1..].iter().take_while(|b| **b == b'%').count();
                        if percents > 0 && bytes.get(i + 1 + percents) == Some(&b'"') {
                            self.open = Some(percents);
                            i += percents + 2;
                            continue;
                        }
                    }
                    // A single-line string: skip its body so a `m%"` inside it
                    // does not open a multiline string that never closes.
                    if bytes[i] == b'"' {
                        i += 1;
                        while i < bytes.len() && bytes[i] != b'"' {
                            i += if bytes[i] == b'\\' { 2 } else { 1 };
                        }
                    }
                    i += 1;
                }
            }
        }
        // The offset is a byte index, and a field holding German prose puts
        // multi-byte characters in front of it. Slicing on it directly would
        // panic, so it is walked forward to the next character boundary.
        closed_at.map(|at| {
            let mut at = at.min(line.len());
            while at < line.len() && !line.is_char_boundary(at) {
                at += 1;
            }
            &line[at..]
        })
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'\''
}

/// Extracts the first ```nickel block from a message, by the rules of
/// `BlockScan`. Returns `None` when the message has no such block.
pub fn extract(text: &str) -> Option<String> {
    let mut scan = BlockScan::default();
    let mut collected: Vec<&str> = Vec::new();

    for line in text.lines() {
        match scan.feed(line) {
            Line::Outside | Line::Open => {}
            Line::Content => collected.push(line),
            Line::Close => return Some(collected.join("\n")),
        }
    }

    // An unterminated block is still worth evaluating: the writer will get a
    // parse error naming the real problem instead of "no block found".
    scan.inside().then(|| collected.join("\n"))
}

/// What one line is to the ```nickel block around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    /// Text outside any block.
    Outside,
    /// The opening ```nickel fence.
    Open,
    /// A line of the record.
    Content,
    /// The bare fence that ends the block.
    Close,
}

/// Finds ```nickel blocks line by line. The extractor and the display hook
/// both use it, so the two agree on where a block starts and ends.
///
/// Only a bare fence closes the block, so a nested ```rust inside it does not
/// end it early. A bare fence inside a Nickel multiline string does not close
/// it either: fields hold Markdown, and a fenced code example is prose the
/// writer put there on purpose.
#[derive(Default)]
pub struct BlockScan {
    inside: bool,
    strings: StringScan,
}

impl BlockScan {
    /// Continues a scan from saved state: whether a block is open, and the
    /// lines collected inside it so far.
    ///
    /// The display hook stores the collected text between calls, not the
    /// scanner, so the string state is rebuilt from that text.
    pub fn resume(inside: bool, collected: &str) -> BlockScan {
        let mut scan = BlockScan {
            inside,
            strings: StringScan::default(),
        };
        if inside {
            for line in collected.lines() {
                scan.strings.feed(line);
            }
        }
        scan
    }

    pub fn inside(&self) -> bool {
        self.inside
    }

    /// Classifies one line and advances the scan past it.
    pub fn feed(&mut self, line: &str) -> Line {
        let tag = line.trim();
        if !self.inside {
            if tag == "```nickel" {
                self.inside = true;
                self.strings = StringScan::default();
                return Line::Open;
            }
            return Line::Outside;
        }
        if tag == "```" && !self.strings.inside() {
            self.inside = false;
            return Line::Close;
        }
        self.strings.feed(line);
        Line::Content
    }
}

/// Evaluates a block and validates the record it produces.
pub fn evaluate(source: &str, config: &Config) -> Result<Answer, BlockError> {
    let answer = eval_answer(source).map_err(BlockError::Eval)?;
    answer.validate(config).map_err(BlockError::Rule)?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_block() {
        let text = "prose\n```nickel\nrender {}\n```\nmore";
        assert_eq!(extract(text).as_deref(), Some("render {}"));
    }

    #[test]
    fn ignores_a_foreign_fence() {
        let text = "```bash\njust check\n```";
        assert_eq!(extract(text), None);
    }

    // An opening fence with a language tag does not close the block. Only a
    // bare ``` does, which keeps the extractor to one rule.
    #[test]
    fn a_tagged_fence_does_not_close_the_block() {
        let text = "```nickel\na = 1\n```sh\nb = 2\n```";
        let block = extract(text).expect("block");
        assert!(block.contains("```sh"));
        assert!(block.contains("b = 2"));
    }

    // A field holds Markdown, so a fenced code example inside it is prose. The
    // line-based scan used to close the block at that fence and hand Nickel a
    // fragment, which then failed with "unexpected end of file".
    #[test]
    fn a_bare_fence_inside_a_field_does_not_close_the_block() {
        let text = "```nickel\na = m%\"\n```bash\nx\n```\n\"%,\n```";
        let block = extract(text).expect("block");
        assert!(block.contains("a = m%"), "{block}");
        assert!(block.contains("\"%,"), "{block}");
    }

    // The delimiter run is matched by length, mirroring `prose.rs`: a string
    // opened with `m%%"` is not closed by a bare `"%`.
    #[test]
    fn a_longer_delimiter_run_is_matched_by_length() {
        let text = "```nickel\na = m%%\"\nm%\"innen\"%\n```\n\"%%,\n```";
        let block = extract(text).expect("block");
        assert!(block.contains("m%\"innen\"%"), "{block}");
        assert!(block.contains("\"%%,"), "{block}");
    }

    // Once the field closes, a bare fence is the block's own terminator again.
    #[test]
    fn a_fence_after_the_field_closes_the_block() {
        let text = "```nickel\na = m%\"\n```\n\"%,\nb = 1\n```\ntrailing prose";
        let block = extract(text).expect("block");
        assert!(block.contains("b = 1"), "{block}");
        assert!(!block.contains("trailing prose"), "{block}");
    }

    // A `m%"` inside a single-line string is text, not an opener. Treating it
    // as one would leave the scanner permanently inside a string and swallow
    // the rest of the message.
    #[test]
    fn a_quoted_opener_does_not_open_a_string() {
        let text = "```nickel\na = \"schreibe m%\\\"so\\\"%\",\nb = 1\n```\nprose";
        let block = extract(text).expect("block");
        assert!(block.contains("b = 1"), "{block}");
        assert!(!block.contains("prose"), "{block}");
    }

    // `m` closes an identifier here, so `farm%"` is not an opener.
    #[test]
    fn an_identifier_ending_in_m_does_not_open_a_string() {
        let mut scan = StringScan::default();
        scan.feed("farm%\"x");
        assert!(!scan.inside());
    }

    // A comment is no string. An opener quoted in one used to leave the scanner
    // inside a string for good, so the block swallowed the rest of the message.
    #[test]
    fn an_opener_in_a_comment_does_not_open_a_string() {
        let text = "```nickel\n  # long fields use m%\"\nb = 1\n```\nprose";
        let block = extract(text).expect("block");
        assert!(block.contains("b = 1"), "{block}");
        assert!(!block.contains("prose"), "{block}");
    }

    // The display hook resumes from the text it saved. A field still open in
    // that text must keep its fences from closing the block.
    #[test]
    fn a_resumed_scan_remembers_an_open_field() {
        let mut scan = BlockScan::resume(true, "a = m%\"\nText\n");
        assert_eq!(scan.feed("```"), Line::Content);
        assert_eq!(scan.feed("\"%,"), Line::Content);
        assert_eq!(scan.feed("```"), Line::Close);
        assert!(!scan.inside());
    }

    #[test]
    fn accepts_an_unterminated_block() {
        let text = "```nickel\nrender {\n";
        assert_eq!(extract(text).as_deref(), Some("render {"));
    }

    // --- the diagnosis names the cause it can see -------------------------

    #[test]
    fn a_quoted_terminator_is_named() {
        let source = "a = m%\"\nEin Beispiel: m%\"innen\"% und danach Text.\n\"%,\n";
        let hint = parse_hint(source).expect("hint");
        assert!(hint.contains("multiline terminator"), "{hint}");
    }

    // The extractor no longer hands over a fragment, but `check` accepts a bare
    // record too, so the fence case still has to be named when it appears.
    #[test]
    fn a_bare_fence_in_a_field_is_named() {
        let source = "a = m%\"\n```\nx\n";
        let hint = parse_hint(source).expect("hint");
        assert!(hint.contains("bare ```"), "{hint}");
    }

    // A parse error with neither signature gets no explanation. Asserting the
    // terminator cause unconditionally is what sent writers escalating percent
    // signs against a defect in the extractor.
    #[test]
    fn an_unexplained_parse_error_gets_no_hint() {
        assert_eq!(parse_hint("render {\n  bottom_line = \"Kurz.\"\n"), None);
    }

    // A terminator that ends its field is not an early close, whatever
    // punctuation closes the record on the same line.
    #[test]
    fn a_regular_terminator_is_not_flagged() {
        assert_eq!(parse_hint("a = m%\"\nText.\n\"%,\nb = 1\n"), None);
        assert_eq!(parse_hint("a = m%\"\nText.\n\"% }\n"), None);
    }

    // A comment, an operator or the `in` of a `let` after the terminator is
    // code. Reading it as prose blamed the terminator for an unrelated parse
    // error and sent the writer escalating percent signs.
    #[test]
    fn code_after_a_terminator_is_not_flagged() {
        assert_eq!(parse_hint("a = m%\"\nText.\n\"%,   # details\n"), None);
        assert_eq!(parse_hint("a = m%\"\nText.\n\"% ++ b,\n"), None);
        assert_eq!(parse_hint("let a = m%\"\nText.\n\"% in\n"), None);
    }
}
