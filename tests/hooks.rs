//! End-to-end tests for the Stop hook's error reporting.
//!
//! The hook's job is not only to block an invalid
//! record but to say why. A block whose reason names a schema path and box
//! drawing instead of the broken rule costs a turn and teaches nothing, so the
//! reason text is what is asserted here.

use n4t::config::{Config, UserFile, UserPack};
use n4t::hooks::{parse_input, validate};
use serde_json::json;

/// Runs the Stop hook on a record and returns the blocking reason, or `None`
/// when the hook let the turn through.
fn reason_for(record: &str) -> Option<String> {
    let text = format!("```nickel\n{record}\n```");
    reason_for_message(&text, false)
}

fn reason_for_message(text: &str, stop_hook_active: bool) -> Option<String> {
    let payload = json!({
        "hook_event_name": "Stop",
        "stop_hook_active": stop_hook_active,
        "last_assistant_message": text,
    })
    .to_string();

    match validate::verdict_with(&parse_input(&payload), &Config::default()) {
        validate::Verdict::Pass => None,
        validate::Verdict::Block(reason) => Some(reason),
    }
}

/// Builds a render{...} record around the given field assignments.
fn record_with(fields: &str) -> String {
    format!(
        "let {{ render, .. }} = import \"{}/render.ncl\" in\nrender {{\n{fields}\n}}\n",
        n4t::hooks::inject::schema_dir().display()
    )
}

// --- the reason must name the broken rule --------------------------------

/// The em-dash rule. This is the case the reporter saw, and it worked; it is
/// kept so a change to the reporting cannot silently break the case that did.
#[test]
fn em_dash_violation_names_the_rule() {
    let r = reason_for(&record_with(
        "  bottom_line = \"Kurzer Satz.\",\n  \
         supporting_detail = \"Ein Satz \u{2014} mit Gedankenstrich.\",\n  \
         confidence = 'high,",
    ))
    .expect("blocked");
    assert!(r.contains("em-dash"), "{r}");
}

/// The word budget. Its violation line opens with a digit ("299 words, limit is
/// 200"). A trimmer that drops every line starting with a number, to strip
/// Nickel's source line numbers, throws this one away too and leaves the
/// writer with a schema path and box drawing in its place.
#[test]
fn word_budget_violation_names_the_count() {
    let long = "Wort wort wort wort. ".repeat(65);
    let r = reason_for(&record_with(&format!(
        "  bottom_line = \"Kurzer Satz.\",\n  \
         supporting_detail = \"{long} Ende.\",\n  \
         confidence = 'high,"
    )))
    .expect("blocked");
    assert!(
        r.contains("words, limit is 200"),
        "reason did not name the count: {r}"
    );
}

/// The sentence-length rule. Same shape: the informative line starts with "a
/// sentence runs 43 words", and its note carries the instruction.
#[test]
fn sentence_length_violation_names_the_count() {
    let sentence = "Wort ".repeat(42);
    let r = reason_for(&record_with(&format!(
        "  bottom_line = \"{sentence} Ende.\",\n  confidence = 'high,"
    )))
    .expect("blocked");
    assert!(r.contains("a sentence runs"), "{r}");
    assert!(r.contains("words, limit is 40"), "{r}");
}

// --- the reason must not carry Nickel's scaffolding ----------------------

fn probe_reasons() -> Vec<String> {
    let long = "Wort wort wort wort. ".repeat(65);
    vec![
        reason_for(&record_with(
            "  bottom_line = \"Kurz.\",\n  \
             supporting_detail = \"Ein Satz \u{2014} mit Strich.\",\n  \
             confidence = 'high,",
        ))
        .expect("blocked"),
        reason_for(&record_with(&format!(
            "  bottom_line = \"Kurz.\",\n  \
             supporting_detail = \"{long} Ende.\",\n  \
             confidence = 'high,"
        )))
        .expect("blocked"),
    ]
}

#[test]
fn reasons_have_no_box_drawing() {
    for r in probe_reasons() {
        assert!(
            !r.chars().any(|c| "\u{2500}\u{2502}\u{250c}\u{2510}\u{2514}\u{2518}\u{251c}\u{2524}\u{252c}\u{2534}\u{253c}\u{256d}\u{256e}\u{256f}\u{2570}".contains(c)),
            "box drawing in: {r}"
        );
    }
}

#[test]
fn reasons_have_no_schema_path() {
    for r in probe_reasons() {
        assert!(!r.contains("answer.ncl"), "schema path in: {r}");
        assert!(!r.contains("render.ncl"), "schema path in: {r}");
    }
}

#[test]
fn reasons_have_no_temp_path() {
    for r in probe_reasons() {
        assert!(!r.contains("/tmp/"), "temp path in: {r}");
        assert!(!r.contains("n4t-"), "temp path in: {r}");
    }
}

