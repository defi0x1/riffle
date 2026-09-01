#!/bin/sh
# CI guard for the provenance policy (see plans/09 in the private planning repo,
# which is not part of this tree). Fails the build if any forbidden internal
# identifier appears in tracked source.
#
# NOTE: this script has to grep for the forbidden terms in order to find them.
# That is the one place the policy allows them to appear at all -- everywhere
# else in this repository they must never show up. Do not "fix" this by
# removing or obfuscating the pattern below.
set -eu

TERMS='datapipe|cp-amm-tracker|rpc-relayer|blacklist-rs|blacklist-client|token-service|meteora-interface|lb-clmm-ext|yellowstone-vixen|swaps_agg|pool_historical_tvl|position_with_bin|cumulative_volume_snapshots|min_token_ratio|calculate_tvl_with_token_ratio|m031_create_bins_table'

# References to the private planning repository. These leak its structure, and its
# phrasing can leak its contents; they also read as machine-generated. The reason belongs
# in the comment, not a document coordinate.
PLANREFS='plans/[0-9]|[0-9][0-9]-[a-z-]+\.md|§|\[A-?[0-9]'

SELF='scripts/provenance-check.sh'

cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"

if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    FILES=$(git ls-files -- '*.rs' '*.sql' '*.toml' '*.md' '*.yaml' '*.yml' | grep -v -F "$SELF" || true)
else
    FILES=$(find . -type d \( -name .git -o -name target \) -prune -o \
        -type f \( -name '*.rs' -o -name '*.sql' -o -name '*.toml' -o -name '*.md' \
                   -o -name '*.yaml' -o -name '*.yml' \) -print \
        | sed 's#^\./##' | grep -v -F "$SELF" || true)
fi

found=0
for f in $FILES; do
    [ -f "$f" ] || continue
    if grep -niE "$TERMS|$PLANREFS" "$f" >/dev/null 2>&1; then
        grep -niE "$TERMS|$PLANREFS" "$f" | while IFS=: read -r lineno rest; do
            echo "provenance: forbidden term in $f:$lineno: $rest"
        done
        found=1
    fi
done

if [ "$found" -ne 0 ]; then
    echo "provenance-check: forbidden internal identifiers found; see the provenance policy" >&2
    exit 1
fi

exit 0
