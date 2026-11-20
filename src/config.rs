//! The user's settings and the language packs.
//!
//! Two kinds of file live in the n4t config directory:
//!
//! - `config.ncl` holds the limits: word budgets per field, the sentence
//!   length and the default language. Without it the defaults below apply.
//!   They are the only place the numbers are written down; the rule text, the
//!   validation and the skeleton file all follow them, and a test keeps the
//!   skeleton in line.
//! - `languages/<tag>.ncl` holds a language pack: headings, phrase lists for
//!   the prose rules and optional checks written in Nickel. n4t ships packs
//!   for English and German. A pack of the same name in the directory lays its
//!   headings and lists over the shipped ones, one by one.
//!
//! Everything is evaluated in one Nickel program. A second evaluation would
//! load the Nickel standard library a second time, which is the bulk of the
//! cost.
//!
//! A broken file does not break the hooks. They fall back to the defaults, so
//! the rule Claude reads and the rule the Stop hook enforces stay the same, and
//! the rule text names the problem so it does not go unnoticed.

use crate::nickel::{eval_json, EvalError};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Default word budget of `bottom_line`.
pub const DEFAULT_BOTTOM_LINE: usize = 100;
/// Default word budget of `supporting_detail`.
pub const DEFAULT_SUPPORTING_DETAIL: usize = 200;
/// Default word budget of each `evidence` entry.
pub const DEFAULT_EVIDENCE: usize = 40;
/// Default word budget of each `caveats` entry.
pub const DEFAULT_CAVEATS: usize = 40;
/// Default word budget of `next_action`.
pub const DEFAULT_NEXT_ACTION: usize = 50;
/// Default maximum number of words in one sentence.
pub const DEFAULT_SENTENCE_LENGTH: usize = 40;
/// Language of the headings when an answer does not set `language`.
pub const DEFAULT_LANGUAGE: &str = "en";

/// The active config file inside the config directory. `just install` never
/// writes it.
pub const CONFIG_FILE: &str = "config.ncl";

/// The directory of the user's language packs, inside the config directory.
pub const PACK_DIR: &str = "languages";

/// The packs n4t ships, embedded so the binary needs no files besides the
/// schema. Sorted by tag.
pub const BUILTIN_PACKS: [(&str, &str); 2] = [
    ("de", include_str!("../languages/de.ncl")),
    ("en", include_str!("../languages/en.ncl")),
];

/// A phrase longer than this is no phrase but a sentence.
const MAX_PHRASE_CHARS: usize = 100;
/// More entries than this in one list point to a mistake, and every entry
/// becomes part of a regular expression.
const MAX_PHRASES: usize = 500;

/// Word budgets per field. A field left out of the file keeps its default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Budgets {
    pub bottom_line: usize,
    pub supporting_detail: usize,
    pub evidence: usize,
    pub caveats: usize,
    pub next_action: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Budgets {
            bottom_line: DEFAULT_BOTTOM_LINE,
            supporting_detail: DEFAULT_SUPPORTING_DETAIL,
            evidence: DEFAULT_EVIDENCE,
            caveats: DEFAULT_CAVEATS,
            next_action: DEFAULT_NEXT_ACTION,
        }
    }
}

/// Lays `own` over `base`, one field at a time.
macro_rules! overlay {
    ($own:expr, $base:expr, [$($field:ident),*]) => {
        Self { $($field: $own.$field.clone().or_else(|| $base.$field.clone()),)* }
    };
}

/// Headings and confidence labels for one language. A label left out keeps
/// the built-in English text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Labels {
    pub bottom_line: Option<String>,
    pub supporting_detail: Option<String>,
    pub evidence: Option<String>,
    pub caveats: Option<String>,
    pub next_action: Option<String>,
    pub confidence: Option<String>,
    pub verified: Option<String>,
    pub high: Option<String>,
    pub medium: Option<String>,
    pub low: Option<String>,
}

impl Labels {
    fn over(&self, base: &Labels) -> Labels {
        overlay!(
            self,
            base,
            [
                bottom_line,
                supporting_detail,
                evidence,
                caveats,
                next_action,
                confidence,
                verified,
                high,
                medium,
                low
            ]
        )
    }
}

