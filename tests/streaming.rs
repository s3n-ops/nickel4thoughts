//! MessageDisplay streaming, driven through the binary the way Claude Code
//! calls it.
//!
//! The event arrives in two shapes. Interactive use sends a batch of newly
//! completed lines per call, with `index` rising and `final` false until the
//! end. Non-interactive runs (`-p`, the Agent SDK) send the whole message at
//! once with `index` 0 and `final` true. The fence state has to survive between
//! calls, so both shapes are exercised against the real state file rather than
//! an in-process struct.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_n4t")
}

/// Sends one delta to `n4t render` and returns the replacement text.
fn render_delta(message_id: &str, index: u32, is_final: bool, delta: &str) -> String {
    let payload = json!({
        "hook_event_name": "MessageDisplay",
        "message_id": message_id,
        "index": index,
        "final": is_final,
        "delta": delta,
    })
    .to_string();

    // A config directory that does not exist, so the user's own limits cannot
    // change what these tests see.
    let mut child = Command::new(binary())
        .arg("render")
        .env("N4T_CONFIG_DIR", "/nonexistent/n4t-test-config")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn n4t render");

    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write payload");

    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "n4t render exited {:?}", out.status);

    if out.stdout.is_empty() {
        return String::new();
    }

    let parsed: Value = serde_json::from_slice(&out.stdout).expect("hook output is JSON");
    parsed["hookSpecificOutput"]["displayContent"]
        .as_str()
        .expect("displayContent")
        .to_string()
}

fn record() -> String {
    let schema = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
    format!(
        "let {{ render, .. }} = import \"{}/render.ncl\" in\nrender {{ bottom_line = \"The hook renders.\", confidence = 'high }}",
        schema.display()
    )
}

/// A distinct id per test, so concurrent runs cannot share a state file.
fn message_id(name: &str) -> String {
    format!("{name}-{}", std::process::id())
}

#[test]
fn a_whole_message_in_one_call() {
    let id = message_id("oneshot");
    let delta = format!("```nickel\n{}\n```\n", record());
    let out = render_delta(&id, 0, true, &delta);
    assert!(out.contains("## Bottom line"), "{out}");
    assert!(out.contains("The hook renders."), "{out}");
    assert!(!out.contains("```nickel"), "{out}");
}

#[test]
fn a_message_split_across_calls() {
    let id = message_id("streamed");
    let rec = record();

    // The opening fence alone: nothing to show yet.
    assert_eq!(render_delta(&id, 0, false, "```nickel\n"), "");
    // The record itself: still collecting.
    assert_eq!(render_delta(&id, 1, false, &format!("{rec}\n")), "");
    // The closing fence releases the rendering.
    let out = render_delta(&id, 2, true, "```\n");
    assert!(out.contains("## Bottom line"), "{out}");
    assert!(out.contains("The hook renders."), "{out}");
}

#[test]
fn a_foreign_fence_passes_through() {
    let id = message_id("foreign");
    let out = render_delta(&id, 0, true, "```bash\njust check\n```\n");
    assert!(out.contains("just check"), "{out}");
}

/// An unrenderable record is shown as source, so the reader is not left with a
/// hole where the answer was. The Stop hook blocks on it separately.
#[test]
fn an_invalid_record_shows_its_source() {
    let id = message_id("invalid");
    let out = render_delta(&id, 0, true, "```nickel\nrender {\n```\n");
    assert!(out.contains("```nickel"), "{out}");
    assert!(out.contains("render {"), "{out}");
}

/// A message interrupted inside the block must still show what it collected.
/// The final batch is empty whenever the message ends on a newline, which is
/// the common shape, so the recovery has to survive an empty delta.
#[test]
fn an_interrupted_block_survives_an_empty_final_batch() {
    let id = message_id("interrupted-empty");
    render_delta(&id, 0, false, "```nickel\nrender {\n");
    let out = render_delta(&id, 1, true, "");
    assert!(out.contains("render {"), "collected source lost: {out:?}");
    assert!(out.contains("```nickel"), "{out:?}");
    assert!(out.trim_end().ends_with("```"), "fence left open: {out:?}");
}

/// The same interruption when the final batch still carries text.
#[test]
fn an_interrupted_block_is_shown_as_source() {
    let id = message_id("interrupted");
    render_delta(&id, 0, false, "```nickel\nrender {\n");
    let out = render_delta(&id, 1, true, "more source\n");
    assert!(out.contains("more source"), "{out:?}");
    assert!(out.trim_end().ends_with("```"), "fence left open: {out:?}");
}

