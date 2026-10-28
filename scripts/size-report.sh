#!/usr/bin/env bash
# Size report for the Rust sources: lists the largest files and warns (as GitHub annotations in
# CI) about source files over MAX_FILE lines and functions over MAX_FN lines. It never fails the
# build; clippy's too_many_lines lint (clippy.toml) is what enforces the function limit.
set -euo pipefail

MAX_FILE="${MAX_FILE:-800}"
MAX_FN="${MAX_FN:-100}"
cd "$(dirname "$0")/.."

warn() {  # warn file line message
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
        echo "::warning file=$1,line=$2::$3"
    else
        echo "warning: $1:$2: $3"
    fi
}

mapfile -t files < <(git ls-files '*.rs')

echo "Largest source files:"
wc -l "${files[@]}" | grep -v ' total$' | sort -rn | head -10 | sed 's/^/  /'

warnings=0
for f in "${files[@]}"; do
    lines=$(wc -l < "$f")
    if [ "$lines" -gt "$MAX_FILE" ]; then
        warn "$f" 1 "$lines lines (guideline: at most $MAX_FILE) - consider splitting this file"
        warnings=$((warnings + 1))
    fi
    # Function length by brace matching; strings and comments are stripped first, so braces
    # inside them don't count.
    while IFS=: read -r line len name; do
        warn "$f" "$line" "fn $name is $len lines (guideline: at most $MAX_FN)"
        warnings=$((warnings + 1))
    done < <(awk -v max="$MAX_FN" '
        function strip(s) {
            gsub(/\\./, "", s); gsub(/"[^"]*"/, "\"\"", s); gsub(/'\''[^'\'']'\''/, "", s)
            sub(/\/\/.*/, "", s); return s
        }
        {
            s = strip($0)
            if (!infn && match(s, /(^|[^a-z_])fn [A-Za-z_][A-Za-z0-9_]*/)) {
                name = substr(s, RSTART, RLENGTH); sub(/.*fn /, "", name)
                infn = 1; start = NR; depth = 0; opened = 0
            }
            if (infn) {
                n = split(s, ch, "")
                for (i = 1; i <= n; i++) {
                    if (ch[i] == "{") { depth++; opened = 1 }
                    else if (ch[i] == "}") depth--
                }
                if (!opened && s ~ /;[ \t]*$/) infn = 0
                else if (opened && depth <= 0) {
                    if (NR - start + 1 > max) print start ":" NR - start + 1 ":" name
                    infn = 0
                }
            }
        }' "$f")
done

echo "Size report: $warnings warning(s) (files > $MAX_FILE lines, functions > $MAX_FN lines)."
