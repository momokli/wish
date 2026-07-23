#!/usr/bin/env bash
# ── wish playlist pipeline test harness ─────────────────────────────
# Usage:
#   bash scripts/test-playlist.sh                    # single run
#   bash scripts/test-playlist.sh --runs 3           # 3 runs, compare
#   bash scripts/test-playlist.sh --keep             # don't reset between runs
#   bash scripts/test-playlist.sh --track 5          # test only 5 tracks
#
# Prerequisites: curl, jq, spotify client creds in env

set -euo pipefail

# ── Config ──────────────────────────────────────────────────────────

PLAYLIST_ID="${PLAYLIST_ID:-2UCh0hUr8OXrMykCO4HkI3}"
BASE_URL="${BASE_URL:-https://wish.simonklimke.de}"
SPOTIFY_ID="${WISH_SPOTIFY_CLIENT_ID:-}"
SPOTIFY_SECRET="${WISH_SPOTIFY_CLIENT_SECRET:-}"
MAX_TRACKS="${MAX_TRACKS:-0}"   # 0 = all
RUNS=1
KEEP=false
TIMEOUT_SECS=600
MUSIC_HOST="${MUSIC_HOST:-momo@192.168.178.200}"

# ── Parse args ──────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
    case "$1" in
        --runs)   RUNS="$2"; shift 2 ;;
        --keep)   KEEP=true; shift ;;
        --track)  MAX_TRACKS="$2"; shift 2 ;;
        --timeout) TIMEOUT_SECS="$2"; shift 2 ;;
        *) echo "Unknown: $1"; exit 1 ;;
    esac
done

# ── Helpers ─────────────────────────────────────────────────────────

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; NC='\033[0m'

log()  { echo -e "${GREEN}[$(date +%H:%M:%S)]${NC} $*"; }
warn() { echo -e "${YELLOW}[$(date +%H:%M:%S)]${NC} $*"; }
err()  { echo -e "${RED}[$(date +%H:%M:%S)]${NC} $*"; }

# ── Spotify: fetch playlist tracks ──────────────────────────────────

fetch_playlist_tracks() {
    log "Fetching playlist tracks from Spotify..."

    if [[ -z "$SPOTIFY_ID" ]]; then
        # Try .env in repo root
        if [[ -f .env ]]; then
            export $(grep -v '^#' .env | xargs)
            SPOTIFY_ID="${WISH_SPOTIFY_CLIENT_ID:-}"
            SPOTIFY_SECRET="${WISH_SPOTIFY_CLIENT_SECRET:-}"
        fi
    fi

    if [[ -z "$SPOTIFY_ID" ]]; then
        err "WISH_SPOTIFY_CLIENT_ID not set. Source .env or export it."
        exit 1
    fi

    # Get access token
    local token
    token=$(curl -s -X POST https://accounts.spotify.com/api/token \
        -H "Content-Type: application/x-www-form-urlencoded" \
        -d "grant_type=client_credentials" \
        -d "client_id=$SPOTIFY_ID" \
        -d "client_secret=$SPOTIFY_SECRET" \
        | jq -r '.access_token')

    if [[ -z "$token" || "$token" == "null" ]]; then
        err "Failed to get Spotify access token"
        exit 1
    fi

    # Fetch all tracks (paginated)
    local url="https://api.spotify.com/v1/playlists/$PLAYLIST_ID/tracks?limit=50"
    local tracks=()
    local count=0

    while [[ -n "$url" ]]; do
        local resp
        resp=$(curl -s "$url" -H "Authorization: Bearer $token")
        local items
        items=$(echo "$resp" | jq -r '.items[].track | select(.id != null) | "\(.uri)|\(.name)|\(.artists[0].name)"')
        while IFS= read -r line; do
            [[ -z "$line" ]] && continue
            tracks+=("$line")
            count=$((count + 1))
            [[ "$MAX_TRACKS" -gt 0 && $count -ge "$MAX_TRACKS" ]] && break 2
        done <<< "$items"
        url=$(echo "$resp" | jq -r '.next // empty')
    done

    log "Resolved ${#tracks[@]} tracks from playlist"
    printf '%s\n' "${tracks[@]}"
}

# ── Submit all tracks ───────────────────────────────────────────────

