//! The three hook subcommands, one per Claude Code event.
//!
//! Each reads the hook payload as JSON from stdin and writes its answer to
//! stdout. Parsing it in the same process saves the `jq` calls a shell hook
//! would need, which cost 23 ms of 54 ms per MessageDisplay call in a shell
//! version.

pub mod display;
pub mod inject;
pub mod validate;

use serde::Deserialize;
use std::io::{self, Read, Write};

/// Set N4T_HOOK_LOG to a path to record hook activity. Without it the hooks
/// leave no trace, which makes "did it even run" unanswerable.
pub fn log(line: &str) {
    let Ok(path) = std::env::var("N4T_HOOK_LOG") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

/// The fields these hooks read, across all three events. Unknown fields are
/// ignored: Claude Code adds them over time and a strict parse would turn a
/// new field into a broken hook.
#[derive(Debug, Default, Deserialize)]
pub struct HookInput {
    // MessageDisplay
    #[serde(default)]
    pub delta: String,
    #[serde(default)]
    pub message_id: Option<String>,
    /// Zero-based flush index. Claude Code runs up to three flushes of one
    /// message at once, so this is what puts them back in order.
    #[serde(default)]
    pub index: Option<u64>,
    // `final` is a Rust keyword, so the field is renamed on the way in.
    #[serde(default, rename = "final")]
    pub final_: Option<bool>,

    // Stop
    #[serde(default)]
    pub last_assistant_message: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub stop_hook_active: Option<bool>,
}

impl HookInput {
    pub fn is_final(&self) -> bool {
        self.final_.unwrap_or(false)
    }
}

/// Reads and parses the payload. A malformed payload yields defaults rather
/// than an error: these hooks are deliberately permissive, since a false block
/// costs a whole turn and a false pass costs nothing.
pub fn read_input() -> HookInput {
    let mut raw = String::new();
    if io::stdin().read_to_string(&mut raw).is_err() {
        return HookInput::default();
    }
    parse_input(&raw)
}

pub fn parse_input(raw: &str) -> HookInput {
    serde_json::from_str(raw).unwrap_or_default()
}
