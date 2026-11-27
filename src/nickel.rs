//! Evaluating a Nickel record and reporting what went wrong.
//!
//! Nickel's rendered diagnostics are text meant for a terminal. Cutting them
//! down by pattern loses information: dropping every line with a leading
//! digit strips the source line numbers, but also the informative line, which
//! often opens with a number ("299 words, limit is 200").
//!
//! Here the diagnostics arrive as `Diagnostic` structs through the public
//! `IntoDiagnostics` trait, with the headline in `message` and the instruction
//! in `notes`. No regex, no locale dependency, no `head -4`.
//!
//! `report(..., ErrorFormat::Json, ...)` looks like the obvious route, but
//! `report_with` writes JSON to stderr unconditionally (`report.rs:128`),
//! ignoring its writer argument. `IntoDiagnostics` is public and gives the same
//! data without the detour.

use crate::answer::Answer;
use nickel_lang_core::error::{Error as NickelError, IntoDiagnostics, LabelStyle};
use nickel_lang_core::eval::cache::CacheImpl;
use nickel_lang_core::identifier::LocIdent;
use nickel_lang_core::program::{FieldPath, Program, ProgramBuilder};
use std::fmt::Write as _;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

/// The name Nickel gives the in-memory source in its messages. It is no path,
/// so nothing on disk is read or written, and the angle brackets keep it from
/// looking like a file the writer could open.
const SOURCE_NAME: &str = "<answer>";

/// What went wrong while turning source text into an `Answer`.
#[derive(Debug)]
pub enum EvalError {
    /// The source did not parse. `offset` is a byte offset into the source.
    Parse {
        message: String,
        offset: Option<usize>,
    },
    /// The record parsed but evaluation failed: a broken contract, a missing
    /// field, an unknown enum tag.
    Contract { message: String, notes: Vec<String> },
    /// The evaluation ran longer than `EVAL_TIMEOUT` and was abandoned.
    Timeout,
}

impl EvalError {
    /// The diagnosis as the writer should see it: the rule and the
    /// instruction, nothing about schema internals or source names.
    pub fn diagnosis(&self) -> String {
        match self {
            EvalError::Parse { message, .. } => message.clone(),
            EvalError::Contract { message, notes } => {
                let mut out = message.clone();
                for note in notes {
                    out.push('\n');
                    out.push_str(note);
                }
                out
            }
            EvalError::Timeout => format!(
                "evaluation took longer than {} s and was stopped",
                EVAL_TIMEOUT.as_secs()
            ),
        }
    }
}

/// Evaluates Nickel source text and deserialises the result into an `Answer`.
///
/// The source is evaluated from memory. The record's `import` of the schema
/// uses an absolute path, so it resolves without a file of its own.
///
/// The prose rules do not run here; `Answer::validate` applies them to the
/// returned record.
pub fn eval_answer(source: &str) -> Result<Answer, EvalError> {
    let json = eval_json(source, SOURCE_NAME)?;
    serde_json::from_value(json).map_err(|e| EvalError::Contract {
        message: e.to_string(),
        notes: Vec::new(),
    })
}

/// How long one evaluation may run. Nickel is Turing-complete, so a language
/// pack or a config with a runaway recursion would otherwise hang the hook
/// until Claude Code kills it. Well below the 5 s the display hook is
/// registered with.
pub const EVAL_TIMEOUT: Duration = Duration::from_secs(2);

/// The stack of the evaluation thread. Nickel walks the source recursively,
/// and a spawned thread gets 2 MiB by default, which a nested array of a few
/// hundred levels overflows. A stack overflow aborts the process, so the hook
/// would crash instead of failing open. The memory is reserved, not used, so
/// a generous size costs nothing until the evaluation needs it.
const EVAL_STACK: usize = 64 << 20;

/// Evaluates Nickel source text from memory and exports the result as JSON.
///
/// `name` is what Nickel calls the source in its messages. The answer uses a
/// name that is no path; the config passes its file path, which the reader of
/// that error does control.
pub fn eval_json(source: &str, name: &str) -> Result<serde_json::Value, EvalError> {
    eval_field(source, name, None)
}

/// Evaluates Nickel source text and exports `field`, or the whole result when
/// `field` is `None`, as JSON.
///
/// Exporting one field matters when the result holds functions: they cannot
/// be exported, and the field that carries the verdict does not contain any.
///
/// The evaluation runs on its own thread and is abandoned after
/// `EVAL_TIMEOUT`. The thread keeps running, but the hook process exits right
/// after it answers, which ends it.
///
/// Everything is one source. `nickel-lang-core` 0.19 can merge several
/// inputs, but panics when one of them is JSON (`fixpoint.rs:55`), and the
/// release profile turns a panic into an abort.
pub fn eval_field(
    source: &str,
    name: &str,
    field: Option<&'static str>,
) -> Result<serde_json::Value, EvalError> {
    let (source, name) = (source.to_string(), name.to_string());
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(EVAL_STACK)
        .spawn(move || {
            let _ = tx.send(eval_now(source, name, field));
        })
        .map_err(|e| EvalError::Contract {
            message: format!("could not start the evaluation: {e}"),
            notes: Vec::new(),
        })?;
    match rx.recv_timeout(EVAL_TIMEOUT) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(EvalError::Timeout),
        Err(RecvTimeoutError::Disconnected) => Err(EvalError::Contract {
            message: "evaluation stopped without a result".to_string(),
            notes: Vec::new(),
        }),
    }
}