/// Phrase lists for one language, one per rule category. What a category
/// means, which fields it applies to and what the violation says is fixed in
/// `src/rules.rs`; a pack supplies only the words.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhraseLists {
    pub preamble: Option<Vec<String>>,
    pub question_comment: Option<Vec<String>>,
    pub summary: Option<Vec<String>>,
    pub hedging: Option<Vec<String>>,
    pub forward_reference: Option<Vec<String>>,
    pub meta: Option<Vec<String>>,
    pub apology: Option<Vec<String>>,
    pub filler: Option<Vec<String>>,
    pub spelling: Option<Vec<String>>,
}

impl PhraseLists {
    fn over(&self, base: &PhraseLists) -> PhraseLists {
        overlay!(
            self,
            base,
            [
                preamble,
                question_comment,
                summary,
                hedging,
                forward_reference,
                meta,
                apology,
                filler,
                spelling
            ]
        )
    }

    /// Every list with its category name.
    pub fn lists(&self) -> [(&'static str, &[String]); 9] {
        fn l(list: &Option<Vec<String>>) -> &[String] {
            list.as_deref().unwrap_or(&[])
        }
        [
            ("preamble", l(&self.preamble)),
            ("question_comment", l(&self.question_comment)),
            ("summary", l(&self.summary)),
            ("hedging", l(&self.hedging)),
            ("forward_reference", l(&self.forward_reference)),
            ("meta", l(&self.meta)),
            ("apology", l(&self.apology)),
            ("filler", l(&self.filler)),
            ("spelling", l(&self.spelling)),
        ]
    }

    /// A phrase must carry a letter or a digit: one made of punctuation alone,
    /// such as ".", would match almost every text and block every answer.
    fn check(&self) -> Result<(), String> {
        for (category, list) in self.lists() {
            if list.len() > MAX_PHRASES {
                return Err(format!(
                    "`phrases.{category}` has {} entries, at most {MAX_PHRASES} are allowed",
                    list.len()
                ));
            }
            for phrase in list {
                if !phrase.chars().any(char::is_alphanumeric) {
                    return Err(format!(
                        "`phrases.{category}` holds {phrase:?}, which has no letter or digit"
                    ));
                }
                if phrase.chars().count() > MAX_PHRASE_CHARS {
                    return Err(format!(
                        "`phrases.{category}` holds a phrase longer than {MAX_PHRASE_CHARS} characters"
                    ));
                }
            }
            if !crate::rules::fits(list) {
                return Err(format!(
                    "`phrases.{category}` is too large to match against; shorten or split the list"
                ));
            }
        }
        Ok(())
    }
}

/// Where the checks of a language come from. The Stop hook evaluates them
/// with the answer's prose; the config only records where they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackSource {
    Builtin {
        tag: &'static str,
        source: &'static str,
    },
    File(PathBuf),
}

impl PackSource {
    /// The pack's Nickel source. A user pack is read again here, since only
    /// the Stop hook needs it and only when the language has checks.
    pub fn source(&self) -> std::io::Result<String> {
        match self {
            PackSource::Builtin { source, .. } => Ok(source.to_string()),
            PackSource::File(path) => std::fs::read_to_string(path),
        }
    }

    /// The name the pack goes by in Nickel's messages.
    pub fn name(&self) -> String {
        match self {
            PackSource::Builtin { tag, .. } => format!("<built-in {tag}.ncl>"),
            PackSource::File(path) => path.display().to_string(),
        }
    }
}

/// Everything n4t knows about one language, shipped and user packs merged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Language {
    pub labels: Labels,
    pub phrases: PhraseLists,
    /// The pack whose `checks` apply, if any. A user pack with checks
    /// replaces the shipped ones as a whole.
    pub checks: Option<PackSource>,
    /// Where the language comes from, for `n4t config`.
    pub origin: Vec<String>,
}

/// The settings the hooks apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub budgets: Budgets,
    pub sentence_length: usize,
    /// An enum tag in the file (`'de`), a plain string after export.
    pub default_language: String,
    /// Every language with a pack, keyed by its tag.
    pub languages: BTreeMap<String, Language>,
}