// --- a parse error keeps its own diagnosis -------------------------------

/// A field that quotes Nickel's multiline terminator closes early. The reason
/// must say so and must not leak the source name.
#[test]
fn parse_error_is_named_as_such() {
    let r = reason_for(&record_with(
        "  bottom_line = \"Kurz.\",\n  \
         supporting_detail = m%\"\nEin Beispiel: m%\"innen\"% und danach Text.\n\"%,\n  \
         confidence = 'high,",
    ))
    .expect("blocked");
    assert!(r.contains("parse error"), "{r}");
}

#[test]
fn parse_reason_hides_the_temp_path() {
    let r = reason_for(&record_with(
        "  bottom_line = \"Kurz.\",\n  \
         supporting_detail = m%\"\nEin Beispiel: m%\"innen\"% und danach Text.\n\"%,\n  \
         confidence = 'high,",
    ))
    .expect("blocked");
    assert!(!r.contains("/tmp/"), "{r}");
    assert!(!r.contains(".ncl"), "{r}");
}

/// The reported case: a fenced code example inside a field. The extractor used
/// to end the block at that fence and hand Nickel half a record, and the reason
/// then blamed the multiline terminator, which the payload never touches.
#[test]
fn a_fenced_example_in_a_field_passes() {
    let text = format!(
        "```nickel\n{}\n```",
        record_with(
            "  bottom_line = m%\"\nText davor.\n\n```bash\necho hallo\n```\n\nText danach.\n\"%,\n  \
             confidence = 'verified,"
        )
    );
    assert_eq!(reason_for_message(&text, false), None);
}

/// The terminator case stays diagnosed. It is a real failure with a real fix,
/// and only the unconditional blaming was wrong.
#[test]
fn the_terminator_case_keeps_its_diagnosis() {
    let r = reason_for(&record_with(
        "  bottom_line = \"Kurz.\",\n  \
         supporting_detail = m%\"\nEin Beispiel: m%\"innen\"% und danach Text.\n\"%,\n  \
         confidence = 'high,",
    ))
    .expect("blocked");
    assert!(r.contains("multiline terminator"), "{r}");
}

// --- a valid record must pass --------------------------------------------

#[test]
fn a_valid_record_is_not_blocked() {
    let r = reason_for(&record_with(
        "  bottom_line = \"Der Hook meldet die verletzte Regel.\",\n  \
         supporting_detail = \"Der Bericht behält die Diagnosezeile.\",\n  \
         evidence = [\"`cargo test` läuft durch.\"],\n  \
         confidence = 'high,",
    ));
    assert_eq!(r, None, "{r:?}");
}

// --- the original bug report ---------------------------------------------

/// The record from the first bug report. It fails reproducibly on the word
/// budget, and the reason must name that count rather than the em-dash
/// message that was shown at the time.
#[test]
fn the_original_report_names_the_budget() {
    let filler = "Wort wort wort wort. ".repeat(75);
    let r = reason_for(&record_with(&format!(
        "  bottom_line = \"Review von `pkg/tui` abgeschlossen.\",\n  \
         supporting_detail = \"{filler}\",\n  \
         confidence = 'high,"
    )))
    .expect("blocked");
    assert!(r.contains("words, limit is 200"), "{r}");
    assert!(r.contains("Cut "), "the note is missing: {r}");
    assert!(!r.contains("em-dash"), "{r}");
}

// --- structural failures -------------------------------------------------

#[test]
fn a_message_without_a_block_is_blocked() {
    let r = reason_for_message("Plain prose, no record.", false).expect("blocked");
    assert!(r.contains("no ```nickel block"), "{r}");
}

#[test]
fn a_missing_required_field_is_blocked() {
    let r = reason_for(&record_with("  confidence = 'high,")).expect("blocked");
    assert!(r.contains("bottom_line"), "{r}");
}

#[test]
fn an_unknown_enum_tag_is_blocked() {
    let r = reason_for(&record_with(
        "  bottom_line = \"Kurz.\",\n  confidence = 'ziemlich,",
    ))
    .expect("blocked");
    assert!(!r.is_empty());
}

#[test]
fn the_unfilled_template_is_blocked() {
    let r = reason_for(&record_with(
        "  bottom_line = \"...\",\n  confidence = 'high,",
    ))
    .expect("blocked");
    assert!(r.contains("placeholder"), "{r}");
}

// --- never block twice ---------------------------------------------------

/// Already inside a block-triggered rerun: never block twice, that loops.
#[test]
fn a_rerun_does_not_block_again() {
    let r = reason_for_message("Plain prose, no record.", true);
    assert_eq!(r, None);
}

