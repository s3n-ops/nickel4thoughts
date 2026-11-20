# Entry point for an answer record.
#
# In v1 this file produced Markdown, because `nickel export --format text` was
# the only way to get a string out. In v2 the rendering lives in Rust, so
# `render` applies the structural contract and hands the record back as data.
# The binary exports it as JSON and takes it from there.
#
# The name stays `render` and the import shape stays the same, so a record
# written for v1 evaluates unchanged here.

let { Answer, .. } = import "answer.ncl" in

{
  render = fun answer => answer | Answer,
}