/// What `config.ncl` may set. Unknown keys are rejected, so a misspelt key is
/// reported instead of being ignored in silence.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct Settings {
    budgets: Budgets,
    sentence_length: usize,
    default_language: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            budgets: Budgets::default(),
            sentence_length: DEFAULT_SENTENCE_LENGTH,
            default_language: DEFAULT_LANGUAGE.to_string(),
        }
    }
}

/// One pack as the evaluation exports it: its data, and whether it defines
/// checks. The checks themselves are functions and cannot be exported.
#[derive(Debug, Deserialize)]
struct Probe {
    pack: Pack,
    has_checks: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct Pack {
    labels: Labels,
    phrases: PhraseLists,
}

#[derive(Debug, Deserialize)]
struct Evaluated {
    user: serde_json::Value,
    builtin: BTreeMap<String, Probe>,
    /// Kept as JSON until each pack is read on its own, so an error names the
    /// pack's file rather than the config.
    packs: BTreeMap<String, serde_json::Value>,
}

/// A config file or pack that exists but cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub path: PathBuf,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

/// A user file read from disk, with the name it is reported under.
#[derive(Debug, Clone)]
pub struct UserFile {
    pub path: PathBuf,
    pub source: String,
}

/// A user pack: its language tag and its file.
#[derive(Debug, Clone)]
pub struct UserPack {
    pub tag: String,
    pub file: UserFile,
}

/// Wraps a pack so that its data can be exported. `std.record.filter` does not
/// force the values it drops, so the check functions are never evaluated here.
const PROBE: &str = r#"let probe = fun p => {
  pack = std.record.filter (fun k _v => k != "checks") p,
  has_checks = std.record.has_field "checks" p,
} in
"#;

/// The single program that evaluates the config and every pack. Each part
/// keeps its own source; the line breaks around it end a trailing comment.
fn compose(config: Option<&str>, packs: &[(&str, &str)]) -> String {
    let mut out = String::from(PROBE);
    out.push_str("{\n  user = (\n");
    out.push_str(config.unwrap_or("{}"));
    out.push_str("\n),\n  builtin = {\n");
    for (tag, source) in BUILTIN_PACKS {
        out.push_str(&format!("    \"{tag}\" = probe (\n{source}\n),\n"));
    }
    out.push_str("  },\n  packs = {\n");
    for (tag, source) in packs {
        out.push_str(&format!("    \"{tag}\" = probe (\n{source}\n),\n"));
    }
    out.push_str("  },\n}\n");
    out
}

/// A pack's file name is its language tag. Anything else in the directory is
/// left alone, and the tag can be quoted as a Nickel field name safely.
fn pack_tag(path: &Path) -> Option<String> {
    if path.extension()? != "ncl" {
        return None;
    }
    let tag = path.file_stem()?.to_str()?;
    let mut chars = tag.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| tag.to_string())
}

impl Config {
    /// Parses the contents of a config file, with the shipped packs only.
    pub fn parse(source: &str, path: &Path) -> Result<Config, ConfigError> {
        let file = UserFile {
            path: path.to_path_buf(),
            source: source.to_string(),
        };
        Config::from_files(Some(&file), &[])
    }

