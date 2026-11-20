# Build, test and register the answer-format hooks.
#
# Three hooks, one binary:
#   UserPromptSubmit  n4t inject-rule   injects the format rule
#   MessageDisplay    n4t render        replaces the record on screen
#   Stop              n4t validate      validates and blocks on violation

settings := env_var('HOME') / '.claude' / 'settings.json'
# The build artefact, used by `check` and `bench` so they exercise what was just
# compiled.
# Cargo decides where that is (see .cargo/config.toml), so it is asked rather
# than assumed to be `target/` in the repository.
target_dir := `cargo metadata --format-version 1 --no-deps | jq -r .target_directory`
binary := target_dir / 'release' / 'n4t'
# What the hooks actually run. It lives outside the repository, so `just clean`
# and `cargo clean` leave the registered hooks intact.
installed := env_var('HOME') / '.local' / 'bin' / 'n4t'
schema_dir := justfile_directory() / 'schema'
# The rule text points every answer at this copy. `schema_dir` sits in the
# repository, which the injected rule must not depend on, so the schema is
# installed where `schema_dir()` in src/hooks/inject.rs looks for it, one
# directory up from bin and under share/n4t.
installed_schema := env_var('HOME') / '.local' / 'share' / 'n4t' / 'schema'
# The config directory, resolved the way `config_dir()` in src/config.rs does
# without N4T_CONFIG_DIR: XDG_CONFIG_HOME only when it is an absolute path, as
# the XDG specification requires. Install writes only the skeleton there;
# `config.ncl` belongs to the user.
xdg_config := env_var_or_default('XDG_CONFIG_HOME', '')
config_dir := if xdg_config =~ '^/' { xdg_config / 'n4t' } else { env_var('HOME') / '.config' / 'n4t' }
skeleton := justfile_directory() / 'config' / 'config.skel.ncl'

_default:
    @just --list

# Build the release binary.
build:
    cargo build --release

# Run the whole suite: rules, hooks, rendering, plus the schema itself.
check: build
    #!/usr/bin/env bash
    set -euo pipefail
    cargo test
    cargo clippy --all-targets -- -D warnings
    cargo fmt --check
    # The schema is Nickel, so it is checked by Nickel. A record that the
    # binary accepts must also evaluate on its own.
    probe=$(mktemp -t n4t-check.XXXXXX.ncl) && [ -n "$probe" ] || exit 1
    # A config of its own, so the user's files cannot change the outcome. A
    # German pack of its own overrides one shipped heading, which exercises
    # user packs end to end.
    check_config=$(mktemp -d -t n4t-check-config.XXXXXX) && [ -n "$check_config" ] || exit 1
    trap 'rm -f -- "$probe"; rm -rf -- "$check_config"' EXIT
    mkdir "$check_config/languages"
    printf '{ labels = { bottom_line = "Kurzfazit" } }\n' > "$check_config/languages/de.ncl"
    export N4T_CONFIG_DIR="$check_config"
    for lang in de en; do
      printf 'let { render, .. } = import "%s/render.ncl" in\nrender { bottom_line = "ok", confidence = %s, language = %s }\n' \
        "{{ schema_dir }}" "'high" "'$lang" > "$probe"
      "{{ binary }}" check "$probe" > /dev/null
    done
    echo "Schema OK."
    # End to end through the binary, the way Claude Code calls it.
    for lang in de:Kurzfazit en:"Bottom line"; do
      printf -v record 'let { render, .. } = import "%s/render.ncl" in\nrender { bottom_line = "ok", confidence = %s, language = %s }' \
        "{{ schema_dir }}" "'high" "'${lang%%:*}"
      shown=$(jq -nc --arg d '```nickel'$'\n'"$record"$'\n''```'$'\n' \
        '{hook_event_name:"MessageDisplay",message_id:"check",index:0,final:true,delta:$d}' \
        | "{{ binary }}" render | jq -r '.hookSpecificOutput.displayContent')
      grep -qF "## ${lang#*:}" <<<"$shown" \
        || { echo "Display hook did not render ${lang%%:*}:" >&2; echo "$shown" >&2; exit 1; }
    done
    # The rule text must carry the schema path and its backtick-quoted names.
    out=$("{{ binary }}" inject-rule </dev/null)
    grep -qF '`bottom_line`' <<<"$out" \
      || { echo "Rule text lost its backtick-quoted field names." >&2; exit 1; }
    grep -qF "{{ schema_dir }}/render.ncl" <<<"$out" \
      || { echo "Rule text does not point at the schema." >&2; exit 1; }
    echo "Hooks OK."
    # The skeleton must load as a valid config.
    skel_dir=$(mktemp -d -t n4t-config.XXXXXX) && [ -n "$skel_dir" ] || exit 1
    trap 'rm -f -- "$probe"; rm -rf -- "$check_config" "$skel_dir"' EXIT
    cp "{{ skeleton }}" "$skel_dir/config.ncl"
    N4T_CONFIG_DIR="$skel_dir" "{{ binary }}" config > /dev/null \
      || { echo "The config skeleton does not load." >&2; exit 1; }
    echo "Config OK."

