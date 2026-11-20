//! Rule tests, ported one to one from the v1 prototype.
//!
//! Each case states a string, the rule, and the expected outcome. They are the
//! assurance that porting the rules from Nickel to Rust changed nothing.
//!
//! The second half matters more than the first: a rule that blocks ordinary
//! sentences is worse than one that misses a violation, because it costs a turn
//! every time it fires.

use n4t::config::{self, Config};
use n4t::prose::{prose_word_count, word_count};
use n4t::rules::{self, Check, Phrases};
use std::sync::LazyLock;

/// Both shipped packs at once. The phrase rules applied English and German
/// together before they moved into language packs, and the cases below were
/// written against that.
static BOTH: LazyLock<Phrases> = LazyLock::new(|| {
    let config = Config::default();
    Phrases::compile(&[
        &config.languages["de"].phrases,
        &config.languages["en"].phrases,
    ])
});

fn p() -> &'static Phrases {
    &BOTH
}

fn passes(check: Check) -> bool {
    check.is_ok()
}

macro_rules! rejects {
    ($name:ident, $rule:expr) => {
        #[test]
        fn $name() {
            assert!(!passes($rule), "should have been rejected");
        }
    };
}

macro_rules! accepts {
    ($name:ident, $rule:expr) => {
        #[test]
        fn $name() {
            assert!(passes($rule), "should have passed");
        }
    };
}

// --- must be rejected ----------------------------------------------------

rejects!(preamble_de, rules::no_preamble(p(), "Zunächst der Befund."));
rejects!(preamble_en, rules::no_preamble(p(), "First, some context."));
rejects!(
    question_comment,
    rules::no_preamble(p(), "Gute Frage, hier die Antwort.")
);
rejects!(
    summary_announce,
    rules::no_preamble(p(), "Kurz gesagt: es läuft.")
);
rejects!(
    hedge_de,
    rules::no_hedging(p(), "Das ist vermutlich korrekt.")
);
rejects!(hedge_en, rules::no_hedging(p(), "I believe this works."));
rejects!(trailing_question, rules::is_statement("Läuft der Hook?"));
rejects!(
    forward_reference,
    rules::self_contained(p(), "Der Hook läuft, siehe unten.")
);
rejects!(
    meta_comment,
    rules::no_meta(p(), "Wie bereits erwähnt läuft er.")
);
rejects!(
    apology,
    rules::no_meta(p(), "Entschuldigung, das war falsch.")
);
rejects!(
    filler_de,
    rules::no_filler(p(), "Im Grunde genommen läuft es.")
);
rejects!(
    filler_en,
    rules::no_filler(p(), "It is important to note that it runs.")
);

// --- must pass: ordinary, correct prose ----------------------------------

accepts!(plain_claim, rules::no_preamble(p(), "Der Hook ist aktiv."));
accepts!(
    no_hedge_present,
    rules::no_hedging(p(), "Der Test lief durch.")
);
accepts!(
    question_inside,
    rules::is_statement("Ob er läuft, zeigt der Test: ja.")
);
accepts!(
    no_reference,
    rules::self_contained(p(), "Beide Rezepte sind idempotent.")
);
accepts!(
    no_meta_present,
    rules::no_meta(p(), "Die Registrierung liegt in der User-Ebene.")
);
accepts!(
    no_filler_present,
    rules::no_filler(p(), "Sieben Fälle wurden geprüft.")
);

// --- word boundaries: substrings must not trigger ------------------------

accepts!(
    maybe_inside_word,
    rules::no_hedging(p(), "Die Funktion maybeify lief.")
);
accepts!(
    first_inside_word,
    rules::no_preamble(p(), "Firstname wurde gesetzt.")
);

// --- sentence length -----------------------------------------------------

accepts!(
    short_sentences,
    rules::sentence_length(40, "Kurz. Und knapp. Fertig.")
);
rejects!(
    long_sentence,
    rules::sentence_length(5, "Dieser eine Satz ist deutlich zu lang geraten.")
);
accepts!(
    many_short_sentences,
    rules::sentence_length(5, "Eins zwei. Drei vier. Fünf sechs.")
);