submit_tracks() {
    local track_file="$1"
    local id=0
    while IFS='|' read -r uri title artist; do
        [[ -z "$uri" ]] && continue
        id=$((id + 1))
        local resp
        resp=$(curl -s -X POST "$BASE_URL/download" \
            -H 'Content-Type: application/json' \
            -d "{\"url\":\"$uri\",\"source\":\"spotify\"}")
        local sub_id
        sub_id=$(echo "$resp" | jq -r '.id // empty')
        if [[ -n "$sub_id" ]]; then
            echo "$sub_id|$title|$artist|pending|$uri"
        else
            local errmsg
            errmsg=$(echo "$resp" | jq -r '.error // "unknown"')
            warn "  failed to submit: $title ($errmsg)"
        fi
    done < "$track_file"
}

# ── Poll until done ─────────────────────────────────────────────────

poll_until_done() {
    local start=$(date +%s)
    while true; do
        local stats
        stats=$(curl -s "$BASE_URL/stats")
        local total ready pending failed
        total=$(echo "$stats" | jq -r '.total')
        ready=$(echo "$stats" | jq -r '.ready')
        pending=$(echo "$stats" | jq -r '.pending')
        failed=$(echo "$stats" | jq -r '.failed')

        local elapsed=$(($(date +%s) - start))
        printf "\r  %s/%s ready, %s pending, %s failed (%ss)  " "$ready" "$total" "$pending" "$failed" "$elapsed"

        if [[ "$pending" -eq 0 ]]; then
            echo ""
            log "All done: $ready ready, $failed failed (${elapsed}s)"
            return 0
        fi

        if [[ $elapsed -gt $TIMEOUT_SECS ]]; then
            echo ""
            warn "Timeout after ${TIMEOUT_SECS}s — $pending still pending"
            return 1
        fi

        sleep 2
    done
}

# ── Fetch full admin data ───────────────────────────────────────────

fetch_admin_data() {
    curl -s "$BASE_URL/admin/data"
}

# ── Validate ────────────────────────────────────────────────────────

validate_run() {
    local data="$1"
    local mismatches=0
    local total=0
    local per_layer_deemix=0 per_layer_deemix_ok=0
    local per_layer_spotdl=0 per_layer_spotdl_ok=0
    local per_layer_ytdlp=0 per_layer_ytdlp_ok=0

    echo ""
    echo "  VALIDATION:"
    printf "  %-4s %-8s %-8s %-30s %-45s %s\n" "ID" "STATUS" "VIA" "TITLE" "FILE" "MATCH"
    printf "  %-4s %-8s %-8s %-30s %-45s %s\n" "---" "------" "---" "-----" "----" "-----"

    while IFS='|' read -r id status title filename via; do
        [[ -z "$id" ]] && continue
        total=$((total + 1))

        local match="✗"
        if [[ "$status" != "ready" ]]; then
            match="—"
        elif [[ -n "$title" && -n "$filename" ]]; then
            # Check if title appears in filename (case-insensitive, first 8 chars)
            local short_title
            short_title=$(echo "$title" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]//g' | head -c 8)
            local short_file
            short_file=$(echo "$filename" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]//g')
            if [[ "$short_file" == *"$short_title"* ]]; then
                match="✓"
            fi
        fi

        # Per-layer tracking
        case "$via" in
            *deemix*) per_layer_deemix=$((per_layer_deemix + 1)); [[ "$match" == "✓" ]] && per_layer_deemix_ok=$((per_layer_deemix_ok + 1)) ;;
            *spotDL*) per_layer_spotdl=$((per_layer_spotdl + 1)); [[ "$match" == "✓" ]] && per_layer_spotdl_ok=$((per_layer_spotdl_ok + 1)) ;;
            *yt-dlp*) per_layer_ytdlp=$((per_layer_ytdlp + 1)); [[ "$match" == "✓" ]] && per_layer_ytdlp_ok=$((per_layer_ytdlp_ok + 1)) ;;
        esac

        if [[ "$match" == "✗" ]]; then
            mismatches=$((mismatches + 1))
            printf "  ${RED}%-4s %-8s %-8s %-30s %-45s ✗${NC}\n" "$id" "$status" "${via:0:8}" "${title:0:30}" "${filename:0:45}"
        else
            printf "  %-4s %-8s %-8s %-30s %-45s %s\n" "$id" "$status" "${via:0:8}" "${title:0:30}" "${filename:0:45}" "$match"
        fi
    done < <(echo "$data" | jq -r '.[] | "\(.id)|\(.status)|\(.track_title // "?")|\(.filename // "-")|\(.error_message // "-")"')

    echo ""
    if [[ $mismatches -eq 0 ]]; then
        log "  ✓ All $total files correctly associated"
    else
        warn "  ✗ $mismatches/$total MISMATCHES"
    fi

    # Per-layer summary
    echo ""
    echo "  PER LAYER:"
    printf "  %-10s %s\n" "deemix:"  "$per_layer_deemix_ok/$per_layer_deemix correct"
    printf "  %-10s %s\n" "spotDL:"  "$per_layer_spotdl_ok/$per_layer_spotdl correct"
    printf "  %-10s %s\n" "yt-dlp:"  "$per_layer_ytdlp_ok/$per_layer_ytdlp correct"

    return $mismatches
}