/// Two messages must not share state, even when they interleave.
#[test]
fn two_messages_keep_separate_state() {
    let a = message_id("interleaved-a");
    let b = message_id("interleaved-b");
    let rec = record();

    render_delta(&a, 0, false, "```nickel\n");
    render_delta(&b, 0, false, "plain prose\n");
    render_delta(&a, 1, false, &format!("{rec}\n"));

    let out_b = render_delta(&b, 1, true, "more prose\n");
    assert!(
        !out_b.contains("## Bottom line"),
        "b picked up a's block: {out_b}"
    );

    let out_a = render_delta(&a, 2, true, "```\n");
    assert!(out_a.contains("## Bottom line"), "{out_a}");
}

// Concurrent flushes. Claude Code runs up to three flushes of one message at
// once and stitches only their outputs together in index order. The fence state
// must be advanced in index order regardless of which process runs first.

/// The record split the way the bug report's reproduction splits it.
fn chunks() -> Vec<String> {
    let rec = record();
    let (head, tail) = rec.split_at(rec.find('\n').expect("two lines") + 1);
    vec![
        "```nickel\n".to_string(),
        head.to_string(),
        format!("{tail}\n```\n"),
    ]
}

/// Spawns one flush without waiting for it.
fn spawn_flush(
    message_id: &str,
    index: u32,
    is_final: bool,
    delta: &str,
) -> std::thread::JoinHandle<String> {
    let (id, delta) = (message_id.to_string(), delta.to_string());
    std::thread::spawn(move || render_delta(&id, index, is_final, &delta))
}

#[test]
fn flushes_out_of_order_render_the_record() {
    let id = message_id("out-of-order");
    let c = chunks();

    // Flush 1 starts first and has to wait for flush 0.
    let one = spawn_flush(&id, 1, false, &c[1]);
    std::thread::sleep(std::time::Duration::from_millis(50));
    let zero = render_delta(&id, 0, false, &c[0]);
    let two = render_delta(&id, 2, false, &c[2]);
    let three = render_delta(&id, 3, true, "");
    let shown = format!("{zero}{}{two}{three}", one.join().expect("flush 1"));

    assert!(shown.contains("## Bottom line"), "{shown}");
    assert!(!shown.contains("render {"), "raw record shown: {shown}");
}

/// The race depends on timing, so it is run several times over.
#[test]
fn concurrent_flushes_render_the_record() {
    let c = chunks();
    for round in 0..10 {
        let id = message_id(&format!("concurrent-{round}"));
        let handles: Vec<_> = (0..3)
            .map(|i| spawn_flush(&id, i as u32, false, &c[i]))
            .collect();
        let outs: Vec<String> = handles
            .into_iter()
            .map(|h| h.join().expect("flush"))
            .collect();
        let shown = format!("{}{}", outs.concat(), render_delta(&id, 3, true, ""));

        assert!(shown.contains("## Bottom line"), "round {round}: {shown}");
        assert!(
            !shown.contains("render {"),
            "round {round}: raw record shown: {shown}"
        );
    }
}

/// A predecessor that never arrives degrades to the original text after the
/// wait bound, instead of hanging or swallowing the collected lines.
#[test]
fn a_missing_flush_passes_the_rest_through() {
    let id = message_id("missing");
    let c = chunks();

    assert_eq!(render_delta(&id, 0, false, &c[0]), "");
    // Flush 1 never arrives.
    let started = std::time::Instant::now();
    let two = render_delta(&id, 2, false, &c[2]);
    let waited = started.elapsed();
    assert!(waited >= std::time::Duration::from_secs(1), "{waited:?}");
    assert!(waited < std::time::Duration::from_secs(4), "{waited:?}");
    // The opening fence held back by flush 0 is shown again, followed by the
    // delta unchanged.
    assert_eq!(two, format!("```nickel\n{}", c[2]));

    // The passthrough holds for the rest of the message, without a second
    // wait, even for a block that would otherwise render.
    let whole = format!("```nickel\n{}\n```\n", record());
    let started = std::time::Instant::now();
    assert_eq!(render_delta(&id, 3, true, &whole), whole);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn a_one_shot_message_does_not_wait() {
    let id = message_id("oneshot-fast");
    let delta = format!("```nickel\n{}\n```\n", record());
    let started = std::time::Instant::now();
    let out = render_delta(&id, 0, true, &delta);
    assert!(out.contains("## Bottom line"), "{out}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}

/// A reused id starts over at index 0, as `just check` does with its fixed id.
#[test]
fn index_zero_starts_a_reused_id_over() {
    let id = message_id("reused");
    render_delta(&id, 0, false, "```nickel\n");
    let delta = format!("```nickel\n{}\n```\n", record());
    let out = render_delta(&id, 0, true, &delta);
    assert!(out.contains("## Bottom line"), "{out}");
}