// --- combined rule sets: report the first failure, pass when all hold ----

rejects!(
    combine_rejects,
    rules::bluf(p(), config::DEFAULT_SENTENCE_LENGTH, "Vermutlich ja.")
);
accepts!(
    combine_accepts,
    rules::bluf(p(), config::DEFAULT_SENTENCE_LENGTH, "Der Test lief durch.")
);

// --- word count ----------------------------------------------------------

#[test]
fn counts_words() {
    assert_eq!(word_count("eins zwei drei"), 3);
}

#[test]
fn ignores_extra_space() {
    assert_eq!(word_count("  eins   zwei  "), 2);
}

#[test]
fn empty_is_zero() {
    assert_eq!(word_count("   "), 0);
}

// --- Markdown: markup must not trip the prose rules ----------------------
// Code carries someone else's wording. Judging it as prose is wrong.

accepts!(
    hedge_word_in_code_span,
    rules::no_hedging(p(), "Setze `maybe = true`.")
);
accepts!(
    hedge_in_fenced_block,
    rules::no_hedging(p(), "So:\n\n```py\n# I think this is wrong\nx = 1\n```")
);
accepts!(
    reference_in_path,
    rules::self_contained(p(), "Datei `docs/siehe unten.md` fehlt.")
);
accepts!(
    long_line_in_code_block,
    rules::sentence_length(
        10,
        "Der Befehl:\n\n```\neins zwei drei vier fünf sechs sieben acht neun zehn elf zwölf\n```"
    )
);

// Prose next to markup still gets judged.

rejects!(
    hedge_beside_code_span,
    rules::no_hedging(p(), "Der Wert `x` ist vermutlich falsch.")
);
rejects!(
    hedge_in_list_item,
    rules::no_hedging(p(), "Befunde:\n\n- Vermutlich aktiv")
);
rejects!(
    filler_in_table_cell,
    rules::no_filler(p(), "| a |\n|---|\n| Im Grunde genommen gut |")
);

// A heading must not shield a preamble underneath it.

rejects!(
    preamble_under_heading,
    rules::no_preamble(p(), "## Titel\n\nZunächst der Befund.")
);
accepts!(
    heading_then_claim,
    rules::no_preamble(p(), "## Titel\n\nDer Hook läuft.")
);

// A trailing code block must not hide a trailing question.

rejects!(
    question_before_code_block,
    rules::is_statement("Läuft der Hook?\n\n```bash\njust check\n```")
);
accepts!(
    claim_before_code_block,
    rules::is_statement("Der Hook läuft.\n\n```bash\njust check\n```")
);

// --- umlauts: ASCII transcriptions must not slip past --------------------
// Writing "zunaechst" instead of "zunächst" is common enough that a rule
// keyed on the umlaut alone would be trivial to sidestep by accident.

rejects!(
    preamble_ae_form,
    rules::no_preamble(p(), "Zunaechst der Befund.")
);
rejects!(
    preamble_umlaut,
    rules::no_preamble(p(), "Zunächst der Befund.")
);
rejects!(hedge_oe_form, rules::no_hedging(p(), "Moeglicherweise so."));
rejects!(hedge_umlaut, rules::no_hedging(p(), "Möglicherweise so."));
rejects!(
    meta_ae_form,
    rules::no_meta(p(), "Wie bereits erwaehnt gilt das.")
);
rejects!(
    hedge_ue_form,
    rules::no_hedging(p(), "Ich wuerde sagen ja.")
);

// --- ASCII umlauts: sampled through short function words -----------------
// The list covers function words only, not the vocabulary. One match is
// enough to reject the answer, since the spelling is a property of the
// whole text rather than of a single word.