#[test]
fn a_rerun_does_not_block_an_invalid_record() {
    let text = format!(
        "```nickel\n{}\n```",
        record_with("  bottom_line = \"Kurz \u{2014} zu kurz.\",\n  confidence = 'high,")
    );
    assert_eq!(reason_for_message(&text, true), None);
}

// --- turns that are not final answers pass through -----------------------

#[test]
fn an_empty_message_passes() {
    let payload = json!({
        "hook_event_name": "Stop",
        "stop_hook_active": false,
        "last_assistant_message": "   ",
    })
    .to_string();
    assert!(matches!(
        validate::verdict(&parse_input(&payload)),
        validate::Verdict::Pass
    ));
}

#[test]
fn a_payload_without_a_message_passes() {
    let payload = json!({"hook_event_name": "Stop"}).to_string();
    assert!(matches!(
        validate::verdict(&parse_input(&payload)),
        validate::Verdict::Pass
    ));
}

#[test]
fn a_malformed_payload_passes() {
    assert!(matches!(
        validate::verdict(&parse_input("not json")),
        validate::Verdict::Pass
    ));
}

// --- language packs ------------------------------------------------------

fn reason_with(record: &str, config: &Config) -> Option<String> {
    let payload = json!({
        "hook_event_name": "Stop",
        "stop_hook_active": false,
        "last_assistant_message": format!("```nickel\n{record}\n```"),
    })
    .to_string();
    match validate::verdict_with(&parse_input(&payload), config) {
        validate::Verdict::Pass => None,
        validate::Verdict::Block(reason) => Some(reason),
    }
}

/// The phrase rules follow the language the answer declares.
#[test]
fn phrase_rules_follow_the_answer_language() {
    let config = Config::default();
    let german_hedge = "  bottom_line = \"Das ist vermutlich korrekt.\",\n  confidence = 'high,";

    let r = reason_with(
        &record_with(&format!("{german_hedge}\n  language = 'de,")),
        &config,
    )
    .expect("German hedge blocked in a German answer");
    assert!(r.contains("\"vermutlich\""), "{r}");

    // An English answer is judged by the English list.
    assert_eq!(
        reason_with(
            &record_with(&format!("{german_hedge}\n  language = 'en,")),
            &config
        ),
        None
    );
}

/// A language without a pack gets the universal rules only.
#[test]
fn a_language_without_a_pack_gets_the_universal_rules() {
    let config = Config::default();
    let hedge = "  bottom_line = \"C'est peut-être vrai, maybe.\",\n  confidence = 'high,\n  language = 'fr,";
    assert_eq!(reason_with(&record_with(hedge), &config), None);

    let question = "  bottom_line = \"Est-ce vrai ?\",\n  confidence = 'high,\n  language = 'fr,";
    let r = reason_with(&record_with(question), &config).expect("question blocked");
    assert!(r.contains("ends in a question"), "{r}");
}

/// Question marks of other scripts end a bottom line just as `?` does.
#[test]
fn question_marks_of_other_scripts_are_recognised() {
    let config = Config::default();
    for ending in ["هل هذا صحيح؟", "これは本当？", "Είναι αλήθεια\u{037E}"]
    {
        let r = reason_with(
            &record_with(&format!(
                "  bottom_line = \"{ending}\",\n  confidence = 'high,\n  language = 'xx,"
            )),
            &config,
        )
        .unwrap_or_else(|| panic!("not blocked: {ending}"));
        assert!(r.contains("ends in a question"), "{r}");
    }
}

/// A pack's own checks block the turn and say which check spoke.
#[test]
fn a_pack_check_blocks_the_turn() {
    let path = std::env::temp_dir().join(format!("n4t-hooks-test-{}-xx.ncl", std::process::id()));
    std::fs::write(
        &path,
        r#"{ checks = { no_digits = fun text => if std.string.is_match "[0-9]" text then 'Violation "spell out numbers" else 'Ok } }"#,
    )
    .expect("write pack");
    let pack = UserPack {
        tag: "xx".to_string(),
        file: UserFile {
            path: path.clone(),
            source: std::fs::read_to_string(&path).expect("read pack"),
        },
    };
    let config = Config::from_files(None, &[pack]).expect("pack loads");

    let digits = "  bottom_line = \"Es lief 3 Mal.\",\n  confidence = 'high,\n  language = 'xx,";
    let words = "  bottom_line = \"Es lief dreimal.\",\n  confidence = 'high,\n  language = 'xx,";
    let blocked = reason_with(&record_with(digits), &config);
    let passed = reason_with(&record_with(words), &config);
    let _ = std::fs::remove_file(&path);

    let r = blocked.expect("blocked by the pack check");
    assert!(r.contains("spell out numbers"), "{r}");
    assert!(r.contains("no_digits"), "{r}");
    assert_eq!(passed, None);
}
