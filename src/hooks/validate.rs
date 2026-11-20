//! Stop hook: validates the final answer against the schema and the rules.
//!
//! Display is not this hook's job. The MessageDisplay hook renders the record
//! in place, which keeps the output free of the "Stop says:" prefix Claude Code
//! adds to `systemMessage`. Here the same evaluation runs only for its verdict:
//! on failure the turn is blocked and the reason goes to Claude.
//!
//! Deliberately permissive: anything that is not clearly a finished text answer
//! passes through. A false block costs a whole turn, a false pass costs nothing.

use crate::block;
use crate::config::{self, Config};
use crate::hooks::{log, HookInput};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::io::Write;

/// Blocks the turn with a reason addressed to Claude.
///
/// The flat shape is the one that works. Both were measured against Claude
/// Code 2.1.235 on 2026-08-19 with a throwaway hook that blocked once and told
/// the model to answer with a different word: the flat form changed the answer,
/// the nested `hookSpecificOutput` form left it untouched even though the hook
/// ran. The docs describe the nested variant, so this is worth rechecking after
/// a Claude Code upgrade.
///
/// `suppressOutput` keeps the raw JSON out of the transcript.
fn block_turn(reason: &str) {
    let payload = json!({
        "suppressOutput": true,
        "decision": "block",
        "reason": reason,
    });
    let mut stdout = std::io::stdout();
    let _ = writeln!(stdout, "{payload}");
}

// --- transcript fallback -------------------------------------------------

#[derive(Debug, Deserialize)]
struct TranscriptEntry {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    #[serde(rename = "isSidechain")]
    is_sidechain: bool,
    #[serde(default)]
    message: Option<TranscriptMessage>,
}

#[derive(Debug, Deserialize)]
struct TranscriptMessage {
    #[serde(default)]
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    text: String,
}

/// The text of the last assistant turn, or `None` when the turn is not a final
/// text answer.
///
/// Subagent output (`isSidechain`) is not our concern, and a turn that ends in
/// a tool call is not a final answer.
fn text_from_transcript(path: &str) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    let last = contents
        .lines()
        .filter_map(|line| serde_json::from_str::<TranscriptEntry>(line).ok())
        .rfind(|e| e.r#type == "assistant" && !e.is_sidechain)?;

    let message = last.message?;
    if message.content.iter().any(|b| b.r#type == "tool_use") {
        return None;
    }

    let text = message
        .content
        .iter()
        .filter(|b| b.r#type == "text")
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    Some(text)
}

// --- the hook itself -----------------------------------------------------

/// The verdict, separated from the I/O so it can be tested directly.
pub enum Verdict {
    Pass,
    Block(String),
}

/// The verdict under the user's limits.
pub fn verdict(input: &HookInput) -> Verdict {
    verdict_with(input, &config::load_or_default())
}

/// The verdict under the given limits. Tests pass the defaults here, so the
/// user's own config file cannot change their outcome.
pub fn verdict_with(input: &HookInput, limits: &Config) -> Verdict {
    // Already inside a block-triggered rerun: never block twice, that loops.
    // Claude Code caps consecutive blocks anyway, but the cap is a backstop,
    // not a design.
    let in_rerun = input.stop_hook_active.unwrap_or(false);

    // `last_assistant_message` carries the current turn's text directly. The
    // transcript is written asynchronously and can lag behind it, so it is only
    // the fallback for older Claude Code versions that do not send the field.
    let text = match input
        .last_assistant_message
        .as_deref()
        .filter(|t| !t.trim().is_empty())
    {
        Some(text) => text.to_string(),
        None => match input
            .transcript_path
            .as_deref()
            .and_then(text_from_transcript)
        {
            Some(text) => text,
            None => {
                log("skip: no message, no usable transcript");
                return Verdict::Pass;
            }
        },
    };

    if text.trim().is_empty() {
        log("skip: empty text");
        return Verdict::Pass;
    }

    // An empty block is treated like a missing one: there is no record.
    let Some(source) = block::extract(&text).filter(|s| !s.trim().is_empty()) else {
        if in_rerun {
            log("skip: no block, rerun in progress");
            return Verdict::Pass;
        }
        log("block: no nickel block");
        return Verdict::Block(
            "The final answer has no ```nickel block. Restate it as a render{...} record."
                .to_string(),
        );
    };

    match block::evaluate(&source, limits) {
        Ok(_) => {
            log("pass: schema satisfied");
            Verdict::Pass
        }
        Err(err) => {
            if in_rerun {
                log("skip: invalid, rerun in progress");
                return Verdict::Pass;
            }
            let reason = err.reason(&source);
            log(&format!(
                "block: {}",
                reason.lines().next().unwrap_or_default()
            ));
            Verdict::Block(format!(
                "The nickel block does not satisfy the schema:\n\n{reason}\n\nFix the record and answer again."
            ))
        }
    }
}

pub fn run(input: &HookInput) -> i32 {
    match verdict(input) {
        Verdict::Pass => {}
        Verdict::Block(reason) => block_turn(&reason),
    }
    0
}
