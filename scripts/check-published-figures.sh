#!/usr/bin/env bash
# Enforces product.md's evidence rule: no figure enters a document before it has
# been measured. Every latency and WER figure in the published documents must be
# the figure the newest live evaluation run measured, under the label that run
# measured it under — a figure in the right document under the wrong name is
# still a wrong figure.
#
# What it reads:
#   evals/runs-live/*.json                the newest one, and only that one
#   README.md, docs/architecture.md       every figure in them
#   crates/harness/OPTIMISATION-LOG.md    its newest entry only — the entries
#                                         below it are history, and some describe
#                                         a system that no longer exists
set -euo pipefail

cd "$(dirname "$0")/.."
fail=0

err() {
  echo "published figures: $*" >&2
  fail=1
}

# --- The run -----------------------------------------------------------------

run=$(find evals/runs-live -maxdepth 1 -name '*.json' | sort | tail -1)
if [ -z "$run" ]; then
  err "nothing has been measured: no run file under evals/runs-live/"
  exit 1
fi

# The newest log entry is everything from the first "## " heading after the
# conditions table down to the one below it.
entry_file=$(mktemp)
trap 'rm -f "$entry_file"' EXIT
awk '
  /^## / && seen { exit }
  /^## / && !/^## Conditions$/ && !/^## Optimisation Log$/ { seen = 1 }
  seen { print }
' crates/harness/OPTIMISATION-LOG.md > "$entry_file"

documents=(README.md docs/architecture.md "$entry_file")
name_of() {
  case "$1" in
  "$entry_file") echo "OPTIMISATION-LOG.md (newest entry)" ;;
  *) echo "$1" ;;
  esac
}

solver=$(jq -r '.solver // "missing"' "$run")
build=$(jq -r '.build // "absent"' "$run")
[ "$solver" = "live" ] || err "$run: solver is \"$solver\", not \"live\" — component runs do not measure what a caller waits"

# `build` is written by every run since the field was added. An older file
# predates it and cannot say; a file that does say must say release, because a
# debug build inflates a stage by half again.
if [ "$build" != "absent" ] && [ "$build" != "release" ]; then
  err "$run: build is \"$build\" — figures come from release builds (product.md §Evidence)"
fi

ok=$(jq -r '.summary.samples_ok // 0' "$run")
[ "$ok" -ge 14 ] || err "$run: samples_ok is $ok, fewer than the 14 of the 2026-08-15 run — the percentiles cover a hole"

# Provenance. A document quoting figures names the run they came from, and it has
# to be the run this script checks against — otherwise a newer run lands, the
# published figures go stale, and they all still pass because they were once true.
for doc in "${documents[@]}"; do
  cited=$(grep -oE 'evals/runs-live/[^ )`]+\.json' "$doc" | sort -u || true)
  count=$(printf '%s' "$cited" | grep -c . || true)
  if [ "$count" -eq 0 ]; then
    err "$(name_of "$doc"): quotes no run file; a figure without its provenance cannot be checked"
  elif [ "$count" -gt 1 ]; then
    err "$(name_of "$doc"): quotes more than one run file, so which one its figures came from is a guess"
  elif [ "$cited" != "$run" ]; then
    err "$(name_of "$doc"): quotes $cited, but the newest run is $run — the figures are stale"
  fi
done

# --- What the run measured ---------------------------------------------------
#
# Milliseconds are published rounded to the integer, WER as a percentage to one
# decimal.

ms() { jq -r "$1 | round" "$run"; }
pct() { jq -r "$1 | . * 1000 | round | . / 10" "$run" | sed 's/\.0$//'; }

system_p50=$(ms '.summary.system_response.p50')
system_p95=$(ms '.summary.system_response.p95')
perceived_p50=$(ms '.summary.perceived_latency.p50')
perceived_p95=$(ms '.summary.perceived_latency.p95')
summary_ms=$(printf '%s\n' "$system_p50" "$system_p95" "$perceived_p50" "$perceived_p95" | sort -u)
# The endpointing values the run was taken under. A document explaining why the
# two latency figures differ has to be able to name the hangover, and only a line
# talking about endpointing may quote one.
config_ms=$(jq -r '.config | (.silence_flush_ms, .min_utterance_ms, .min_speech_confirm_ms)' "$run" | sort -u)
wer_all=$(
  printf '%s\n' "$(pct '.summary.corpus_wer')" "$(pct '.summary.wer_p50')" "$(pct '.summary.wer_p90')" |
    sort -u
)

