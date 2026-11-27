//! MessageDisplay hook: replaces the raw ```nickel record on screen with its
//! rendered Markdown.
//!
//! The record is the answer's source form; the reader wants the rendering.
//! This hook collects the block while it streams and, at the closing fence,
//! emits the rendering in its place. The Stop hook still validates and blocks
//! on violation, but it no longer needs to display anything, so the reader is
//! spared Claude Code's "Stop says:" prefix.
//!
//! Display-only. The transcript and what Claude sees keep the original text,
//! so nothing here can corrupt the conversation. If the hook fails or times
//! out, Claude Code shows the original record: degraded, never broken.
//!
//! The event streams: in interactive use each call carries newly completed
//! lines in `delta`, not the whole message. Non-interactive runs (`-p`, the
//! Agent SDK) deliver it in one piece with `index` 0 and `final` true. The
//! fence state therefore has to survive between calls, and both shapes have to
//! work.
//!
//! Claude Code runs up to three flushes of one message at once and starts a new
//! one every 100 ms. Only their outputs are stitched together in index order;
//! the executions overlap. Each call therefore takes an exclusive lock on the
//! state file and waits until every lower index has been processed. A
//! predecessor that never arrives is waited for only so long: after that the
//! message degrades to its original text, never to a hung hook.
//!
//! This hook deletes nothing. It runs on every displayed message, so a
//! deletion bug here would be both frequent and silent. State lives in one
//! file per message that is truncated on reuse and swept by age, never removed.

use crate::block::{BlockScan, Line};
use crate::config::{self, Config};
use crate::hooks::{log, HookInput};
use crate::nickel::eval_answer;
use crate::render::render;
use serde_json::json;
use std::fs;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Files older than this belong to messages that were interrupted before their
/// final batch. An hour outlives any live message.
const STALE_AFTER: Duration = Duration::from_secs(3600);

/// How long a flush waits for its predecessors. Well below the 5 s timeout the
/// hook is registered with, so a stuck predecessor costs a delay and the raw
/// text, not a timed-out hook.
const WAIT_LIMIT: Duration = Duration::from_secs(2);

/// Pause between two looks at the state while a predecessor is still running.
const WAIT_STEP: Duration = Duration::from_millis(5);

/// The fence state carried between calls.
#[derive(Debug, PartialEq)]
struct State {
    inside: bool,
    block: String,
    /// The flush index whose turn it is.
    next: u64,
    /// Set once a flush gave up waiting. The fence state no longer matches the
    /// text on screen, so every later flush of the message passes its delta
    /// through unchanged.
    passthrough: bool,
}

impl State {
    fn empty() -> State {
        State {
            inside: false,
            block: String::new(),
            next: 0,
            passthrough: false,
        }
    }
}

/// `message_id` comes from outside, so it is not spliced into a path unchecked:
/// a value like "../../x" would place the state file anywhere. Keep the
/// characters a UUID can contain and nothing else.
///
/// `None` when the id is missing or unusable. There is then no way to tell one
/// message from the next, and a single shared file would carry an open fence
/// from one message into the next, whose prose would be collected as block
/// content and never displayed. Such a batch is rewritten without persisted
/// state instead: a block contained in one batch still renders, and a block
/// split across batches degrades to the original text rather than swallowing it.
fn safe_message_id(id: Option<&str>) -> Option<String> {
    match id {
        Some(id) if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => {
            Some(id.to_string())
        }
        _ => None,
    }
}

/// The state directory is derived, never taken from the input.
fn state_dir() -> PathBuf {
    state_dir_from(
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var("USER").ok(),
        std::env::temp_dir(),
    )
}

