# German language pack. See en.ncl for the format.
#
# German prose is written with its umlauts. The ASCII spellings (`ae`, `oe`,
# `ue`) are derived from the words below instead of being listed twice, so a
# lead-in like "zunaechst" is caught as well as "zunächst".
let ascii = fun word =>
  word
  |> std.string.replace "ä" "ae"
  |> std.string.replace "ö" "oe"
  |> std.string.replace "ü" "ue"
in
# Each word with its ASCII spelling, where that differs.
let both = fun words =>
  words
  |> std.array.flat_map (fun w => if ascii w == w then [w] else [w, ascii w])
in
{
  labels = {
    bottom_line = "Fazit",
    supporting_detail = "Details",
    evidence = "Belege",
    caveats = "Einschränkungen",
    next_action = "Nächster Schritt",
    confidence = "Verlässlichkeit",
    verified = "ausgeführt und beobachtet",
    high = "hoch",
    medium = "mittel",
    low = "gering",
  },

  phrases = {
    preamble = both ["zunächst", "zuerst", "vorab", "einleitend"],
    question_comment = ["gute frage", "interessante frage"],
    summary = ["kurz gesagt", "zusammengefasst"],
    hedging = both [
      "ich denke",
      "ich glaube",
      "ich würde sagen",
      "vermutlich",
      "wahrscheinlich",
      "eventuell",
      "möglicherweise",
    ],
    forward_reference = ["siehe unten", "siehe details", "wie unten", "s.u."],
    meta = both [
      "wie bereits erwähnt",
      "wie gesagt",
      "in dieser antwort",
      "ich habe oben",
      "ich habe bereits",
    ],
    apology = ["entschuldigung", "es tut mir leid"],
    filler = both [
      "im grunde genommen",
      "letztendlich",
      "im endeffekt",
      "grundsätzlich gesagt",
    ],
    # Words that German prose of any length is unlikely to avoid, written
    # with ASCII digraphs. Dropping umlauts is a property of the whole text,
    # so a sample of short function words detects it. Words whose ASCII form
    # is also a plausible surname (Groesse, Faellt, Prueft) stay out. `ss`
    # for `ß` is valid Swiss orthography and cannot be judged mechanically.
    spelling = std.array.map ascii [
      "für",
      "über",
      "wäre",
      "wären",
      "hätte",
      "hätten",
      "können",
      "könnte",
      "müssen",
      "müsste",
      "würde",
      "würden",
      "zunächst",
      "später",
      "möglich",
      "nötig",
      "natürlich",
      "ungefähr",
    ],
  },
}
