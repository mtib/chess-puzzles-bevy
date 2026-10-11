#!/bin/sh
# Pick one puzzle near a target ELO.
#
# Serves from the local cache (one JSON puzzle per line) and avoids the
# puzzles used most recently. The cache is seeded by seed.py from the Lichess
# puzzle database; if it is missing or empty, this script falls back to the
# anonymous Lichess /api/puzzle/next endpoint and appends what it fetched, so
# the cache grows on its own.
#
# Needs curl and jq.
#
# $1 = target ELO

elo="$1"
case "$elo" in ''|*[!0-9]*) elo=1500;; esac

dir="${XDG_STATE_HOME:-$HOME/.local/state}/omarchy/chesspuzzles"
cache="$dir/cache.jsonl"
recent="$dir/recent.txt"
mkdir -p "$dir"
touch "$cache" "$recent"

# Online fallback + cache refill: one flat record, or failure.
online_puzzle() {
  curl -fsS --max-time 8 "https://lichess.org/api/puzzle/next" 2>/dev/null |
    jq -ce '.puzzle
      | select(.fen != null and (.solution // [] | length) > 0)
      | {id: (.id|tostring), fen, rating: (.rating // 0),
         themes: ((.themes // [])[:3]), solution}' 2>/dev/null
}

[ -s "$cache" ] || online_puzzle >>"$cache"

pick=$(jq -nc --arg elo "$elo" --rawfile recent "$recent" --rawfile cache "$cache" '
  ($recent | split("\n") | map(select(length > 0))) as $seen
  | ($cache | split("\n") | map(select(length > 0)) | map(fromjson)
      | map(select(.fen != null and (.solution | length) > 0))) as $rows
  | ($rows | map(select(.id as $id | $seen | index($id) | not))) as $fresh
  | (if ($fresh | length) > 0 then $fresh else $rows end) as $pool
  | ($elo | tonumber) as $target
  | $pool | sort_by(if (.rating - $target) < 0 then -(.rating - $target)
      else (.rating - $target) end)
    | .[0] // empty
')

# Cache is empty and the network is down: nothing to serve.
[ -n "$pick" ] || exit 1

# Record the served id so the next pick is a different puzzle.
printf '%s\n' "$pick" | jq -r '.id' >>"$recent"
tail -n 80 "$recent" >"$recent.tmp" && mv "$recent.tmp" "$recent"

printf '%s\n' "$pick"
