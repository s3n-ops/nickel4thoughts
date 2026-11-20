# n4t configuration skeleton
#
# `just install` overwrites this file on every run. To change a limit, copy it
# to `config.ncl` in the same directory and edit the copy:
#
#   cp ~/.config/n4t/config.skel.ncl ~/.config/n4t/config.ncl
#
# `config.ncl` is never touched by an install. Without it, the defaults below
# apply. Run `n4t config` to see the limits in effect.
#
# Every key is optional. A key left out keeps its default, so a config can name
# only what it changes. All limits count words separated by whitespace and must
# be at least 1. A misspelt key is reported, not ignored.
#
# Examples:
#
#   # Longer bottom lines, everything else unchanged
#   {
#     budgets = {
#       bottom_line = 150,
#     },
#   }
#
#   # Tighter answers overall
#   {
#     budgets = {
#       bottom_line = 60,
#       supporting_detail = 120,
#     },
#     sentence_length = 25,
#   }
#
#   # German as the language of answers that do not set one
#   {
#     default_language = 'de,
#   }
#
# Headings and phrase lists belong to the language packs, one file per
# language in `languages/` next to this file, such as `languages/fr.ncl`. n4t
# ships packs for English and German. A pack of the same name replaces the
# shipped headings and lists it names and keeps the rest. Run `n4t config` to
# see the languages in effect.
#
# If the file is invalid, the hooks fall back to the defaults and the injected
# rule says why. `n4t config` shows the error as well.
{
  # Word budget per field. `evidence` and `caveats` apply to each entry.
  budgets = {
    bottom_line = 100,
    supporting_detail = 200,
    evidence = 40,
    caveats = 40,
    next_action = 50,
  },

  # Maximum number of words in one sentence, in every prose field.
  sentence_length = 40,

  # Language of an answer that does not set `language`. It picks the pack
  # for the headings and the phrase rules.
  default_language = 'en,
}