/// Picks the state directory from the environment it is given.
///
/// `$XDG_RUNTIME_DIR` comes first. It belongs to the user alone, lives in RAM
/// and is emptied at logout, while /tmp may sit on disk on some systems. Every
/// batch of every answer writes here, so keeping those writes off the disk
/// matters. The variable is only trusted when it holds an absolute path.
///
/// Without it, the user name keeps two accounts on a shared /tmp apart. The
/// directory is created with mode 700 below, so the name only has to be
/// distinct, not unguessable.
fn state_dir_from(runtime: Option<PathBuf>, user: Option<String>, tmp: PathBuf) -> PathBuf {
    if let Some(dir) = runtime.filter(|d| d.is_absolute()) {
        return dir.join("n4t-render");
    }
    let user = user
        .filter(|u| {
            !u.is_empty()
                && u.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .unwrap_or_else(|| "default".to_string());
    tmp.join(format!("n4t-render-{user}"))
}

/// Creates the state directory with owner-only permissions, so a shared /tmp
/// cannot hand us someone else's files.
fn prepare_state_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Stale files from interrupted messages.
///
/// Safety of this sweep, in order: only the derived directory is read, only its
/// immediate entries are considered, symlinks and directories are skipped, and a
/// stale file that still holds state is truncated rather than unlinked, so a
/// wrong match costs an emptied scratch file and not a lost one.
///
/// A stale file that is already empty is removed. Every message leaves one file
/// behind, truncated on its last batch and never deleted, so without this the
/// directory grows without bound for the life of the machine's /tmp. Removing
/// only the empty ones keeps the guarantee above: nothing that still carries
/// state is unlinked.
///
/// `symlink_metadata` rather than `metadata`, because the latter follows the
/// link and would report a symlink to a regular file as a file, truncating the
/// target instead of skipping it.
fn sweep(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if now.duration_since(modified).unwrap_or_default() > STALE_AFTER {
            if meta.len() == 0 {
                let _ = fs::remove_file(entry.path());
            } else {
                let _ = fs::File::create(entry.path());
            }
        }
    }
}

/// One file per message. The first line is `<mode> <next>`: mode `0` outside
/// the block, `1` inside it, `p` for passthrough, and the index of the flush
/// whose turn it is. The rest is the block collected so far. An empty or
/// unreadable file is a fresh message.
fn parse_state(contents: &str) -> State {
    let mut lines = contents.splitn(2, '\n');
    let mut header = lines.next().unwrap_or("").split(' ');
    let mode = header.next().unwrap_or("");
    let next = header.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    State {
        inside: mode == "1",
        block: lines.next().unwrap_or("").to_string(),
        next,
        passthrough: mode == "p",
    }
}

fn format_state(state: &State) -> String {
    let mode = match (state.passthrough, state.inside) {
        (true, _) => "p",
        (false, true) => "1",
        (false, false) => "0",
    };
    format!("{mode} {}\n{}", state.next, state.block)
}

/// The state file, opened and exclusively locked. The lock is released when the
/// handle is dropped.
fn lock_state(path: &Path) -> std::io::Result<fs::File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock()?;
    Ok(file)
}

fn read_state(file: &mut fs::File) -> State {
    let mut contents = String::new();
    if file.rewind().is_err() || file.read_to_string(&mut contents).is_err() {
        return State::empty();
    }
    parse_state(&contents)
}

/// Replaces the file's contents through the locked handle. Writing through a
/// new path would bypass the lock.
fn write_state(file: &mut fs::File, contents: &str) {
    let _ = file.set_len(0);
    let _ = file.rewind();
    let _ = file.write_all(contents.as_bytes());
}

/// Renders the collected block, or returns `None` when it does not evaluate.
///
/// Only the structure is checked here. The prose rules and the checks of the
/// language packs belong to the Stop hook, which blocks the turn on a
/// violation; running them here too would cost the time twice and replace a
/// readable answer with its raw source.
fn render_block(source: &str, config: &Config) -> Option<String> {
    eval_answer(source).ok().map(|a| render(&a, config))
}