rejects!(
    ascii_fuer,
    rules::no_substitute_spelling(p(), "Der Schlüssel ist fuer das Archiv.")
);
rejects!(
    ascii_ueber,
    rules::no_substitute_spelling(p(), "Das läuft ueber PAM.")
);
rejects!(
    ascii_waere,
    rules::no_substitute_spelling(p(), "Das waere korrekt.")
);
rejects!(
    ascii_koennte,
    rules::no_substitute_spelling(p(), "Das koennte stimmen.")
);
rejects!(
    ascii_muessen,
    rules::no_substitute_spelling(p(), "Beide muessen brechen.")
);
rejects!(
    ascii_zunaechst,
    rules::no_substitute_spelling(p(), "Er prüft das zunaechst.")
);
rejects!(
    ascii_moeglich,
    rules::no_substitute_spelling(p(), "Das ist moeglich.")
);
rejects!(
    ascii_natuerlich,
    rules::no_substitute_spelling(p(), "Das greift natuerlich.")
);
// `waere` is a prefix of `waeren`. The shorter phrase matched first, ended
// inside the word and hid the longer one.
rejects!(
    ascii_waeren_behind_its_prefix,
    rules::no_substitute_spelling(p(), "Das waeren drei.")
);
rejects!(
    ascii_haetten_behind_its_prefix,
    rules::no_substitute_spelling(p(), "Sie haetten es gesehen.")
);
rejects!(
    ascii_wuerden_behind_its_prefix,
    rules::no_substitute_spelling(p(), "Beide wuerden brechen.")
);

// An anchored category gets a single try at the start of the opening line,
// so the longer phrase must be tried before the shorter one fails there.
#[test]
fn an_anchored_phrase_behind_its_prefix_is_found() {
    let lists = config::PhraseLists {
        preamble: Some(vec!["vor".to_string(), "vorweg".to_string()]),
        ..Default::default()
    };
    let phrases = Phrases::compile(&[&lists]);
    assert!(rules::no_preamble(&phrases, "Vorweg: Es läuft.").is_err());
    assert!(rules::no_preamble(&phrases, "Vorhanden ist alles.").is_ok());
}

rejects!(
    ascii_capitalised,
    rules::no_substitute_spelling(p(), "Ueber den Hook läuft das.")
);

// Correct spelling passes, which is the point of the rule.

accepts!(
    umlaut_fuer_passes,
    rules::no_substitute_spelling(p(), "Der Schlüssel ist für das Archiv.")
);
accepts!(
    umlaut_moeglich_passes,
    rules::no_substitute_spelling(p(), "Das ist möglich.")
);
accepts!(
    umlaut_muessen_passes,
    rules::no_substitute_spelling(p(), "Beide müssen brechen.")
);

// The digraph is common in correct German. These must never fire, since a
// false positive costs a turn while a miss only delays detection.

accepts!(
    neue_passes,
    rules::no_substitute_spelling(p(), "Der neue Hook ist aktiv.")
);
accepts!(
    feuer_passes,
    rules::no_substitute_spelling(p(), "Die Feuerwand steht.")
);
accepts!(
    steuer_passes,
    rules::no_substitute_spelling(p(), "Die Steuerung greift.")
);
accepts!(
    aufwand_passes,
    rules::no_substitute_spelling(p(), "Der Aufwand ist gering.")
);
accepts!(
    queue_passes,
    rules::no_substitute_spelling(p(), "Die Queue ist leer.")
);
accepts!(
    koeffizient_passes,
    rules::no_substitute_spelling(p(), "Der Koeffizient stimmt.")
);
accepts!(
    poetisch_passes,
    rules::no_substitute_spelling(p(), "Das klingt poetisch.")
);
accepts!(
    treuer_passes,
    rules::no_substitute_spelling(p(), "Ein treuer Begleiter.")
);

// English answers are not German prose, so their vocabulary must pass.

accepts!(
    en_value_passes,
    rules::no_substitute_spelling(p(), "The value is set.")
);
accepts!(
    en_requeue_passes,
    rules::no_substitute_spelling(p(), "The job was requeued.")
);

// Proper names are spelled the way their bearer spells them. Keeping the
// list to function words is what makes this work: nobody is called Fuer or
// Waere, so no exemption by context is needed. Surnames that would collide
// with a longer list are covered here as a guard against reintroducing them.