    /// Builds the config from the user's files. Each is optional.
    pub fn from_files(
        config: Option<&UserFile>,
        packs: &[UserPack],
    ) -> Result<Config, ConfigError> {
        let pack_sources: Vec<(&str, &str)> = packs
            .iter()
            .map(|p| (p.tag.as_str(), p.file.source.as_str()))
            .collect();
        let program = compose(config.map(|c| c.source.as_str()), &pack_sources);
        let name = config.map_or_else(
            || "<language packs>".to_string(),
            |c| c.path.display().to_string(),
        );

        let json = match eval_json(&program, &name) {
            Ok(json) => json,
            // Finding the culprit evaluates each file again, and a runaway
            // file would use up a second timeout. Two of them would push the
            // hook past the 5 s Claude Code allows it.
            Err(err @ EvalError::Timeout) => {
                return Err(ConfigError {
                    path: PathBuf::from(COMBINED),
                    message: err.diagnosis(),
                })
            }
            Err(err) => return Err(locate_error(config, packs, err.diagnosis())),
        };
        let evaluated: Evaluated = serde_json::from_value(json).map_err(|e| ConfigError {
            path: PathBuf::from(&name),
            message: e.to_string(),
        })?;

        let config_path = config.map_or_else(|| PathBuf::from(CONFIG_FILE), |c| c.path.clone());
        let settings = settings_from(evaluated.user, &config_path)?;

        let mut languages = BTreeMap::new();
        for (tag, probe) in evaluated.builtin {
            let builtin = BUILTIN_PACKS
                .iter()
                .find(|(t, _)| *t == tag)
                .map(|&(tag, source)| PackSource::Builtin { tag, source });
            languages.insert(
                tag.clone(),
                Language {
                    labels: probe.pack.labels,
                    phrases: probe.pack.phrases,
                    checks: builtin.filter(|_| probe.has_checks),
                    origin: vec!["built-in".to_string()],
                },
            );
        }
        for user in packs {
            let Some(value) = evaluated.packs.get(&user.tag) else {
                continue;
            };
            let probe: Probe = serde_json::from_value(value.clone()).map_err(|e| ConfigError {
                path: user.file.path.clone(),
                message: e.to_string(),
            })?;
            let entry: &mut Language = languages.entry(user.tag.clone()).or_default();
            entry.labels = probe.pack.labels.over(&entry.labels);
            entry.phrases = probe.pack.phrases.over(&entry.phrases);
            if probe.has_checks {
                entry.checks = Some(PackSource::File(user.file.path.clone()));
            }
            entry.origin.push(user.file.path.display().to_string());
            probe.pack.phrases.check().map_err(|message| ConfigError {
                path: user.file.path.clone(),
                message,
            })?;
        }

        let config = Config {
            budgets: settings.budgets,
            sentence_length: settings.sentence_length,
            default_language: settings.default_language,
            languages,
        };
        config.check().map_err(|message| ConfigError {
            path: config_path,
            message,
        })?;
        Ok(config)
    }

    /// Every limit must allow at least one word. A zero budget would block
    /// every answer that uses the field, which is never what was meant.
    fn check(&self) -> Result<(), String> {
        let b = &self.budgets;
        let limits = [
            ("budgets.bottom_line", b.bottom_line),
            ("budgets.supporting_detail", b.supporting_detail),
            ("budgets.evidence", b.evidence),
            ("budgets.caveats", b.caveats),
            ("budgets.next_action", b.next_action),
            ("sentence_length", self.sentence_length),
        ];
        if let Some((name, _)) = limits.iter().find(|(_, value)| *value == 0) {
            return Err(format!("`{name}` must be at least 1"));
        }
        if self.default_language.is_empty() {
            return Err("`default_language` must name a language".to_string());
        }
        Ok(())
    }

    /// The pack data for the language of an answer, if there is a pack.
    pub fn language(&self, tag: &str) -> Option<&Language> {
        self.languages.get(tag)
    }
}

/// Reads the settings out of the evaluated `config.ncl`. Headings used to live
/// there, so a config that still carries them gets told where they went.
fn settings_from(user: serde_json::Value, path: &Path) -> Result<Settings, ConfigError> {
    let error = |message: String| ConfigError {
        path: path.to_path_buf(),
        message,
    };
    if user.get("labels").is_some() {
        return Err(error(format!(
            "`labels` moved to the language packs: put the headings of a language into \
             `{PACK_DIR}/<tag>.ncl` next to this file"
        )));
    }
    serde_json::from_value(user).map_err(|e| error(e.to_string()))
}

/// The name of the config and the packs together, when no single file can be
/// blamed.
const COMBINED: &str = "<config.ncl and language packs>";

/// Names the file a failed evaluation comes from.
///
/// The config and the packs are evaluated together, so Nickel's message
/// cannot say which file it is about. Only on this error path each user file
/// is evaluated on its own until one fails.
fn locate_error(config: Option<&UserFile>, packs: &[UserPack], message: String) -> ConfigError {
    let culprit = config
        .filter(|c| eval_json(&c.source, &c.path.display().to_string()).is_err())
        .map(|c| c.path.clone())
        .or_else(|| {
            packs
                .iter()
                .find(|p| {
                    let program = format!("{PROBE}probe (\n{}\n)", p.file.source);
                    eval_json(&program, &p.file.path.display().to_string()).is_err()
                })
                .map(|p| p.file.path.clone())
        })
        .unwrap_or_else(|| PathBuf::from("<language packs>"));
    ConfigError {
        path: culprit,
        message,
    }
}

