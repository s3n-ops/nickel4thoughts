# Structure of a BLUF-structured AI answer.
#
# This schema checks shape only: field names, types, enum tags, which fields
# are required. The prose rules and the word budgets live in Rust
# (`src/rules.rs`), where the regexes compile once instead of once per field.
#
# The split is deliberate. Nickel stays because the record is real Nickel
# syntax (`m%"..."%`, enum tags, `import`), and writing a parser for that would
# be a second project. Everything past parsing is cheaper in Rust: v1 spent
# 22 ms of its 32 ms evaluating `replace_regex` contracts.

{
  Answer = {
    # The single most important statement. Comes first, always present.
    # Answers the question on its own, without the fields below.
    bottom_line
      | String
      | doc "The core answer. Must stand alone, assert, and not hedge.",

    # Reasoning, evidence, mechanism. Only what supports the bottom line.
    supporting_detail
      | String
      | optional
      | doc "Why the bottom line holds. Omit when it adds nothing.",

    # What was actually run or observed, as opposed to inferred.
    evidence
      | Array String
      | doc "Commands run, outputs seen, files read. Facts, not claims."
      | default
      = [],

    # Known limits of the answer.
    caveats
      | Array String
      | doc "What was not checked, what could still be wrong."
      | default
      = [],

    # Concrete next step for the reader, if one exists.
    next_action
      | String
      | optional
      | doc "What the reader should do or decide next.",

    # Confidence in the bottom line. Forces an explicit stance.
    confidence
      | [| 'verified, 'high, 'medium, 'low |]
      | doc "'verified means it was executed and observed, not reasoned.",

    # Language of the rendered headings. Any tag is accepted: which languages
    # have headings depends on the user's config, which this schema does not
    # see. Without the field, the configured default applies. The prose rules
    # do not depend on it.
    language
      | std.enum.Tag
      | optional
      | doc "Language of the headings. Match the language of the answer.",
  },
}
