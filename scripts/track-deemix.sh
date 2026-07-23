#!/usr/bin/env bash
# ── Deemix per-track analysis ───────────────────────────────────────
# Submits tracks one at a time, records deemix UUID + status + errors.
# Usage: bash scripts/track-deemix.sh [--count N] [--all]
set -euo pipefail

BASE="${BASE:-https://wish.simonklimke.de}"
DEEMIX="${DEEMIX:-http://192.168.178.200:6598}"
PLAYLIST_ID="${PLAYLIST_ID:-2UCh0hUr8OXrMykCO4HkI3}"
COUNT="${COUNT:-10}"

# Fetch playlist tracks
echo "=== Resolving playlist $PLAYLIST_ID ==="
SPOTIFY_TOKEN=$(curl -s -X POST https://accounts.spotify.com/api/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=client_credentials" \
  -d "client_id=$WISH_SPOTIFY_CLIENT_ID" \
  -d "client_secret=$WISH_SPOTIFY_CLIENT_SECRET" | python3 -c "import sys,json;print(json.load(sys.stdin)['access_token'])")

TRACKS=$(mktemp)
curl -s "https://api.spotify.com/v1/playlists/$PLAYLIST_ID/tracks?limit=100" \
  -H "Authorization: Bearer $SPOTIFY_TOKEN" \
  | python3 -c "
import sys,json
for item in json.load(sys.stdin)['items']:
    t = item.get('track')
    if t and t.get('uri'):
        print(f\"{t['uri']}|{t['name']}|{t['artists'][0]['name']}\")
" > "$TRACKS"

TOTAL=$(wc -l < "$TRACKS" | tr -d ' ')
echo "Resolved $TOTAL tracks"
[[ "$COUNT" != "all" && $TOTAL -gt $COUNT ]] && head -n "$COUNT" "$TRACKS" > "$TRACKS.tmp" && mv "$TRACKS.tmp" "$TRACKS"
echo "Testing ${COUNT} tracks"
echo ""

# SSH wrapper for music host
music() { ssh momo@192.168.178.200 "$@"; }

# Reset
echo "=== Resetting ==="
music "sudo systemctl stop wish.service"
music "rm -f /home/momo/wish/wish.db* /opt/music-stack/wish-downloads/*.mp3 /opt/music-stack/deemix-wish-config/queue/*.json"
music "docker restart deemix-wish" > /dev/null 2>&1
sleep 2
music "sudo systemctl start wish.service"
sleep 4

# Submit and track
echo ""
printf "%-4s %-35s %-12s %-30s %s\n" "ID" "TITLE" "DEEMIX" "UUID" "ERROR"
printf "%-4s %-35s %-12s %-30s %s\n" "---" "-----" "------" "----" "-----"

ID=0
while IFS='|' read -r uri title artist; do
    [[ -z "$uri" ]] && continue
    ID=$((ID + 1))

    # Submit
    RESP=$(curl -s -X POST "$BASE/download" -H 'Content-Type: application/json' -d "{\"url\":\"$uri\",\"source\":\"spotify\"}")
    SUB_ID=$(echo "$RESP" | python3 -c "import sys,json;print(json.load(sys.stdin).get('id',''))" 2>/dev/null)
    [[ -z "$SUB_ID" ]] && { echo "  skip: already submitted?"; continue; }

    # Wait for processing
    for i in $(seq 1 30); do
        sleep 2
        STATUS=$(curl -s "$BASE/queue" | python3 -c "
import sys,json
for t in json.load(sys.stdin)['tasks']:
    if t['id'] == $SUB_ID:
        print(t['status'])
        break
" 2>/dev/null)
        [[ "$STATUS" == "ready" || "$STATUS" == "failed" ]] && break
    done

    # Get the via info
    VIA=$(curl -s "$BASE/admin/data" | python3 -c "
import sys,json
for t in json.load(sys.stdin):
    if t['id'] == $SUB_ID:
        print(t.get('error_message',''))
        break
" 2>/dev/null)
    FN=$(curl -s "$BASE/admin/data" | python3 -c "
import sys,json
for t in json.load(sys.stdin):
    if t['id'] == $SUB_ID:
        print(t.get('filename','-'))
        break
" 2>/dev/null)

    # Check deemix for this track's errors
    DM_ERR=""
    if [[ "$VIA" != *"deemix"* ]]; then
        # It didn't go through deemix — check why
        QUEUE_DATA=$(music "curl -s http://localhost:6598/api/connect" 2>/dev/null)
        DM_ERR=$(echo "$QUEUE_DATA" | python3 -c "
import sys,json
q = json.load(sys.stdin).get('queue',{}).get('queue',{})
for uuid,item in q.items():
    if item.get('title','').lower()[:10] == '$title'[:10].lower():
        errs = item.get('errors',[])
        if errs:
            print(errs[0].get('message','')[:60])
        elif item.get('status') != 'completed':
            print(f\"status={item.get('status')}\")
        break
" 2>/dev/null)
    fi

    [[ -z "$DM_ERR" ]] && DM_ERR="-"

    printf "%-4s %-35s %-12s %-30s %s\n" \
        "$SUB_ID" "${title:0:35}" "${VIA:0:12}" "${FN:0:30}" "$DM_ERR"

done < "$TRACKS"

rm -f "$TRACKS"