/// Rewrites one delta, carrying the fence state across calls.
///
/// The record is the whole answer, so prose in front of it is a preamble the
/// rule already forbids. When the opening fence arrives in the same batch as
/// that prose, it has not been drawn yet and can still be dropped. Text from an
/// earlier batch is already on screen and stays there. This tightens the common
/// case without pretending to fix every one.
///
/// The config is read from the user's file, and only when a block closes, so a
/// batch of plain prose costs no Nickel evaluation.
fn rewrite(delta: &str, state: &mut State) -> String {
    rewrite_with(delta, state, &config::load_or_default)
}

/// `rewrite` with the config supplied by the caller, so tests do not depend on
/// the config file of the machine they run on.
fn rewrite_with(delta: &str, state: &mut State, config: &dyn Fn() -> Config) -> String {
    let mut out = String::new();
    // Whether this batch has already emitted a rendering. Dropping the preamble
    // may only discard prose that has not been drawn yet; once a block has been
    // rendered into `out`, a second block in the same batch must append to it
    // rather than replace it, or the first answer and everything between them
    // disappears from the screen.
    let mut rendered_here = false;
    let mut scan = BlockScan::resume(state.inside, &state.block);

    for line in delta.lines() {
        match scan.feed(line) {
            Line::Outside => {
                out.push_str(line);
                out.push('\n');
            }
            Line::Open => {
                state.block.clear();
                if !rendered_here {
                    out.clear();
                }
            }
            Line::Content => {
                state.block.push_str(line);
                state.block.push('\n');
            }
            Line::Close => {
                rendered_here = true;
                match render_block(&state.block, &config()) {
                    Some(rendered) => {
                        out.push_str(&rendered);
                        out.push('\n');
                    }
                    None => {
                        // Unrenderable: show the source so the reader is not
                        // left with a hole. The Stop hook blocks on this and a
                        // corrected answer follows.
                        append_source(&mut out, &state.block);
                    }
                }
                state.block.clear();
            }
        }
    }
    state.inside = scan.inside();

    // `lines()` drops a trailing newline the delta did not have. Restore the
    // original ending so line breaks are not silently added or removed.
    if !delta.ends_with('\n') {
        while out.ends_with('\n') {
            out.pop();
        }
    }

    out
}

/// Appends an unrendered block as its own source, so an interrupted message
/// shows what it collected instead of dropping it.
///
/// The fence is closed. An unterminated one makes a Markdown renderer treat
/// everything after it as code, which swallows the rest of the screen.
fn append_source(out: &mut String, block: &str) {
    out.push_str("```nickel\n");
    out.push_str(block);
    if !block.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("```\n");
}

/// Emits the replacement text. Claude Code renders this in place of the delta;
/// an empty string renders nothing.
fn emit(text: &str) {
    let payload = json!({
        "hookSpecificOutput": {
            "hookEventName": "MessageDisplay",
            "displayContent": text,
        }
    });
    let mut stdout = std::io::stdout();
    let _ = writeln!(stdout, "{payload}");
}

/// What a flush does once it holds the lock.
enum Turn {
    /// Its predecessors are done: rewrite the delta.
    Ready,
    /// A predecessor is still running: release the lock and look again.
    Wait,
    /// The fence state cannot be trusted for this flush: show the delta as is.
    PassThrough,
}

/// Decides whether the flush with `index` may proceed.
///
/// Index 0 always starts a message. Its id is fresh, so leftover state under the
/// same id comes from a reused id, as in `just check`, and is discarded. The
/// exception is a message already in passthrough: then a later flush gave up on
/// this one, and processing it now would rewrite text out of order.
fn turn(index: Option<u64>, state: &mut State) -> Turn {
    let Some(index) = index else {
        // No index, no order to restore: behave as before ordering existed.
        return if state.passthrough {
            Turn::PassThrough
        } else {
            Turn::Ready
        };
    };
    if state.passthrough {
        return Turn::PassThrough;
    }
    if index == 0 {
        *state = State::empty();
        return Turn::Ready;
    }
    match index.cmp(&state.next) {
        std::cmp::Ordering::Equal => Turn::Ready,
        std::cmp::Ordering::Greater => Turn::Wait,
        // Already processed: a repeated flush. Rewriting it again would apply
        // its lines to the fence state twice.
        std::cmp::Ordering::Less => Turn::PassThrough,
    }
}