# ── Reset state ─────────────────────────────────────────────────────

reset_state() {
    log "Resetting DB and downloads via SSH..."
    if ! ssh -o ConnectTimeout=5 "$MUSIC_HOST" "sudo systemctl stop wish.service && rm -f /home/momo/wish/wish.db* /opt/music-stack/wish-downloads/*.mp3 /opt/music-stack/deemix-wish-config/queue/*.json && docker restart deemix-wish && sleep 2 && sudo systemctl start wish.service && echo ok" 2>/dev/null; then
        warn "SSH reset failed — trying manual..."
        warn "ssh $MUSIC_HOST 'sudo systemctl stop wish && rm -f /home/momo/wish/wish.db* /opt/music-stack/wish-downloads/*.mp3 && sudo systemctl start wish'"
        warn "Press enter after resetting..."
        read -r
    fi
    sleep 4
    local health
    health=$(curl -s "$BASE_URL/health" | jq -r '.status')
    if [[ "$health" != "ok" ]]; then
        err "Server not healthy after reset: $health"
        exit 1
    fi
    log "Reset complete, server healthy"
}

# ── Main ────────────────────────────────────────────────────────────

run_single() {
    local run_num="$1"

    echo ""
    echo "═══════════════════════════════════════════════════════════════"
    echo "  RUN $run_num/$RUNS"
    echo "═══════════════════════════════════════════════════════════════"

    # Fetch tracks
    local tmpfile
    tmpfile=$(mktemp /tmp/wish-test-tracks.XXXXXX)
    fetch_playlist_tracks > "$tmpfile"
    local track_count
    track_count=$(wc -l < "$tmpfile" | tr -d ' ')
    log "Will test $track_count tracks"

    # Reset if not keeping state
    if ! $KEEP; then
        reset_state
    fi

    # Submit all
    log "Submitting $track_count tracks..."
    local subfile
    subfile=$(mktemp /tmp/wish-test-subs.XXXXXX)
    submit_tracks "$tmpfile" > "$subfile"
    local sub_count
    sub_count=$(wc -l < "$subfile" | tr -d ' ')
    log "Submitted $sub_count/$track_count"

    # Poll
    poll_until_done || true

    # Fetch results
    local data
    data=$(fetch_admin_data)

    # Validate
    local mismatches
    validate_run "$data"
    mismatches=$?

    # Cleanup
    rm -f "$tmpfile" "$subfile"

    return $mismatches
}

# ── Entry ───────────────────────────────────────────────────────────

echo "wish playlist test harness"
echo "  playlist: $PLAYLIST_ID"
echo "  target:   $BASE_URL"
echo "  runs:     $RUNS"
echo "  max:      ${MAX_TRACKS:-all}"
echo ""

total_mismatches=0
for r in $(seq 1 "$RUNS"); do
    run_single "$r" || true
    # Don't keep state between runs unless --keep
    if [[ $r -lt $RUNS ]] && ! $KEEP; then
        KEEP=false  # reset resets each time
    fi
done

echo ""
if [[ $total_mismatches -eq 0 ]]; then
    log "All runs passed with zero mismatches"
else
    warn "$total_mismatches total mismatches across $RUNS runs"
fi