/// The shipped packs with default limits, evaluated once per process.
static BUILTIN: LazyLock<Config> =
    LazyLock::new(|| Config::from_files(None, &[]).expect("the shipped language packs are valid"));

impl Default for Config {
    fn default() -> Self {
        BUILTIN.clone()
    }
}

/// The config directory: `N4T_CONFIG_DIR` first, then the XDG location.
pub fn config_dir() -> Option<PathBuf> {
    config_dir_from(
        std::env::var_os("N4T_CONFIG_DIR").map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// Picks the config directory from the environment it is given. Relative
/// paths are ignored, as the XDG specification requires for its variables.
fn config_dir_from(
    explicit: Option<PathBuf>,
    xdg: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(dir) = explicit.filter(|d| d.is_absolute()) {
        return Some(dir);
    }
    if let Some(dir) = xdg.filter(|d| d.is_absolute()) {
        return Some(dir.join("n4t"));
    }
    home.filter(|d| d.is_absolute())
        .map(|d| d.join(".config").join("n4t"))
}

/// The path of the active config file, if a config directory can be found.
pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(CONFIG_FILE))
}

/// Loads the active config. Missing files are no error: the defaults and the
/// shipped packs apply.
pub fn load() -> Result<Config, ConfigError> {
    let Some(dir) = config_dir() else {
        return Ok(Config::default());
    };
    load_from(&dir)
}

fn load_from(dir: &Path) -> Result<Config, ConfigError> {
    let config = read_optional(&dir.join(CONFIG_FILE))?;
    let packs = read_packs(&dir.join(PACK_DIR))?;
    if config.is_none() && packs.is_empty() {
        return Ok(Config::default());
    }
    Config::from_files(config.as_ref(), &packs)
}

fn read_optional(path: &Path) -> Result<Option<UserFile>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(source) => Ok(Some(UserFile {
            path: path.to_path_buf(),
            source,
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ConfigError {
            path: path.to_path_buf(),
            message: e.to_string(),
        }),
    }
}

/// The user's packs, sorted by tag so the result does not depend on the order
/// the file system lists them in.
fn read_packs(dir: &Path) -> Result<Vec<UserPack>, ConfigError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(ConfigError {
                path: dir.to_path_buf(),
                message: e.to_string(),
            })
        }
    };
    let mut packs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(tag) = pack_tag(&path) else {
            continue;
        };
        if let Some(file) = read_optional(&path)? {
            packs.push(UserPack { tag, file });
        }
    }
    packs.sort_by(|a, b| a.tag.cmp(&b.tag));
    Ok(packs)
}