/// The text a flush that gave up shows: everything held back so far, in its
/// original form, followed by its own delta.
///
/// The earlier flushes returned nothing for the block lines they collected, so
/// without this they would be lost. Their lines appear after the missing
/// predecessor's text rather than before it, which is the price of not knowing
/// whether that predecessor will ever run. Out of order beats swallowed.
///
/// On the final flush nothing follows that could close a reopened fence, so it
/// is closed here when the text leaves it open, as `append_source` does.
fn give_up_text(state: &State, delta: &str, is_final: bool) -> String {
    let mut out = String::new();
    if state.inside {
        out.push_str("```nickel\n");
        out.push_str(&state.block);
    }
    out.push_str(delta);
    if is_final && ends_inside_block(&out) {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("```\n");
    }
    out
}

/// Whether `text` ends inside a ```nickel block, by the rule `rewrite` uses to
/// close one.
fn ends_inside_block(text: &str) -> bool {
    let mut scan = BlockScan::default();
    for line in text.lines() {
        scan.feed(line);
    }
    scan.inside()
}

/// Rewrites the delta once its predecessors are done. On give-up the delta is
/// shown unchanged, see `give_up_text`.
fn run_ordered(input: &HookInput, path: &Path) -> String {
    let started = Instant::now();
    let mut waited = false;
    loop {
        let Ok(mut file) = lock_state(path) else {
            // No lock, no safe state: show the original text.
            return input.delta.clone();
        };
        let mut state = read_state(&mut file);

        match turn(input.index, &mut state) {
            Turn::Ready => {
                if waited {
                    log(&format!(
                        "render {}: index {:?} waited {} ms",
                        path.display(),
                        input.index,
                        started.elapsed().as_millis()
                    ));
                }
                return finish(input, &mut file, state, rewrite);
            }
            Turn::PassThrough => {
                return finish(input, &mut file, state, |delta, _| delta.to_string());
            }
            Turn::Wait if started.elapsed() >= WAIT_LIMIT => {
                log(&format!(
                    "render {}: index {:?} gave up waiting for index {}",
                    path.display(),
                    input.index,
                    state.next
                ));
                let out = give_up_text(&state, &input.delta, input.is_final());
                state.passthrough = true;
                state.inside = false;
                state.block.clear();
                return finish(input, &mut file, state, move |_, _| out);
            }
            Turn::Wait => {
                waited = true;
                drop(file);
                std::thread::sleep(WAIT_STEP);
            }
        }
    }
}

/// Produces the flush's text with `step`, advances the index and stores the
/// state. Runs under the lock.
fn finish(
    input: &HookInput,
    file: &mut fs::File,
    mut state: State,
    step: impl FnOnce(&str, &mut State) -> String,
) -> String {
    let mut out = step(&input.delta, &mut state);
    if let Some(index) = input.index {
        state.next = state.next.max(index + 1);
    }

    // Truncate on the last batch rather than deleting: the next message gets a
    // fresh id and its own file, and the sweep clears the leftovers.
    if input.is_final() {
        // A message that ends inside the block was interrupted, or its closing
        // fence never arrived. Everything collected since the opening fence is
        // held in the state and has never been drawn, so without this it is
        // simply lost and the reader is left with a truncated message and no
        // sign of why. Show the source instead.
        if state.inside && !state.block.is_empty() {
            append_source(&mut out, &state.block);
        }
        write_state(file, "");
    } else {
        write_state(file, &format_state(&state));
    }
    out
}

