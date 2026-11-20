//! n4t: BLUF answer validation for Claude Code.
//!
//! Nickel does the parsing and checks the structure with a contract, because
//! the record is real Nickel syntax (`m%"..."%`, enum tags, `import`).
//! Everything past that (prose rules, word budgets, rendering, error
//! reporting) runs in Rust, where the regexes compile once instead of once per
//! field. As Nickel contracts, that recompilation took 22 of 32 ms.
//!
//! The crate is a library so the integration tests can reach the rules
//! directly. The binary in `main.rs` is a thin clap wrapper around it.

pub mod answer;
pub mod block;
pub mod checks;
pub mod config;
pub mod hooks;
pub mod nickel;
pub mod prose;
pub mod render;
pub mod rules;
