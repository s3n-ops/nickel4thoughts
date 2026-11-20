//! Golden-file tests for the Markdown rendering.
//!
//! The rendering is what the reader sees, so its exact shape is asserted:
//! heading order, blank lines, the leading pair of newlines, the indent on
//! continuation lines. Set N4T_UPDATE_GOLDEN=1 to rewrite the files after an
//! intentional change.

use n4t::config::{Config, UserFile, UserPack};
use n4t::{block, render};
use std::fs;
use std::path::PathBuf;

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.md"))
}

/// The shipped packs. The German one brings the headings the German golden
/// files show.
fn german() -> Config {
    Config::default()
}

/// The shipped packs plus user packs, given as (tag, source).
fn with_packs(packs: &[(&str, &str)]) -> Config {
    let packs: Vec<UserPack> = packs
        .iter()
        .map(|(tag, source)| UserPack {
            tag: tag.to_string(),
            file: UserFile {
                path: PathBuf::from(format!("<test>/{tag}.ncl")),
                source: source.to_string(),
            },
        })
        .collect();
    Config::from_files(None, &packs).expect("packs load")
}

/// Compares the rendering against its golden file, under the shipped packs.
fn assert_golden(name: &str, record: &str) {
    let config = german();
    let answer = block::evaluate(record, &config).expect("record evaluates");
    let rendered = render::render(&answer, &config);
    let path = golden_path(name);

    if std::env::var_os("N4T_UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        fs::write(&path, &rendered).expect("write golden");
        return;
    }

    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}; rerun with N4T_UPDATE_GOLDEN=1", path.display()));
    assert_eq!(rendered, expected, "rendering of {name} changed");
}

fn record(fields: &str) -> String {
    format!(
        "let {{ render, .. }} = import \"{}/render.ncl\" in\nrender {{\n{fields}\n}}\n",
        n4t::hooks::inject::schema_dir().display()
    )
}

#[test]
fn full_record_de() {
    assert_golden(
        "full-de",
        &record(
            "  bottom_line = \"Der Stop-Hook nennt die verletzte Regel.\",\n  \
             supporting_detail = \"Die Diagnose kommt als Struktur, nicht als Text.\",\n  \
             evidence = [\"`cargo test` meldet 115 Regeltests.\", \"Der Bericht enthält keine Rahmenzeichen.\"],\n  \
             caveats = [\"Das Blockierformat ist noch nicht im Betrieb geprüft.\"],\n  \
             next_action = \"Die Hooks registrieren und einen Tag beobachten.\",\n  \
             confidence = 'verified,\n  language = 'de,",
        ),
    );
}

#[test]
fn full_record_en() {
    assert_golden(
        "full-en",
        &record(
            "  bottom_line = \"The stop hook names the broken rule.\",\n  \
             supporting_detail = \"Diagnostics arrive as structs, not as text.\",\n  \
             evidence = [\"`cargo test` reports 115 rule tests.\"],\n  \
             caveats = [\"The blocking format is not yet verified in practice.\"],\n  \
             next_action = \"Register the hooks and watch for a day.\",\n  \
             confidence = 'high,\n  language = 'en,",
        ),
    );
}

/// Sections are dropped when empty, so the output has no hollow headings.
#[test]
fn minimal_record() {
    assert_golden(
        "minimal",
        &record("  bottom_line = \"Der Hook läuft.\",\n  confidence = 'low,\n  language = 'de,"),
    );
}

/// Entries may hold several lines. Continuation lines are indented so they stay
/// inside the bullet instead of ending the list.
#[test]
fn multiline_list_entry() {
    assert_golden(
        "multiline",
        &record(
            "  bottom_line = \"Der Block rendert mehrzeilig.\",\n  \
             evidence = [m%\"\nErste Zeile.\nZweite Zeile.\n\"%],\n  \
             confidence = 'medium,\n  language = 'de,",
        ),
    );
}