pub fn run(input: &HookInput) -> i32 {
    // Nothing to rewrite and nothing to close off. A flush with an index still
    // has to take its turn, or its successor waits for it in vain. The final
    // batch is empty whenever the message ends on a newline, and it must not
    // skip the final-batch handling: a message interrupted inside a block holds
    // its collected source in the state file, which still has to be shown.
    if input.delta.is_empty() && !input.is_final() && input.index.is_none() {
        return 0;
    }

    // Without a usable id there is no per-message file to key state on, so the
    // batch is rewritten on its own and nothing is carried across calls.
    let Some(id) = safe_message_id(input.message_id.as_deref()) else {
        let mut state = State::empty();
        let mut out = rewrite(&input.delta, &mut state);
        if state.inside && !state.block.is_empty() {
            append_source(&mut out, &state.block);
        }
        emit(&out);
        return 0;
    };

    let dir = state_dir();
    if prepare_state_dir(&dir).is_err() {
        return 0;
    }
    sweep(&dir);

    let out = run_ordered(input, &dir.join(id));
    emit(&out);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped packs, which bring German headings, independent of the
    /// config of the machine the tests run on.
    fn german() -> Config {
        Config::default()
    }

    /// `rewrite` under a fixed config instead of the machine's own.
    fn rw(delta: &str, state: &mut State) -> String {
        rewrite_with(delta, state, &german)
    }

    fn record(language: &str) -> String {
        format!(
            "let {{ render, .. }} = import \"{}/render.ncl\" in\nrender {{ bottom_line = \"ok\", confidence = 'high, language = '{language} }}",
            crate::hooks::inject::schema_dir().display()
        )
    }

    #[test]
    fn renders_a_block_in_one_batch() {
        let delta = format!("```nickel\n{}\n```\n", record("de"));
        let mut state = State::empty();
        let out = rw(&delta, &mut state);
        assert!(out.contains("## Fazit"), "{out}");
    }

    #[test]
    fn renders_english_headings() {
        let delta = format!("```nickel\n{}\n```\n", record("en"));
        let mut state = State::empty();
        let out = rw(&delta, &mut state);
        assert!(out.contains("## Bottom line"), "{out}");
    }

    /// A field holding a fenced code example. The scan used to close the block
    /// at that inner fence and render half a record, or fall back to showing
    /// the source. Both fences arrive in one batch here.
    #[test]
    fn a_fenced_example_in_a_field_does_not_close_the_block() {
        let record = format!(
            "let {{ render, .. }} = import \"{}/render.ncl\" in\nrender {{ bottom_line = m%\"\nText davor.\n\n```bash\necho hallo\n```\n\nText danach.\n\"%, confidence = 'high, language = 'de }}",
            crate::hooks::inject::schema_dir().display()
        );
        let delta = format!("```nickel\n{record}\n```\n");
        let mut state = State::empty();
        let out = rw(&delta, &mut state);
        assert!(out.contains("## Fazit"), "{out}");
        assert!(out.contains("echo hallo"), "{out}");
        assert!(!state.inside, "block never closed: {out}");
    }

    /// The same record split so the inner fence lands in its own batch, which
    /// is where the scan has to be rebuilt from the collected block.
    #[test]
    fn a_fenced_example_survives_a_batch_split() {
        let head = format!(
            "let {{ render, .. }} = import \"{}/render.ncl\" in\nrender {{ bottom_line = m%\"\nText davor.\n\n```bash",
            crate::hooks::inject::schema_dir().display()
        );
        let mut state = State::empty();
        rw(&format!("```nickel\n{head}\n"), &mut state);
        let out = rw(
            "echo hallo\n```\n\nText danach.\n\"%, confidence = 'high, language = 'de }\n```\n",
            &mut state,
        );
        assert!(out.contains("## Fazit"), "{out}");
        assert!(!state.inside, "block never closed: {out}");
    }

    #[test]
    fn drops_a_preamble_in_the_same_batch() {
        let delta = format!("PREAMBLE\n```nickel\n{}\n```\n", record("de"));
        let mut state = State::empty();
        let out = rw(&delta, &mut state);
        assert!(!out.contains("PREAMBLE"), "{out}");
    }

    // Prose from an earlier batch is already drawn and cannot be taken back, so
    // the rendering has to start on a line of its own. Without the leading
    // blank line a Markdown renderer joins `## Fazit` to that prose.
    #[test]
    fn rendering_starts_on_its_own_line() {
        let mut state = State::empty();
        rw("EARLIER PROSE", &mut state);
        let delta = format!("```nickel\n{}\n```\n", record("de"));
        let out = rw(&delta, &mut state);
        assert!(out.starts_with('\n'), "{out:?}");
    }

    // Prose rules belong to the Stop hook. The display renders a record that
    // breaks one, instead of showing its raw source, and the Stop hook asks
    // for the corrected answer.
    #[test]
    fn a_rule_violation_still_renders() {
        let record = format!(
            "let {{ render, .. }} = import \"{}/render.ncl\" in\nrender {{ bottom_line = \"Das ist vermutlich korrekt.\", confidence = 'high, language = 'de }}",
            crate::hooks::inject::schema_dir().display()
        );
        let mut state = State::empty();
        let out = rw(&format!("```nickel\n{record}\n```\n"), &mut state);
        assert!(out.contains("## Fazit"), "{out}");
        assert!(!out.contains("```nickel"), "{out}");
    }

    #[test]
    fn a_foreign_fence_passes_through() {
        let mut state = State::empty();
        let out = rw("```bash\njust check\n```\n", &mut state);
        assert!(out.contains("just check"), "{out}");
    }

    // Streaming: the block arrives split across calls and the fence state has
    // to survive between them.
    #[test]
    fn renders_a_block_split_across_batches() {
        let rec = record("de");
        let (head, tail) = rec.split_at(rec.find('\n').expect("two lines") + 1);
        let mut state = State::empty();

        let first = rw(&format!("```nickel\n{head}"), &mut state);
        assert_eq!(first, "");
        assert!(state.inside);

        let out = rw(&format!("{tail}\n```\n"), &mut state);
        assert!(out.contains("## Fazit"), "{out}");
        assert!(!state.inside);
    }

    // An invalid record must still show something, so the reader is not left
    // with a hole where the answer was.
    #[test]
    fn an_invalid_record_falls_back_to_the_source() {
        let mut state = State::empty();
        let out = rw("```nickel\nrender {\n```\n", &mut state);
        assert!(out.contains("```nickel"), "{out}");
        assert!(out.contains("render {"), "{out}");
    }

    #[test]
    fn a_traversal_message_id_is_rejected() {
        assert_eq!(safe_message_id(Some("../../etc/passwd")), None);
        assert_eq!(safe_message_id(Some("")), None);
        assert_eq!(safe_message_id(None), None);
        assert_eq!(safe_message_id(Some("abc-123")).as_deref(), Some("abc-123"));
    }

    // An interrupted block is shown as source with its fence closed. An
    // unterminated fence would make the renderer treat the rest of the screen
    // as code.
    #[test]
    fn an_interrupted_block_closes_its_fence() {
        let mut out = String::new();
        append_source(&mut out, "render {\n");
        assert!(out.starts_with("```nickel\n"), "{out}");
        assert!(out.ends_with("```\n"), "unterminated fence: {out:?}");
    }

    // A block whose last line has no newline must not run its closing fence
    // onto the same line, where it would no longer close anything.
    #[test]
    fn append_source_separates_an_unterminated_last_line() {
        let mut out = String::new();
        append_source(&mut out, "render {");
        assert!(out.ends_with("render {\n```\n"), "{out:?}");
    }

    // Two blocks in one batch: the second must not erase the first. The preamble
    // drop may only discard prose that has not been rendered yet.
    #[test]
    fn a_second_block_does_not_erase_the_first() {
        let delta = format!(
            "```nickel\n{}\n```\nMIDDLE\n```nickel\n{}\n```\n",
            record("de"),
            record("en")
        );
        let mut state = State::empty();
        let out = rw(&delta, &mut state);
        assert!(out.contains("## Fazit"), "first block lost: {out}");
        assert!(out.contains("MIDDLE"), "prose between blocks lost: {out}");
        assert!(out.contains("## Bottom line"), "second block lost: {out}");
    }

    #[test]
    fn the_state_survives_a_round_trip() {
        let state = State {
            inside: true,
            block: "render {\n".to_string(),
            next: 7,
            passthrough: false,
        };
        assert_eq!(parse_state(&format_state(&state)), state);
        let passthrough = State {
            passthrough: true,
            ..State::empty()
        };
        assert_eq!(parse_state(&format_state(&passthrough)), passthrough);
        assert_eq!(parse_state(""), State::empty());
    }

    #[test]
    fn a_flush_ahead_of_its_turn_waits() {
        let mut state = State::empty();
        state.next = 1;
        assert!(matches!(turn(Some(2), &mut state), Turn::Wait));
        assert!(matches!(turn(Some(1), &mut state), Turn::Ready));
    }

    #[test]
    fn a_repeated_flush_passes_through() {
        let mut state = State::empty();
        state.next = 3;
        assert!(matches!(turn(Some(1), &mut state), Turn::PassThrough));
    }

    #[test]
    fn index_zero_resets_unless_given_up() {
        let mut state = State::empty();
        state.next = 5;
        state.inside = true;
        assert!(matches!(turn(Some(0), &mut state), Turn::Ready));
        assert_eq!(state, State::empty());

        let mut given_up = State {
            passthrough: true,
            next: 3,
            ..State::empty()
        };
        assert!(matches!(turn(Some(0), &mut given_up), Turn::PassThrough));
    }

    #[test]
    fn giving_up_restores_the_held_back_text() {
        let state = State {
            inside: true,
            block: "let x = 1 in\n".to_string(),
            next: 1,
            passthrough: false,
        };
        assert_eq!(
            give_up_text(&state, "render {\n", false),
            "```nickel\nlet x = 1 in\nrender {\n"
        );
        assert_eq!(give_up_text(&State::empty(), "prose\n", false), "prose\n");
    }

    // A final flush that gives up has no successor to close the reopened fence.
    // Left open, the renderer would treat the rest of the screen as code.
    #[test]
    fn a_final_give_up_closes_the_fence() {
        let state = State {
            inside: true,
            block: "let x = 1 in\n".to_string(),
            next: 1,
            passthrough: false,
        };
        assert_eq!(
            give_up_text(&state, "render {", true),
            "```nickel\nlet x = 1 in\nrender {\n```\n"
        );
        // A delta that closes the block itself gets no second fence.
        assert_eq!(
            give_up_text(&state, "render {}\n```\n", true),
            "```nickel\nlet x = 1 in\nrender {}\n```\n"
        );
    }

    // --- where the state lives ---------------------------------------------

    #[test]
    fn state_prefers_the_runtime_dir() {
        let dir = state_dir_from(
            Some(PathBuf::from("/run/user/1000")),
            Some("alice".into()),
            PathBuf::from("/tmp"),
        );
        assert_eq!(dir, PathBuf::from("/run/user/1000/n4t-render"));
    }

    #[test]
    fn state_falls_back_to_tmp_per_user() {
        let dir = state_dir_from(None, Some("alice".into()), PathBuf::from("/tmp"));
        assert_eq!(dir, PathBuf::from("/tmp/n4t-render-alice"));
    }

    // A relative runtime dir would put state wherever the hook happens to run.
    #[test]
    fn a_relative_runtime_dir_is_ignored() {
        let dir = state_dir_from(
            Some(PathBuf::from("run")),
            Some("alice".into()),
            PathBuf::from("/tmp"),
        );
        assert_eq!(dir, PathBuf::from("/tmp/n4t-render-alice"));
    }

    // The user name ends up in a path, so anything but a plain name is replaced.
    #[test]
    fn an_unsafe_user_name_is_replaced() {
        let dir = state_dir_from(None, Some("../etc".into()), PathBuf::from("/tmp"));
        assert_eq!(dir, PathBuf::from("/tmp/n4t-render-default"));
    }
}