accepts!(
    surname_fuerst,
    rules::no_substitute_spelling(p(), "Herr Fuerst hat den Schlüssel.")
);
accepts!(
    company_fuersa,
    rules::no_substitute_spelling(p(), "Die Fuersa GmbH liefert.")
);
accepts!(
    surname_faellt,
    rules::no_substitute_spelling(p(), "Herr Faellt kam an.")
);
accepts!(
    surname_groesse,
    rules::no_substitute_spelling(p(), "Frau Groesse meldete sich.")
);
accepts!(
    surname_hoehe,
    rules::no_substitute_spelling(p(), "Herr Hoehe zog um.")
);
accepts!(
    surname_baehr,
    rules::no_substitute_spelling(p(), "Herr Baehr kam an.")
);
accepts!(
    place_ueberlingen,
    rules::no_substitute_spelling(p(), "Die Stadt Ueberlingen liegt am See.")
);

// A name in the sentence does not license a violation next to it.

rejects!(
    violation_beside_a_surname,
    rules::no_substitute_spelling(p(), "Herr Fuerst braucht das fuer sein Archiv.")
);

// Identifiers keep their ASCII spelling. Code is not the writer's prose.

accepts!(
    ascii_form_in_code_span,
    rules::no_substitute_spelling(p(), "Setze `fuer = 1`.")
);
accepts!(
    ascii_form_in_fenced_block,
    rules::no_substitute_spelling(p(), "So:\n\n```sh\nfuer_test --run\n```")
);

// --- dashes: clause separators are rejected, ranges are not --------------
// The house style bans em- and en-dashes as clause joiners. Ranges bind
// tight and stay allowed, which is why the rule keys on the surrounding
// whitespace instead of the character alone. No exclude list needed.

rejects!(
    em_dash_spaced,
    rules::no_clause_dashes("Der Hook läuft \u{2014} er ist registriert.")
);
rejects!(
    em_dash_tight,
    rules::no_clause_dashes("Der Hook läuft\u{2014}er ist registriert.")
);
rejects!(
    en_dash_as_clause_joiner,
    rules::no_clause_dashes("Der Hook läuft \u{2013} er ist registriert.")
);
rejects!(
    en_dash_leading_space,
    rules::no_clause_dashes("Zwei Dinge \u{2013}ein Befund.")
);
rejects!(
    en_dash_trailing_space,
    rules::no_clause_dashes("Zwei Dinge\u{2013} ein Befund.")
);

accepts!(
    numeric_range_allowed,
    rules::no_clause_dashes("Die Zeilen 10\u{2013}20 fehlen.")
);
accepts!(
    year_range_allowed,
    rules::no_clause_dashes("Der Zeitraum 2020\u{2013}2024 ist erfasst.")
);
accepts!(
    relation_allowed,
    rules::no_clause_dashes("Die Client\u{2013}Server-Verbindung steht.")
);
accepts!(
    hyphen_untouched,
    rules::no_clause_dashes("Der Stop-Hook ist registriert.")
);
accepts!(
    plain_prose_passes,
    rules::no_clause_dashes("Der Hook läuft, er ist registriert.")
);

// Quoted foreign text is not the writer's prose, so it is exempt already.

accepts!(
    em_dash_in_code_span,
    rules::no_clause_dashes("Der Wert `a \u{2014} b` steht so da.")
);
accepts!(
    em_dash_in_fenced_block,
    rules::no_clause_dashes("So:\n\n```\nfoo \u{2014} bar\n```")
);

// A Markdown rule is layout, not punctuation.
accepts!(
    horizontal_rule_allowed,
    rules::no_clause_dashes("Oben.\n\n---\n\nUnten.")
);

// --- English must be judged as strictly as German ------------------------
// The answer language varies; the rules must not be a German-only filter.