member() { echo "$2" | grep -qx "$1"; }

# --- Every figure, under the label it was measured under ---------------------

for doc in "${documents[@]}"; do
  name=$(name_of "$doc")
  line_number=0
  while IFS= read -r line; do
    line_number=$((line_number + 1))

    found_ms=$(echo "$line" | grep -oE '[0-9]+ ?ms' | grep -oE '[0-9]+' | tr '\n' ' ' | sed 's/ $//' || true)
    if [ -n "$found_ms" ]; then
      case "$line" in
      *"system response"* | *"System response"*) expected="$system_p50 $system_p95" ;;
      *"perceived latency"* | *"Perceived latency"*) expected="$perceived_p50 $perceived_p95" ;;
      *) expected="" ;;
      esac

      if [ -n "$expected" ]; then
        # A row naming one of the two figures carries that figure's p50 and p95,
        # in that order. Quoting the other figure's numbers under this name, or
        # swapping the pair, is what this catches.
        if [ "$found_ms" != "$expected" ] &&
          [ "$found_ms" != "${expected%% *}" ] &&
          [ "$found_ms" != "${expected##* }" ]; then
          err "$name:$line_number: $found_ms is not what the run measured under that name (${expected})"
        fi
      else
        for value in $found_ms; do
          case "$line" in
          *hangover* | *endpoint* | *silence*)
            member "$value" "$config_ms" ||
              err "$name:$line_number: $value ms is not an endpointing value in $run"
            ;;
          *)
            member "$value" "$summary_ms" ||
              err "$name:$line_number: $value ms is not a figure in $run"
            ;;
          esac
        done
      fi
    fi

    found_pct=$(echo "$line" | grep -oE '[0-9]+(\.[0-9])? ?%' | grep -oE '[0-9]+(\.[0-9])?' || true)
    for value in $found_pct; do
      member "$value" "$wer_all" ||
        err "$name:$line_number: $value% is not an error rate in $run"
    done
  done < "$doc"
done

# --- How the figures are presented -------------------------------------------

# ADR-0010: the two latency figures are always reported together.
for doc in "${documents[@]}"; do
  name=$(name_of "$doc")
  system=$(grep -ci 'system response' "$doc" || true)
  perceived=$(grep -ci 'perceived latency' "$doc" || true)
  if [ "$system" -gt 0 ] && [ "$perceived" -eq 0 ]; then
    err "$name: system response is reported without perceived latency (ADR-0010)"
  fi
  if [ "$perceived" -gt 0 ] && [ "$system" -eq 0 ]; then
    err "$name: perceived latency is reported without system response (ADR-0010)"
  fi
done

# A WER figure without the caveat the run carries is a figure read as an
# instrument, which at this set size it is not.
for doc in "${documents[@]}"; do
  name=$(name_of "$doc")
  if grep -q 'WER' "$doc" && ! grep -qiE 'tripwire|points absolute|confidence interval' "$doc"; then
    err "$name: a WER figure is published without the caveat about the size of the set"
  fi
done

# The conditions are part of the figure: how many epochs the percentiles cover,
# and — while a run predating the `build` field is still the newest — that the
# build was not recorded.
epochs=$(jq -r '.epochs // 0' "$run")
grep -qi 'epoch' "$entry_file" ||
  err "OPTIMISATION-LOG.md (newest entry): does not say the figures come from $epochs epoch(s); the sample count is part of the figure"
if [ "$build" = "absent" ]; then
  grep -qiE 'build [a-z ]{0,12}not recorded|does not record the build' "$entry_file" ||
    err "OPTIMISATION-LOG.md (newest entry): $run predates the build field, and the entry does not say so"
fi

# --- What the documents must not say -----------------------------------------

# Requirements state the target; the measurement lives in the log.
if grep -Eq '[0-9]+ ?ms' docs/product.md; then
  err "docs/product.md: a measurement belongs in the optimisation log, not in the requirements"
fi

# Android is the product surface and is not built. No document may present it as
# something a caller can use. docs/adr/ is not read: accepted records are history
# and are never edited.
if grep -qEi '^\| Client \|.*Android|client .*is Android|Android in production' docs/product.md docs/architecture.md; then
  err "Android is presented as an existing client; it is still to be built"
fi

if [ "$fail" -eq 0 ]; then
  echo "published figures: consistent with $run"
fi
exit "$fail"
