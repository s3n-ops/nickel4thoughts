# English language pack.
#
# A pack describes what n4t knows about one language: the headings of the
# rendered answer, phrase lists for the prose rules, and optional checks. The
# file name is the language tag an answer sets in `language`.
#
# English headings are also the built-in fallback for every language, so this
# pack only lists phrases.
#
# Phrases match whole words, ignoring case. Write them as plain text, not as
# regular expressions. A phrase must contain a letter or a digit.
{
  phrases = {
    # Lead-ins at the start of the bottom line. `first` and `to start` only
    # count with punctuation after them, since bare they open direct answers
    # ("To start the server, run ...").
    preamble = [
      "first,",
      "first:",
      "first of all",
      "to start,",
      "to start:",
      "to start with,",
      "to start with:",
      "before we",
      "let me start",
      "let's start",
    ],
    # Comments on the question at the start of the bottom line.
    question_comment = ["great question", "good question"],
    # Announced summaries at the start of the bottom line.
    summary = ["in short", "in summary", "tl;dr", "tldr"],
    hedging = [
      "i think",
      "i believe",
      "probably",
      "perhaps",
      "maybe",
      "it seems",
      "arguably",
    ],
    forward_reference = ["see below", "as detailed below", "more on this below"],
    meta = ["as mentioned", "as i said", "in this answer", "to summarize what"],
    apology = ["sorry for", "my apologies"],
    filler = [
      "basically",
      "essentially",
      "at the end of the day",
      "it is important to note that",
      "it should be noted that",
    ],
  },
}
