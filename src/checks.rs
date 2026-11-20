//! Checks a language pack writes in Nickel.
//!
//! A pack may carry `checks`: a record of functions, each taking the prose of
//! one field and returning `'Ok` or `'Violation "message"`. They cover what a
//! phrase list cannot express, such as a rule about a script or a word order,
//! without the main program knowing about the language.
//!
//! One program merges the pack, the prose and the runner below. The prose is
//! written as escaped string literals (`nickel::string_literal`), so `%{` in
//! an answer stays text. Only the `result` field is exported; the checks are
//! functions and could not be.
//!
//! The pack is inserted as source text, so a pack has to be self-contained:
//! a relative `import` inside it does not resolve.
//!
//! Only the Stop hook runs this, once per turn. A failing or slow check lets
//! the answer through: a broken pack must not block every turn.

use crate::config::PackSource;
use crate::nickel::{eval_field, string_literal, EvalError};
use serde::Deserialize;

/// Applies every check to every text and returns the first violation, or
/// `null`. Checks run in the order of their names.
const RUNNER: &str = r#"{
  checks | default = {},
  texts | Array { field : String, text : String },
  result =
    let all = std.record.to_array checks in
    std.array.fold_left
      (fun found entry =>
        if found != null then
          found
        else
          std.array.fold_left
            (fun hit c =>
              if hit != null then
                hit
              else
                # Record literals are recursive in Nickel, so the bound names
                # must differ from the field names they fill.
                c.value entry.text
                |> match {
                  'Ok => null,
                  'Violation msg => { field = entry.field, check = c.field, message = msg },
                }
            )
            null
            all
      )
      null
      texts,
}
"#;

/// One piece of prose and the field it comes from.
#[derive(Debug)]
pub struct Text {
    pub field: String,
    pub text: String,
}

/// The first violation a check reported.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Hit {
    pub field: String,
    pub check: String,
    pub message: String,
}

/// Runs the pack's checks on the texts.
pub fn run(pack: &PackSource, texts: &[Text]) -> Result<Option<Hit>, EvalError> {
    let source = pack.source().map_err(|e| EvalError::Contract {
        message: format!("{}: {e}", pack.name()),
        notes: Vec::new(),
    })?;
    let entries = texts
        .iter()
        .map(|t| {
            format!(
                "{{ field = {}, text = {} }}",
                string_literal(&t.field),
                string_literal(&t.text)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let program = format!("(\n{source}\n)\n& {{ texts = [{entries}] }}\n& {RUNNER}");
    let result = eval_field(&program, &pack.name(), Some("result"))?;
    serde_json::from_value(result).map_err(|e| EvalError::Contract {
        message: e.to_string(),
        notes: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A pack on disk, the way a user writes one.
    fn pack(source: &str) -> (tempdir::Dir, PackSource) {
        let dir = tempdir::Dir::new();
        let path = dir.path().join("xx.ncl");
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(source.as_bytes()))
            .expect("write pack");
        (dir, PackSource::File(path))
    }

    fn texts(items: &[(&str, &str)]) -> Vec<Text> {
        items
            .iter()
            .map(|(field, text)| Text {
                field: field.to_string(),
                text: text.to_string(),
            })
            .collect()
    }

    const NO_DIGITS: &str = r#"{
      checks = {
        no_digits = fun text =>
          if std.string.is_match "[0-9]" text then 'Violation "spell out numbers" else 'Ok,
      },
    }"#;

    #[test]
    fn a_passing_text_gives_no_hit() {
        let (_dir, source) = pack(NO_DIGITS);
        let hit = run(&source, &texts(&[("bottom_line", "Alles gut.")])).unwrap();
        assert_eq!(hit, None);
    }

    #[test]
    fn the_first_violation_names_field_and_check() {
        let (_dir, source) = pack(NO_DIGITS);
        let hit = run(
            &source,
            &texts(&[("bottom_line", "Gut."), ("evidence[0]", "Lief 3 Mal.")]),
        )
        .unwrap()
        .expect("hit");
        assert_eq!(hit.field, "evidence[0]");
        assert_eq!(hit.check, "no_digits");
        assert_eq!(hit.message, "spell out numbers");
    }

    // The prose is data. An interpolation sequence in it must reach the check
    // as text, not be evaluated.
    #[test]
    fn prose_is_never_evaluated_as_nickel() {
        let (_dir, source) = pack(
            r#"{ checks = { echo = fun text => if std.string.contains "fail_with" text then 'Violation "seen" else 'Ok } }"#,
        );
        let hit = run(
            &source,
            &texts(&[("bottom_line", "%{std.fail_with \"x\"}")]),
        )
        .unwrap()
        .expect("hit");
        assert_eq!(hit.message, "seen");
    }

    // Nickel is Turing-complete. A runaway check is stopped, not waited for.
    #[test]
    fn a_runaway_check_times_out() {
        let (_dir, source) =
            pack("{ checks = { forever = fun text => let rec f = fun x => f x in f text } }");
        let started = std::time::Instant::now();
        let result = run(&source, &texts(&[("bottom_line", "Text.")]));
        assert!(matches!(result, Err(EvalError::Timeout)), "{result:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
    }

    /// A temporary directory that is removed with its contents when dropped.
    /// Small enough to keep here instead of pulling in a crate for tests.
    mod tempdir {
        use std::path::{Path, PathBuf};

        pub struct Dir(PathBuf);

        impl Dir {
            pub fn new() -> Dir {
                static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("n4t-checks-test-{}-{n}", std::process::id()));
                std::fs::create_dir_all(&path).expect("create temp dir");
                Dir(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