# Install under ~/.local and register all three hooks, replacing only our own.
install: build
    #!/usr/bin/env nu
    let settings = "{{ settings }}"
    let binary = "{{ binary }}"
    let installed = "{{ installed }}"

    if not ($binary | path exists) {
      print --stderr $"($binary) not built."
      exit 1
    }

    # Collected before anything is copied. An empty glob would otherwise reach
    # `cp` as a bare destination and abort the recipe after the installed schema
    # was already deleted, leaving the hooks pointing at a resolvable binary
    # whose `import` path is gone.
    let sources = (glob "{{ schema_dir }}/*")
    if ($sources | is-empty) {
      print --stderr $"No schema files in {{ schema_dir }}."
      exit 1
    }

    # The hooks run the copy, never the build artefact, so `just clean` cannot
    # pull the binary out from under a registered hook.
    mkdir ($installed | path dirname)
    cp --force $binary $installed
    print $"Installed ($installed)."

    # The schema goes along, otherwise the rule text points at a path that the
    # `import` cannot resolve. Removed first, so a deleted file does not survive
    # in the installed copy.
    let installed_schema = "{{ installed_schema }}"
    rm --force --recursive $installed_schema
    mkdir $installed_schema
    cp --force --recursive ...$sources $installed_schema
    print $"Installed ($installed_schema)."

    # The skeleton is ours and replaced on every run. The active config.ncl is
    # the user's and never written here.
    let config_dir = "{{ config_dir }}"
    mkdir $config_dir
    cp --force "{{ skeleton }}" ($config_dir | path join "config.skel.ncl")
    print $"Installed ($config_dir | path join 'config.skel.ncl')."

    mkdir ($settings | path dirname)
    let current = if ($settings | path exists) { open --raw $settings | from json } else { {} }

    # Ours are the entries running either path. Matching the build artefact too
    # clears out registrations from before the binary moved to ~/.local/bin.
    let ours = [$installed, $binary]

    # Drops our own entries for one event, then appends the current one. Foreign
    # entries keep their position, so a rerun is idempotent.
    def register [event: string, entry: record, ours: list<string>]: record -> record {
      let config = $in
      let hooks = ($config.hooks? | default {})
      let kept = ($hooks | get -o $event | default [] | where {|group|
        ($group.hooks? | default [] | where {|hook| ($hook.command? | default "") in $ours } | is-empty)
      })
      $config | upsert hooks ($hooks | upsert $event ($kept | append {hooks: [$entry]}))
    }

    # With `args` present Claude Code starts the binary without a shell, which
    # saves a process and rules out quoting errors. `${CLAUDE_PROJECT_DIR}` is
    # not expanded in that form, so the path is absolute.
    let updated = ($current
      | register "UserPromptSubmit" {type: "command", command: $installed, args: ["inject-rule"]} $ours
      | register "Stop" {type: "command", command: $installed, args: ["validate"]} $ours
      # Claude Code holds the screen until this hook returns, so it gets a
      # short leash. The default for the event is 10 seconds.
      | register "MessageDisplay" {type: "command", command: $installed, args: ["render"], timeout: 5} $ours)

    # Written through a temporary file in the same directory, so an interrupted
    # run cannot leave the settings truncated.
    let tmp = $"($settings).($nu.pid).tmp"
    $updated | to json --indent 2 | save --force $tmp
    mv --force $tmp $settings
    print "Hooks registered."