fn render_with(fields: &str, config: &Config) -> String {
    let answer = block::evaluate(&record(fields), config).expect("evaluates");
    render::render(&answer, config)
}

fn config_from(source: &str) -> Config {
    Config::parse(source, std::path::Path::new("<test>")).expect("config parses")
}

/// Every confidence tag has a label, in English and in a configured language.
#[test]
fn every_confidence_label_renders() {
    let config = german();
    for (tag, de, en) in [
        (
            "verified",
            "ausgeführt und beobachtet",
            "executed and observed",
        ),
        ("high", "hoch", "high"),
        ("medium", "mittel", "medium"),
        ("low", "gering", "low"),
    ] {
        let de_out = render_with(
            &format!("  bottom_line = \"Kurz.\",\n  confidence = '{tag},\n  language = 'de,"),
            &config,
        );
        assert!(
            de_out.contains(&format!("*Verlässlichkeit: {de}*")),
            "{de_out}"
        );

        let en_out = render_with(
            &format!("  bottom_line = \"Short.\",\n  confidence = '{tag},\n  language = 'en,"),
            &config,
        );
        assert!(en_out.contains(&format!("*Confidence: {en}*")), "{en_out}");
    }
}

/// Two leading newlines, so one blank line separates the first heading from
/// whatever precedes the rendering on screen.
#[test]
fn rendering_opens_with_two_newlines() {
    let out = render_with(
        "  bottom_line = \"Short.\",\n  confidence = 'high,",
        &Config::default(),
    );
    assert!(out.starts_with("\n\n## Bottom line"), "{out:?}");
}

/// Without `language` and without a config, the headings are English.
#[test]
fn the_default_language_is_english() {
    let out = render_with(
        "  bottom_line = \"Short.\",\n  confidence = 'high,",
        &Config::default(),
    );
    assert!(out.contains("## Bottom line"), "{out}");
}

/// The config can make another language the default.
#[test]
fn the_config_sets_the_default_language() {
    let config = config_from("{ default_language = 'de }");
    let out = render_with("  bottom_line = \"Kurz.\",\n  confidence = 'high,", &config);
    assert!(out.contains("## Fazit"), "{out}");
}

/// A pack for a new language: its labels apply, the ones it leaves out stay
/// English.
#[test]
fn pack_labels_override_what_they_name() {
    let config = with_packs(&[(
        "fr",
        "{ labels = { bottom_line = \"En bref\", high = \"élevée\" } }",
    )]);
    let out = render_with(
        "  bottom_line = \"Court.\",\n  caveats = [\"Rien.\"],\n  confidence = 'high,\n  language = 'fr,",
        &config,
    );
    assert!(out.contains("## En bref"), "{out}");
    assert!(out.contains("## Caveats"), "{out}");
    assert!(out.contains("*Confidence: élevée*"), "{out}");
}

/// A user pack replaces the shipped headings it names and keeps the others.
#[test]
fn a_user_pack_overrides_shipped_headings() {
    let config = with_packs(&[("de", "{ labels = { bottom_line = \"Kernaussage\" } }")]);
    let out = render_with(
        "  bottom_line = \"Kurz.\",\n  caveats = [\"Keine.\"],\n  confidence = 'high,\n  language = 'de,",
        &config,
    );
    assert!(out.contains("## Kernaussage"), "{out}");
    assert!(out.contains("## Einschränkungen"), "{out}");
}

/// A language without labels still renders, in English.
#[test]
fn an_unknown_language_falls_back_to_english() {
    let out = render_with(
        "  bottom_line = \"Kort.\",\n  confidence = 'high,\n  language = 'nl,",
        &Config::default(),
    );
    assert!(out.contains("## Bottom line"), "{out}");
}

/// The rule text lists every language with a pack, once each.
#[test]
fn languages_include_every_pack() {
    let config = with_packs(&[("fr", "{}"), ("de", "{}")]);
    assert_eq!(render::languages(&config), vec!["de", "en", "fr"]);
}
