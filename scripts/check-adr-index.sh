#!/usr/bin/env bash
# Enforces the five rules stated in docs/adr/README.md. An index that drifts is
# worse than none.
set -euo pipefail

cd "$(dirname "$0")/.."
adr_dir="docs/adr"
index="$adr_dir/README.md"
fail=0

err() {
  echo "ADR index: $*" >&2
  fail=1
}

# The closed tag vocabulary is read from the index's "By tag" line rather than
# duplicated here, so the two cannot disagree.
mapfile -t vocabulary < <(
  grep -E '^`[a-z]+` [0-9]' "$index" |
    grep -oE '`[a-z]+`' |
    tr -d '`' |
    sort -u
)
if [ ${#vocabulary[@]} -eq 0 ]; then
  err "could not read the tag vocabulary from the 'By tag' line"
fi

mapfile -t files < <(find "$adr_dir" -maxdepth 1 -name '[0-9][0-9][0-9][0-9]-*.md' -printf '%f\n' | sort)
mapfile -t rows < <(grep -oE '^\| \[[0-9]{4}\]\([^)]+\)' "$index" | grep -oE '[0-9]{4}\]\([^)]+' | sed 's/^[0-9]*\](//' | sort)

# 1. Every file has exactly one row, and every row has a file.
printf '%s\n' "${files[@]}" > /tmp/adr-files.$$
printf '%s\n' "${rows[@]}" > /tmp/adr-rows.$$
while read -r missing; do
  [ -n "$missing" ] && err "file has no row in the index: $missing"
done < <(comm -23 /tmp/adr-files.$$ /tmp/adr-rows.$$)
while read -r orphan; do
  [ -n "$orphan" ] && err "row points at a file that does not exist: $orphan"
done < <(comm -13 /tmp/adr-files.$$ /tmp/adr-rows.$$)
rm -f /tmp/adr-files.$$ /tmp/adr-rows.$$

duplicates=$(printf '%s\n' "${rows[@]}" | uniq -d)
[ -n "$duplicates" ] && err "more than one row for: $duplicates"

# 2. Numbers are unique and contiguous from 0001.
expected=1
for f in "${files[@]}"; do
  n=$((10#${f:0:4}))
  if [ "$n" -ne "$expected" ]; then
    err "numbering is not contiguous: expected $(printf '%04d' "$expected"), found $f"
    break
  fi
  expected=$((expected + 1))
done

for f in "${files[@]}"; do
  path="$adr_dir/$f"
  number="${f:0:4}"

  # 3. Status is one of the four permitted forms.
  status=$(grep -m1 -oE '^\- \*\*Status\*\*: .*$' "$path" | sed 's/^- \*\*Status\*\*: //' || true)
  case "$status" in
    Accepted | Proposed | Deprecated) ;;
    "Superseded by ADR-"[0-9][0-9][0-9][0-9])
      # 5. A superseded record names a replacement that exists.
      replacement="${status##*ADR-}"
      if ! ls "$adr_dir/$replacement-"*.md > /dev/null 2>&1; then
        err "ADR-$number is superseded by ADR-$replacement, which does not exist"
      fi
      ;;
    "") err "ADR-$number has no Status line" ;;
    *) err "ADR-$number has an invalid Status: '$status'" ;;
  esac

  # 4. Every tag used is in the closed vocabulary.
  tags=$(grep -m1 -oE '^\- \*\*Tags\*\*: .*$' "$path" | grep -oE '`[a-z]+`' | tr -d '`' || true)
  for tag in $tags; do
    found=0
    for known in "${vocabulary[@]}"; do
      [ "$tag" = "$known" ] && found=1 && break
    done
    [ "$found" -eq 0 ] && err "ADR-$number uses tag '$tag', which is not in the vocabulary"
  done
done

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "ADR index: ${#files[@]} records, consistent."