/// The config the hooks use: the files when they are valid, the defaults
/// otherwise.
pub fn load_or_default() -> Config {
    load().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Result<Config, ConfigError> {
        Config::parse(source, Path::new("<test>"))
    }

    fn with_pack(tag: &str, source: &str) -> Result<Config, ConfigError> {
        let pack = UserPack {
            tag: tag.to_string(),
            file: UserFile {
                path: PathBuf::from(format!("<test>/{tag}.ncl")),
                source: source.to_string(),
            },
        };
        Config::from_files(None, &[pack])
    }

    #[test]
    fn an_empty_record_keeps_every_default() {
        assert_eq!(parse("{}").unwrap(), Config::default());
    }

    #[test]
    fn a_partial_record_overrides_only_what_it_names() {
        let config = parse("{ budgets = { bottom_line = 150 } }").unwrap();
        assert_eq!(config.budgets.bottom_line, 150);
        assert_eq!(config.budgets.supporting_detail, DEFAULT_SUPPORTING_DETAIL);
        assert_eq!(config.sentence_length, DEFAULT_SENTENCE_LENGTH);
    }

    #[test]
    fn a_misspelt_key_is_reported() {
        let err = parse("{ budgets = { bottom_lin = 150 } }").unwrap_err();
        assert!(err.message.contains("bottom_lin"), "{err}");
    }

    #[test]
    fn a_zero_limit_is_rejected() {
        let err = parse("{ sentence_length = 0 }").unwrap_err();
        assert!(err.message.contains("sentence_length"), "{err}");
    }

    #[test]
    fn a_negative_limit_is_rejected() {
        assert!(parse("{ sentence_length = -5 }").is_err());
    }

    #[test]
    fn a_syntax_error_is_reported() {
        let err = parse("{ budgets = ").unwrap_err();
        assert_eq!(err.path, PathBuf::from("<test>"), "{err}");
    }

    #[test]
    fn the_default_language_is_read() {
        let config = parse("{ default_language = 'fr }").unwrap();
        assert_eq!(config.default_language, "fr");
    }

    // Headings moved from config.ncl into the packs. A config written before
    // that is told where they went instead of failing on an unknown key.
    #[test]
    fn labels_in_the_config_point_to_the_packs() {
        let err = parse("{ labels = { de = { bottom_line = \"Fazit\" } } }").unwrap_err();
        assert!(err.message.contains("languages/<tag>.ncl"), "{err}");
    }

    #[test]
    fn the_shipped_packs_are_loaded() {
        let config = Config::default();
        let de = config.language("de").expect("de pack");
        assert_eq!(de.labels.bottom_line.as_deref(), Some("Fazit"));
        assert!(de
            .phrases
            .hedging
            .as_ref()
            .unwrap()
            .contains(&"vermutlich".to_string()));
        let en = config.language("en").expect("en pack");
        assert!(en
            .phrases
            .filler
            .as_ref()
            .unwrap()
            .contains(&"basically".to_string()));
    }

    // The German pack derives the ASCII spellings with Nickel code.
    #[test]
    fn the_german_pack_derives_ascii_spellings() {
        let de = Config::default().languages["de"].clone();
        let preamble = de.phrases.preamble.unwrap();
        assert!(preamble.contains(&"zunaechst".to_string()), "{preamble:?}");
        assert!(de.phrases.spelling.unwrap().contains(&"fuer".to_string()));
    }

    #[test]
    fn a_user_pack_adds_a_language() {
        let config = with_pack(
            "fr",
            "{ labels = { bottom_line = \"En bref\" }, phrases = { hedging = [\"peut-être\"] } }",
        )
        .unwrap();
        let fr = config.language("fr").expect("fr pack");
        assert_eq!(fr.labels.bottom_line.as_deref(), Some("En bref"));
        assert_eq!(
            fr.phrases.hedging.as_deref(),
            Some(&["peut-être".to_string()][..])
        );
    }

    // A user pack replaces what it names and keeps the rest of the shipped one.
    #[test]
    fn a_user_pack_overrides_one_list_at_a_time() {
        let config = with_pack("de", "{ phrases = { hedging = [\"wohl\"] } }").unwrap();
        let de = config.language("de").unwrap();
        assert_eq!(
            de.phrases.hedging.as_deref(),
            Some(&["wohl".to_string()][..])
        );
        assert!(de
            .phrases
            .filler
            .as_ref()
            .unwrap()
            .contains(&"letztendlich".to_string()));
        assert_eq!(de.labels.bottom_line.as_deref(), Some("Fazit"));
    }

    #[test]
    fn a_pack_with_checks_is_recorded() {
        let config = with_pack("fr", "{ checks = { x = fun _t => 'Ok } }").unwrap();
        let fr = config.language("fr").unwrap();
        assert!(matches!(fr.checks, Some(PackSource::File(_))));
        assert_eq!(Config::default().languages["de"].checks, None);
    }

    // A phrase without a letter or digit would match almost every text.
    #[test]
    fn a_punctuation_phrase_is_rejected() {
        let err = with_pack("fr", "{ phrases = { filler = [\".\"] } }").unwrap_err();
        assert!(err.message.contains("no letter or digit"), "{err}");
        assert_eq!(err.path, PathBuf::from("<test>/fr.ncl"));
    }

    // The compiled size depends on the script. 500 Greek phrases of 100
    // characters stay within the count limits but not within the pattern
    // size, and the rule used to switch off without a word.
    #[test]
    fn a_list_too_large_to_compile_is_rejected() {
        let phrase = "αβγδεζηθικ".repeat(10);
        let list = (0..MAX_PHRASES)
            .map(|i| format!("\"{phrase}{i}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let err =
            with_pack("el", &format!("{{ phrases = {{ hedging = [{list}] }} }}")).unwrap_err();
        assert!(err.message.contains("phrases.hedging"), "{err}");
    }

    #[test]
    fn a_misspelt_category_is_reported() {
        let err = with_pack("fr", "{ phrases = { hedgeing = [\"peut-être\"] } }").unwrap_err();
        assert!(err.message.contains("hedgeing"), "{err}");
    }

    // The packs are evaluated with the config and reported under its name,
    // so a pack's unknown key used to be blamed on config.ncl.
    #[test]
    fn an_unknown_pack_key_names_the_pack() {
        let config = UserFile {
            path: PathBuf::from("<test>/config.ncl"),
            source: "{}".to_string(),
        };
        let pack = UserPack {
            tag: "fr".to_string(),
            file: UserFile {
                path: PathBuf::from("<test>/fr.ncl"),
                source: "{ label = { bottom_line = \"En bref\" } }".to_string(),
            },
        };
        let err = Config::from_files(Some(&config), &[pack]).unwrap_err();
        assert_eq!(err.path, PathBuf::from("<test>/fr.ncl"), "{err}");
        assert!(err.message.contains("label"), "{err}");
    }

    // The evaluation is shared, so a broken pack has to be named on its own.
    #[test]
    fn a_broken_pack_is_named() {
        let err = with_pack("fr", "{ phrases = ").unwrap_err();
        assert_eq!(err.path, PathBuf::from("<test>/fr.ncl"), "{err}");
    }

    // A runaway pack used to be evaluated a second time to name it, which
    // doubled the time the hook waited.
    #[test]
    fn a_runaway_pack_costs_one_timeout() {
        let started = std::time::Instant::now();
        let err = with_pack(
            "fr",
            "{ phrases = { hedging = let rec f = fun x => f x in f 1 } }",
        )
        .unwrap_err();
        assert!(err.message.contains("longer than"), "{err}");
        let limit = crate::nickel::EVAL_TIMEOUT + std::time::Duration::from_secs(1);
        assert!(started.elapsed() < limit, "{:?}", started.elapsed());
    }

    #[test]
    fn pack_tags_come_from_file_names() {
        assert_eq!(pack_tag(Path::new("/x/fr.ncl")).as_deref(), Some("fr"));
        assert_eq!(
            pack_tag(Path::new("/x/pt-BR.ncl")).as_deref(),
            Some("pt-BR")
        );
        assert_eq!(pack_tag(Path::new("/x/README.md")), None);
        assert_eq!(pack_tag(Path::new("/x/f\"r.ncl")), None);
        assert_eq!(pack_tag(Path::new("/x/1x.ncl")), None);
    }

    #[test]
    fn a_missing_directory_means_defaults() {
        let dir = Path::new("/nonexistent/n4t");
        assert_eq!(load_from(dir).unwrap(), Config::default());
    }

    /// The skeleton shows the defaults. If they drift apart, a user who copies
    /// the skeleton changes limits without meaning to.
    #[test]
    fn the_skeleton_states_the_defaults() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/config.skel.ncl");
        let source = std::fs::read_to_string(&path).expect("skeleton exists");
        assert_eq!(Config::parse(&source, &path).unwrap(), Config::default());
    }

    #[test]
    fn the_explicit_directory_wins() {
        let dir = config_dir_from(
            Some(PathBuf::from("/etc/n4t")),
            Some(PathBuf::from("/xdg")),
            Some(PathBuf::from("/home/u")),
        );
        assert_eq!(dir, Some(PathBuf::from("/etc/n4t")));
    }

    #[test]
    fn xdg_comes_before_home() {
        let dir = config_dir_from(
            None,
            Some(PathBuf::from("/xdg")),
            Some(PathBuf::from("/home/u")),
        );
        assert_eq!(dir, Some(PathBuf::from("/xdg/n4t")));
    }

    #[test]
    fn home_is_the_fallback() {
        let dir = config_dir_from(None, None, Some(PathBuf::from("/home/u")));
        assert_eq!(dir, Some(PathBuf::from("/home/u/.config/n4t")));
    }

    #[test]
    fn relative_paths_are_ignored() {
        let dir = config_dir_from(
            Some(PathBuf::from("rel")),
            Some(PathBuf::from("rel")),
            Some(PathBuf::from("/home/u")),
        );
        assert_eq!(dir, Some(PathBuf::from("/home/u/.config/n4t")));
    }
}