rejects!(
    en_preamble_let_me_start,
    rules::no_preamble(p(), "Let me start with the finding.")
);
rejects!(
    en_preamble_to_start,
    rules::no_preamble(p(), "To start, the hook is registered.")
);
rejects!(
    en_summary_announce,
    rules::no_preamble(p(), "TL;DR: it works.")
);
rejects!(
    en_hedge_seems,
    rules::no_hedging(p(), "It seems the hook fires.")
);
rejects!(
    en_hedge_arguably,
    rules::no_hedging(p(), "This is arguably correct.")
);
rejects!(
    en_forward_ref,
    rules::self_contained(p(), "The hook works, see below.")
);
rejects!(en_apology, rules::no_meta(p(), "Sorry for the confusion."));
rejects!(
    en_filler_at_the_end_of_the_day,
    rules::no_filler(p(), "At the end of the day it runs.")
);
accepts!(
    en_plain_claim_passes,
    rules::no_preamble(p(), "The hook is registered.")
);
accepts!(
    en_factual_sentence_passes,
    rules::no_hedging(p(), "Three tests failed.")
);

// --- placeholders: the unfilled template must not pass -------------------
// The prompt hook ships "..." in every field. Copying it satisfies every
// other rule, so without this the schema waves the template through.

rejects!(bare_ellipsis, rules::filled_in("..."));
rejects!(unicode_ellipsis, rules::filled_in("\u{2026}"));
rejects!(padded_ellipsis, rules::filled_in("  ...  "));
rejects!(dashes_only, rules::filled_in("---"));
rejects!(empty_string, rules::filled_in(""));
accepts!(real_content, rules::filled_in("Der Hook rendert jetzt."));
// An ellipsis inside a real sentence is punctuation, not a placeholder.
accepts!(ellipsis_in_prose, rules::filled_in("Der Befund ... steht."));

// --- prose word count: markup does not consume the budget ----------------

#[test]
fn table_counts_cells_only() {
    assert_eq!(prose_word_count("| a | b |\n|---|---|\n| 1 | 2 |"), 4);
}

#[test]
fn list_markers_not_counted() {
    assert_eq!(prose_word_count("- eins\n- zwei\n- drei"), 3);
}

#[test]
fn code_span_not_counted() {
    assert_eq!(prose_word_count("Wert `x = 1` gesetzt"), 2);
}

#[test]
fn raw_count_keeps_code() {
    assert!(word_count("```\njust check\n```") > 0);
}

// --- word budgets --------------------------------------------------------
// Not in test.ncl, which probed them through the schema. They are rules in
// v2, so they get direct cases.

#[test]
fn budget_names_the_count() {
    let text = "Wort ".repeat(11);
    let err = rules::max_words(10, &text).expect_err("over budget");
    assert_eq!(err.message, "11 words, limit is 10");
    assert_eq!(err.notes, vec!["Cut 1 of them.".to_string()]);
}

#[test]
fn budget_allows_the_limit_exactly() {
    let text = "Wort ".repeat(10);
    assert!(rules::max_words(10, &text).is_ok());
}

#[test]
fn sentence_length_names_the_count() {
    let text = format!("{}Ende.", "Wort ".repeat(42));
    let err = rules::sentence_length(40, &text).expect_err("too long");
    assert_eq!(err.message, "a sentence runs 43 words, limit is 40");
    assert_eq!(
        err.notes,
        vec!["Split it. Long sentences bury the point.".to_string()]
    );
}

#[test]
fn non_empty_rejects_whitespace() {
    assert!(rules::non_empty("   \n  ").is_err());
    assert!(rules::non_empty("x").is_ok());
}

// A fenced block that mentions a single backtick, which a shell comment or a
// Markdown example easily does. The fence pattern used to exclude the backtick
// character, so such a block never matched and its wording reached the prose
// rules and the word budget as if the writer had said it.
#[test]
fn a_fence_containing_a_backtick_is_still_stripped() {
    let block = "```sh\necho \"it's a ` char\" # vermutlich falsch\n```";
    assert!(rules::no_hedging(p(), block).is_ok());
    assert_eq!(prose_word_count(block), 0);
}

#[test]
fn prose_between_two_fences_survives_stripping() {
    let text = "```\naaa\n```\nreal prose here\n```\nbbb\n```";
    assert_eq!(prose_word_count(text), 3);
}