fn eval_now(
    source: String,
    name: String,
    field: Option<&'static str>,
) -> Result<serde_json::Value, EvalError> {
    let build_error = |e: &dyn std::fmt::Display| EvalError::Contract {
        message: e.to_string(),
        notes: Vec::new(),
    };

    let mut builder = ProgramBuilder::new().add_source_string(source, name);
    if let Some(field) = field {
        builder = builder.with_field_path(FieldPath(vec![LocIdent::from(field)]));
    }
    let mut prog: Program<CacheImpl> = builder.build().map_err(|e| build_error(&e))?;

    let term = match prog.eval_full_for_export() {
        Ok(term) => term,
        Err(err) => return Err(convert(err, &mut prog)),
    };

    serde_json::to_value(&term).map_err(|e| build_error(&e))
}

/// Writes `text` as a Nickel string literal that evaluates to exactly `text`.
///
/// Every character except ASCII letters, digits and the space is written as a
/// `\u{..}` escape. No quote, backslash or percent sign survives, so the text
/// can neither end the literal nor start an interpolation, whatever it holds.
pub fn string_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == ' ' {
            out.push(c);
        } else {
            let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
        }
    }
    out.push('"');
    out
}

/// Turns a Nickel error into a diagnosis the writer can act on.
///
/// A parse error is a different failure from a broken contract: it has no rule
/// to name, and its position is the only thing that locates it. The two are
/// told apart before the diagnostics are built, since the error type says so
/// outright.
fn convert(err: NickelError, prog: &mut Program<CacheImpl>) -> EvalError {
    let is_parse = matches!(err, NickelError::ParseErrors(_));
    let mut files = prog.files();
    let diagnostics = err.into_diagnostics(&mut files);

    let Some(first) = diagnostics.into_iter().next() else {
        return EvalError::Contract {
            message: "evaluation failed without a diagnostic".to_string(),
            notes: Vec::new(),
        };
    };

    if is_parse {
        // The primary label points at the offending token. Nickel puts the
        // file path into the message, which names nothing the writer controls,
        // so only the offset is kept.
        let offset = first
            .labels
            .iter()
            .find(|l| l.style == LabelStyle::Primary)
            .map(|l| l.range.start);
        EvalError::Parse {
            message: strip_path(&first.message),
            offset,
        }
    } else {
        EvalError::Contract {
            message: first.message,
            notes: first.notes,
        }
    }
}

/// Removes the source name from a parse message.
///
/// Nickel writes "unexpected end of file when parsing <answer>". The name
/// identifies nothing the writer wrote, so the clause is cut at the word that
/// introduces it.
fn strip_path(message: &str) -> String {
    match message.find(" when parsing ") {
        Some(i) => message[..i].to_string(),
        None => message.to_string(),
    }
}

/// Translates a byte offset into a 1-based line and column.
///
/// Nickel reports byte offsets, and a field holding German prose or any other
/// non-ASCII text puts multi-byte characters in front of the error. Slicing on
/// such an offset would panic, so it is walked back to the nearest character
/// boundary first.
pub fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(source.len());
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let head = &source[..offset];
    let line = head.matches('\n').count() + 1;
    let col = head
        .rsplit('\n')
        .next()
        .map_or(1, |last| last.chars().count() + 1);
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_counts_from_one() {
        assert_eq!(line_col("abc", 0), (1, 1));
        assert_eq!(line_col("abc", 2), (1, 3));
    }

    #[test]
    fn line_col_advances_past_newlines() {
        assert_eq!(line_col("ab\ncd", 3), (2, 1));
        assert_eq!(line_col("ab\ncd", 5), (2, 3));
    }

    #[test]
    fn line_col_clamps_to_the_end() {
        assert_eq!(line_col("ab", 99), (1, 3));
    }

    // Nickel reports byte offsets. A field holding German prose puts multi-byte
    // characters in front of the error, and an offset landing inside one used to
    // panic on the slice, taking the hook down instead of reporting the error.
    #[test]
    fn line_col_survives_an_offset_inside_a_character() {
        assert_eq!(line_col("ä", 1), (1, 1));
        assert_eq!(line_col("aäb", 2), (1, 2));
        assert_eq!(line_col("Zeile\nüber", 7), (2, 1));
    }

    #[test]
    fn strip_path_drops_the_file_name() {
        assert_eq!(
            strip_path("unexpected end of file when parsing <answer>"),
            "unexpected end of file"
        );
    }

    #[test]
    fn a_string_literal_round_trips_anything() {
        let text = "Zitat: \"%{std.fail_with \\\"x\\\"}\" und m%\"…\"% \\ ende\nzeile ü 日本";
        let json = eval_json(&string_literal(text), "<test>").unwrap();
        assert_eq!(json, serde_json::Value::String(text.to_string()));
    }

    // The evaluation thread used to have the default 2 MiB stack, and a
    // nested array of 600 levels overflowed it and aborted the process.
    #[test]
    fn a_deeply_nested_value_does_not_overflow_the_stack() {
        let depth = 2000;
        let source = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        assert!(eval_json(&source, "<test>").is_ok());
    }

    #[test]
    fn strip_path_keeps_a_message_without_one() {
        assert_eq!(strip_path("unexpected token"), "unexpected token");
    }
}
