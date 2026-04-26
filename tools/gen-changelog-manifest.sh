#!/usr/bin/env bash
# Scan changelog/*.md, parse YAML frontmatter, and write
# changelog/manifest.json. The web viewer at docs/patch-notes/ fetches this.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CHANGELOG="$ROOT/changelog"
MANIFEST="$CHANGELOG/manifest.json"

cd "$CHANGELOG"

# Collect files ending in .md, excluding README.md.
shopt -s nullglob
files=()
for f in *.md; do
    [[ "$f" == "README.md" ]] && continue
    files+=("$f")
done
shopt -u nullglob

# Pull a scalar value out of the YAML frontmatter of a single file.
field() {
    local file="$1" key="$2"
    awk -v k="$key" '
        /^---[[:space:]]*$/ { fence++; if (fence == 2) exit; next }
        fence == 1 {
            if (match($0, "^"k":[[:space:]]*(.*)$", m)) {
                v = m[1]
                gsub(/^["'\''[:space:]]+|["'\''[:space:]]+$/, "", v)
                print v
                exit
            }
        }
    ' "$file"
}

# Escape a string for JSON output.
json_escape() {
    python3 -c 'import json,sys; print(json.dumps(sys.stdin.read().rstrip("\n")))' <<<"$1"
}

{
    echo "["
    first=1
    # Sort by build number descending.
    for f in $(
        for f in "${files[@]}"; do
            b=$(field "$f" build)
            printf '%s\t%s\n' "${b:-0}" "$f"
        done | sort -k1,1 -rn | cut -f2
    ); do
        build=$(field "$f" build)
        codename=$(field "$f" codename)
        version=$(field "$f" version)
        date=$(field "$f" date)
        summary=$(field "$f" summary)

        [[ $first -eq 1 ]] || echo ","
        first=0

        printf '  {\n'
        printf '    "file": %s,\n' "$(json_escape "$f")"
        printf '    "build": %s,\n' "${build:-0}"
        printf '    "codename": %s,\n' "$(json_escape "${codename:-}")"
        printf '    "version": %s,\n' "$(json_escape "${version:-}")"
        printf '    "date": %s,\n' "$(json_escape "${date:-}")"
        printf '    "summary": %s\n' "$(json_escape "${summary:-}")"
        printf '  }'
    done
    echo
    echo "]"
} >"$MANIFEST"

echo "wrote $MANIFEST ($(wc -l <"$MANIFEST") lines)"