// A four-backtick fence wrapping a three-backtick one, which is how a Markdown
// example of a code block is written. Fences have to be paired by length: the
// lazy pattern paired the opener with the inner fence, left the rest of the
// block as prose, and then ate the prose after it inside a spurious second
// block.
#[test]
fn a_longer_fence_wraps_a_shorter_one() {
    let text = "````md\n```nickel\nvermutlich falsch\n```\n````\nreal prose here";
    assert_eq!(prose_word_count(text), 3);
    assert!(rules::no_hedging(p(), text).is_ok());
}

// The closing fence may be longer than the opening one, but not shorter.
#[test]
fn a_shorter_fence_does_not_close_a_longer_one() {
    let text = "````\naaa\n```\nbbb\n````\nreal prose here";
    assert_eq!(prose_word_count(text), 3);
}

// A fence that is never closed runs to the end, so an interrupted message does
// not leak its half-written block into the prose rules.
#[test]
fn an_unclosed_fence_swallows_the_rest() {
    let text = "real prose here\n```nickel\nvermutlich falsch";
    assert_eq!(prose_word_count(text), 3);
    assert!(rules::no_hedging(p(), text).is_ok());
}

// An info string marks an opener, so it cannot close the block it sits in.
#[test]
fn an_info_string_line_does_not_close_a_block() {
    let text = "```\naaa\n```sh\nbbb\n```\nreal prose here";
    assert_eq!(prose_word_count(text), 3);
}

// --- regressions from the pre-release review -----------------------------

// `s.u.` ends in a dot, so a trailing `\b` never matched it before a space or
// the end of the text, and the forward reference passed.
rejects!(
    forward_reference_abbreviated,
    rules::self_contained(p(), "Der Hook läuft, Details s.u.")
);
rejects!(
    forward_reference_abbreviated_mid_sentence,
    rules::self_contained(p(), "Details s.u. im Abschnitt Belege.")
);
accepts!(
    abbreviation_inside_word,
    rules::self_contained(p(), "Die Datei heißt plans.u.md.")
);

// `first` and `to start` open direct answers too. Only a lead-in followed by
// punctuation is a preamble.
accepts!(
    to_start_as_instruction,
    rules::no_preamble(p(), "To start the server, run `just install`.")
);
accepts!(
    first_in_a_compound,
    rules::no_preamble(p(), "First-party crates are pinned.")
);
rejects!(
    to_start_with_as_lead_in,
    rules::no_preamble(p(), "To start with, the hook is fine.")
);
rejects!(
    first_of_all_as_lead_in,
    rules::no_preamble(p(), "First of all: it runs.")
);

// A long sentence wrapped over several lines is still one sentence.
#[test]
fn a_wrapped_sentence_counts_as_one() {
    let line = "wort ".repeat(18);
    let text = format!("{line}\n{line}\n{line}\n{line} Ende.");
    assert!(rules::sentence_length(40, &text).is_err());
}

// Line breaks that start a new block still end a sentence, so list items and
// paragraphs without a full stop are counted on their own.
#[test]
fn list_items_and_paragraphs_count_separately() {
    let item = "wort ".repeat(30);
    let list = format!("Befunde:\n- {item}\n- {item}\n* {item}\n1. {item}");
    assert!(rules::sentence_length(40, &list).is_ok());
    let paragraphs = format!("{item}\n\n{item}");
    assert!(rules::sentence_length(40, &paragraphs).is_ok());
}

// A run of backticks with another backtick after it is inline code, not an
// opener. Taken as one, it hid the rest of the field from every prose rule.
#[test]
fn inline_triple_backticks_do_not_open_a_fence() {
    let text = "```x``` ist das Makro.\nBasically it runs.";
    assert!(rules::no_filler(p(), text).is_err());
    assert_eq!(prose_word_count(text), 6);
}

