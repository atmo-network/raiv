#!/usr/bin/env bash
# Check Markdown links in project context files: relative paths, heading anchors and
# release-pinned GitHub URLs. Offline; uses the pinned git tag when present, else the tree.
set -euo pipefail
export LC_ALL=C.UTF-8
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
PINNED='^https://github\.com/atmo-network/raiv/blob/(v[0-9][^/]*)/(.+)$'

prose() { awk '/^```/ { fence = !fence; next } !fence'; }
# GitHub heading anchors: drop link targets and punctuation, keep word characters, hyphens, spaces.
anchors() {
    prose | sed -nE 's/^#{1,6} (.*)$/\1/p' | sed -E 's/\]\([^)]*\)/]/g' |
        tr '[:upper:]' '[:lower:]' | sed -E 's/[^[:alnum:]_ -]//g; s/ /-/g'
}
has_anchor() { [[ -z $2 ]] || anchors <<< "$1" | grep -Fxq -- "$2"; }

docs=0 broken=0
while IFS= read -r -d '' doc; do
    docs=$((docs + 1))
    dir=$(dirname -- "$doc")
    while IFS= read -r target; do
        path=${target%%#*} fragment=
        [[ $target == *'#'* ]] && fragment=${target#*#}
        if [[ $path =~ $PINNED ]]; then
            tag=${BASH_REMATCH[1]} file=${BASH_REMATCH[2]}
            if content=$(git -C "$ROOT" show "$tag:$file" 2> /dev/null) || content=$(cat -- "$ROOT/$file" 2> /dev/null); then
                has_anchor "$content" "$fragment" && continue
            fi
        elif [[ $target =~ ^[a-z]+: ]]; then
            continue
        else
            file=${path:+$dir/$path}
            file=${file:-$doc}
            if [[ -e $file ]]; then
                [[ $file != *.md || -z $fragment ]] && continue
                has_anchor "$(< "$file")" "$fragment" && continue
            fi
        fi
        printf 'broken link: %s: %s\n' "${doc#"$ROOT"/}" "$target" >&2
        broken=$((broken + 1))
    done < <(prose < "$doc" | grep -oE '\]\([^)[:space:]]+\)' | sed -E 's/^\]\((.*)\)$/\1/' || true)
done < <(find "$ROOT" \( -name target -o -name var -o -name .git -o -name node_modules \) -prune -o -name '*.md' -print0 | sort -z)
printf 'checked %s Markdown files; %s broken links\n' "$docs" "$broken"
(( broken == 0 ))