# Remove the installed files and our own entries, leave foreign ones untouched.
uninstall:
    #!/usr/bin/env nu
    let settings = "{{ settings }}"
    let binary = "{{ binary }}"
    let installed = "{{ installed }}"

    # Both paths count as ours, so an uninstall also clears registrations from
    # before the binary moved to ~/.local/bin.
    let ours = [$installed, $binary]

    # Removed before the settings are touched, because the early exit below
    # would otherwise leave the copies behind.
    if ($installed | path exists) {
      rm --force $installed
      print $"Removed ($installed)."
    }

    # The parent directory goes too, but only when nothing else moved in.
    let installed_schema = "{{ installed_schema }}"
    if ($installed_schema | path exists) {
      rm --force --recursive $installed_schema
      let share = ($installed_schema | path dirname)
      if (ls --all $share | is-empty) { rm --force --recursive $share }
      print $"Removed ($installed_schema)."
    }

    # Only the skeleton goes. A config.ncl the user wrote stays, and with it
    # the directory.
    let config_dir = "{{ config_dir }}"
    let skeleton = ($config_dir | path join "config.skel.ncl")
    if ($skeleton | path exists) {
      rm --force $skeleton
      print $"Removed ($skeleton)."
    }
    if ($config_dir | path exists) and (ls --all $config_dir | is-empty) {
      rm --force --recursive $config_dir
    }

    if not ($settings | path exists) {
      print $"Nothing to remove: ($settings) does not exist."
      exit 0
    }

    let current = (open --raw $settings | from json)

    # Removes our entries for one event and drops the event once it is empty,
    # so an uninstall leaves no hollow keys behind.
    def unregister [event: string, ours: list<string>]: record -> record {
      let config = $in
      let hooks = ($config.hooks? | default {})
      if ($hooks | get -o $event | is-empty) { return $config }
      let kept = ($hooks | get $event | where {|group|
        ($group.hooks? | default [] | where {|hook| ($hook.command? | default "") in $ours } | is-empty)
      })
      let hooks = if ($kept | is-empty) { $hooks | reject $event } else { $hooks | upsert $event $kept }
      $config | upsert hooks $hooks
    }

    let updated = ($current
      | unregister "UserPromptSubmit" $ours
      | unregister "Stop" $ours
      | unregister "MessageDisplay" $ours)
    let updated = if ($updated.hooks? | default {} | is-empty) { $updated | reject -o hooks } else { $updated }

    let tmp = $"($settings).($nu.pid).tmp"
    $updated | to json --indent 2 | save --force $tmp
    mv --force $tmp $settings
    print "Hooks removed."

# Measured with the skeleton as config file, so the number includes loading it
# and does not depend on the user's own config.
# Measure the render hook. Target: under 20 ms.
bench: build
    #!/usr/bin/env bash
    set -euo pipefail
    config_dir=$(mktemp -d -t n4t-bench.XXXXXX) && [ -n "$config_dir" ] || exit 1
    trap 'rm -rf -- "$config_dir"' EXIT
    cp "{{ skeleton }}" "$config_dir/config.ncl"
    export N4T_CONFIG_DIR="$config_dir"
    printf -v record 'let { render, .. } = import "%s/render.ncl" in\nrender {\n  bottom_line = "Der Stop-Hook nennt die verletzte Regel.",\n  supporting_detail = "Die Diagnose kommt als Struktur, nicht als Text.",\n  confidence = %s,\n}' \
      "{{ schema_dir }}" "'verified"
    payload=$(jq -nc --arg d '```nickel'$'\n'"$record"$'\n''```'$'\n' \
      '{hook_event_name:"MessageDisplay",message_id:"bench",index:0,final:true,delta:$d}')
    for i in 1 2 3; do "{{ binary }}" render <<<"$payload" > /dev/null; done
    start=$(date +%s%N)
    for i in $(seq 25); do "{{ binary }}" render <<<"$payload" > /dev/null; done
    end=$(date +%s%N)
    echo "render: $(( (end - start) / 25000000 )) ms, mean of 25 runs"

# The build directory alone reaches 1.9 GB with nickel-lang-core and LTO. The registered
# hooks run the copy in ~/.local/bin, so this leaves them working.
# Throw away everything a build leaves behind.
clean:
    #!/usr/bin/env nu
    let target = "{{ target_dir }}"
    let size = if ($target | path exists) { du $target | get apparent.0 } else { 0b }

    cargo clean

    # Profiling and scratch leftovers that .gitignore keeps out of the repo but
    # cargo knows nothing about.
    glob --no-dir "{{ justfile_directory() }}/{perf.data*,*.perf,flamegraph.svg,*.ncl.tmp}"
      | each {|file| rm --force $file; print $"removed ($file | path basename)" }
      | ignore

    print $"Freed ($size)."