// Tilde fences are code blocks as well, and a backtick fence does not close one.
#[test]
fn a_tilde_fence_is_stripped() {
    let text = "~~~\nvermutlich falsch\n```\nimmer noch Code\n~~~\nreal prose here";
    assert!(rules::no_hedging(p(), text).is_ok());
    assert_eq!(prose_word_count(text), 3);
}

// The rule text forbids filler and apologies anywhere, so entries are held to
// it too. Caveats keep their licence to hedge.
rejects!(
    apology_in_caveat,
    rules::caveat_rules(p(), "Sorry for the gap: not tested.")
);
rejects!(
    filler_in_caveat,
    rules::caveat_rules(p(), "Basically not tested.")
);
rejects!(
    filler_in_evidence,
    rules::evidence_rules(p(), "Im Grunde genommen lief der Test.")
);
accepts!(
    hedge_in_caveat,
    rules::caveat_rules(p(), "Der Pfad ist vermutlich ungetestet.")
);

// Sentence terminators of scripts that put no space after them.
#[test]
fn cjk_terminators_end_sentences() {
    let text = "这是第一句。这是第二句。";
    assert_eq!(
        n4t::prose::sentence_word_counts(text)
            .iter()
            .filter(|n| **n > 0)
            .count(),
        2
    );
}

// --- word count in scripts without spaces --------------------------------

// Chinese, Japanese, Thai, Lao, Khmer and Burmese put no spaces between words.
// Split on whitespace, a whole paragraph would be one word and no budget would
// apply. ICU4X finds the word boundaries with dictionaries and models.
#[test]
fn chinese_counts_words_not_characters() {
    // 这是 一个 测试
    assert_eq!(word_count("这是一个测试"), 4);
}

#[test]
fn japanese_counts_words_not_characters() {
    // テスト です, 日本語 の 文
    assert_eq!(word_count("テストです"), 2);
    assert_eq!(word_count("日本語の文"), 3);
}

#[test]
fn thai_counts_words() {
    // ภาษา ไทย เป็น ภาษา ที่ ไม่ มี การ เว้นวรรค ระหว่าง คำ
    assert_eq!(word_count("ภาษาไทยเป็นภาษาที่ไม่มีการเว้นวรรคระหว่างคำ"), 11);
}

#[test]
fn mixed_text_counts_both_scripts() {
    assert_eq!(word_count("Rust の関数 is fast"), 5);
}

// Punctuation does not count, numbers do, and a hyphen joins two words.
#[test]
fn spaced_languages_count_words() {
    assert_eq!(word_count("Client-Server läuft, z.B. hier."), 5);
    assert_eq!(word_count("Version 3.5 läuft seit 2026."), 5);
    assert_eq!(word_count("don't stop"), 2);
}

#[test]
fn a_long_chinese_answer_exceeds_the_budget() {
    let text = "这是一个很长的句子".repeat(20);
    assert!(rules::max_words(100, &text).is_err());
}

// Thai separates sentences with a space, not a full stop. A paragraph of short
// sentences must not count as one long one.
#[test]
fn a_space_ends_a_thai_sentence() {
    let paragraph = ["ภาษาไทยเป็นภาษาที่ไม่มีการเว้นวรรคระหว่างคำ"; 15].join(" ");
    assert!(rules::sentence_length(40, &paragraph).is_ok());
    let run_on = ["ภาษาไทยเป็นภาษาที่ไม่มีการเว้นวรรคระหว่างคำ"; 5].concat();
    assert!(rules::sentence_length(40, &run_on).is_err());
}

// Each space between Thai text ends a sentence, also around a sentence of a
// single character. The matches of one pattern do not overlap, so the space
// after such a sentence used to stay.
#[test]
fn a_one_character_thai_sentence_is_split_on_both_sides() {
    let counts = n4t::prose::sentence_word_counts("ภาษาไทย ณ ภาษาไทย");
    assert_eq!(counts.iter().filter(|n| **n > 0).count(), 3, "{counts:?}");
}

