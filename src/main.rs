//! n4t: BLUF answer validation for Claude Code.
//!
//! One binary, three hook subcommands plus two manual ones. Every hook reads
//! its payload as JSON from stdin and writes its answer to stdout. The rules
//! themselves live in the library; this file is the clap wrapper around them.

use clap::{Parser, Subcommand};
use n4t::{block, config, hooks, render};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "n4t",
    about = "BLUF answer validation for Claude Code",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// UserPromptSubmit hook: print the answer format rule.
    InjectRule,
    /// MessageDisplay hook: replace the record on screen with its rendering.
    Render,
    /// Stop hook: validate the record, block the turn on violation.
    Validate,
    /// Check a record and report the diagnosis. Exits non-zero on violation.
    Check {
        /// File holding a record, or a message containing a ```nickel block.
        file: PathBuf,
    },
    /// Render a record as Markdown.
    Fmt {
        /// File holding a record, or a message containing a ```nickel block.
        file: PathBuf,
    },
    /// Show the active limits and where they come from. Exits non-zero when
    /// the config file is invalid.
    Config,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let code = match cli.command {
        Command::InjectRule => hooks::inject::run(),
        Command::Render => hooks::display::run(&hooks::read_input()),
        Command::Validate => hooks::validate::run(&hooks::read_input()),
        Command::Check { file } => manual(&file, Output::Diagnosis),
        Command::Fmt { file } => manual(&file, Output::Markdown),
        Command::Config => show_config(),
    };

    ExitCode::from(code as u8)
}

enum Output {
    Diagnosis,
    Markdown,
}

/// `check` and `fmt` make the rules testable without Claude Code. They accept
/// both a bare record and a whole message, so a transcript excerpt works
/// unchanged, from a pipe via `/dev/stdin`.
fn manual(file: &PathBuf, output: Output) -> i32 {
    let contents = match std::fs::read_to_string(file) {
        Ok(contents) => contents,
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            return 2;
        }
    };

    let source = block::extract(&contents).unwrap_or(contents);

    // The same limits the hooks apply, so a manual check agrees with them.
    let limits = match config::load() {
        Ok(limits) => limits,
        Err(err) => {
            eprintln!("{err}\nThe default limits apply.");
            config::Config::default()
        }
    };

    match block::evaluate(&source, &limits) {
        Ok(answer) => {
            match output {
                Output::Diagnosis => println!("OK"),
                Output::Markdown => print!("{}", render::render(&answer, &limits)),
            }
            0
        }
        Err(err) => {
            eprintln!("{}", err.reason(&source));
            1
        }
    }
}

/// Prints the limits in effect. The hooks fall back to the defaults on a broken
/// file without saying so on screen, so this is where the reason shows up.
fn show_config() -> i32 {
    let path = config::config_path();
    let (limits, code) = match config::load() {
        Ok(limits) => {
            match &path {
                Some(p) if p.is_file() => println!("Config: {}", p.display()),
                Some(p) => println!("Config: {} (not found, defaults apply)", p.display()),
                None => println!("Config: no config directory (defaults apply)"),
            }
            (limits, 0)
        }
        Err(err) => {
            println!("Config: {err}");
            println!("The default limits apply.");
            (config::Config::default(), 1)
        }
    };

    let b = &limits.budgets;
    println!();
    println!("budgets.bottom_line        {}", b.bottom_line);
    println!("budgets.supporting_detail  {}", b.supporting_detail);
    println!("budgets.evidence           {}", b.evidence);
    println!("budgets.caveats            {}", b.caveats);
    println!("budgets.next_action        {}", b.next_action);
    println!("sentence_length            {}", limits.sentence_length);
    println!("default_language           {}", limits.default_language);
    println!(
        "languages                  {}",
        render::languages(&limits).join(", ")
    );

    // Where each language comes from, and whether its pack brings checks.
    println!();
    for (tag, language) in &limits.languages {
        let phrases: usize = language.phrases.lists().iter().map(|(_, l)| l.len()).sum();
        let checks = if language.checks.is_some() {
            ", checks"
        } else {
            ""
        };
        println!(
            "{tag:<6} {phrases} phrases{checks}  ({})",
            language.origin.join(" + ")
        );
    }
    code
}