// Burmese and Khmer end sentences with their own marks.
#[test]
fn burmese_and_khmer_marks_end_sentences() {
    let burmese = "မြန်မာဘာသာစကားသည်စကားလုံးများကြားတွင်။".repeat(10);
    assert!(rules::sentence_length(40, &burmese).is_ok());
    let khmer = "ភាសាខ្មែរមិនមានដកឃ្លារវាងពាក្យទេ។".repeat(10);
    assert!(rules::sentence_length(40, &khmer).is_ok());
}

// Hindi words often end in a combining vowel sign (के, चलता, है). ICU4X types
// such a segment as no word, so the count goes by its letters instead.
#[test]
fn hindi_counts_every_word() {
    assert_eq!(word_count("हुक हर उत्तर के बाद चलता है।"), 7);
    assert_eq!(word_count("यह रिकॉर्ड की संरचना जाँचता है।"), 6);
}

// A decomposed accent is a combining mark at the end of the word as well.
#[test]
fn a_decomposed_accent_keeps_the_word() {
    assert_eq!(word_count("cafe\u{301} ouvert"), 2);
}

// --- phrases in scripts without spaces -----------------------------------

/// A pack's hedging list for one language, compiled.
fn hedging(words: &[&str]) -> Phrases {
    let lists = config::PhraseLists {
        hedging: Some(words.iter().map(|w| w.to_string()).collect()),
        ..Default::default()
    };
    Phrases::compile(&[&lists])
}

// Chinese has no spaces, so a phrase is a whole word only where the segmenter
// says so. `可能` (maybe) is a hedge, `可能性` (possibility) is a noun.
#[test]
fn a_chinese_hedge_is_found() {
    let p = hedging(&["可能"]);
    assert!(rules::no_hedging(&p, "这个钩子可能有问题。").is_err());
    assert!(rules::no_hedging(&p, "这种可能性很低。").is_ok());
}

#[test]
fn a_japanese_hedge_is_found() {
    let p = hedging(&["たぶん"]);
    assert!(rules::no_hedging(&p, "たぶん動きます。").is_err());
    assert!(rules::no_hedging(&p, "フックが、たぶん動きます。").is_err());
    assert!(rules::no_hedging(&p, "フックは動きます。").is_ok());
}

// A known limit of the dictionary: after the particle は, ICU4X 2.4 splits
// はたぶん as はた (flag) and ぶん, so the hedge is missed. A missed phrase lets
// the answer through, which is the cheap way to be wrong. This test records
// the behaviour so a segmenter update that fixes it shows up.
#[test]
fn a_japanese_hedge_after_wa_is_missed() {
    let p = hedging(&["たぶん"]);
    assert!(rules::no_hedging(&p, "フックはたぶん動きます。").is_ok());
}

// A known limit of the word boundaries: UAX #29 keeps a letter, a colon,
// a full stop or an apostrophe and a letter together as one word. A phrase
// that touches the next word through one of them, without a space, is missed.
// The regex boundary used before caught these, but such text is a typo or a
// contraction, and missing it lets the answer through.
#[test]
fn a_phrase_glued_to_the_next_word_is_missed() {
    let p = hedging(&["probably"]);
    assert!(rules::no_hedging(&p, "It probably works.").is_err());
    assert!(rules::no_hedging(&p, "It probably.Works").is_ok());
    assert!(rules::no_hedging(&p, "the probably's here").is_ok());
}

// ภาษาไทยเป็นภาษา... : "อาจจะ" (maybe) inside an unspaced Thai sentence.
#[test]
fn a_thai_hedge_is_found() {
    let p = hedging(&["อาจจะ"]);
    assert!(rules::no_hedging(&p, "ฮุกอาจจะทำงานหลังทุกคำตอบ").is_err());
    assert!(rules::no_hedging(&p, "ฮุกทำงานหลังทุกคำตอบ").is_ok());
}

// The violation names the phrase as the writer wrote it.
#[test]
fn the_violation_names_the_phrase() {
    let p = hedging(&["可能"]);
    let v = rules::no_hedging(&p, "这个钩子可能有问题。").unwrap_err();
    assert!(v.message.contains("\"可能\""), "{}", v.message);
}
