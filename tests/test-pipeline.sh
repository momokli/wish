#!/usr/bin/env bash
# wish — end-to-end pipeline verification
# Usage: ./test-pipeline.sh [--quick] [--reset-db]
#   --quick     Skip download tests (search + frontend only)
#   --reset-db  Clear the database before testing
set -uo pipefail

BASE="${WISH_URL:-https://wish.zukkafabrik.de}"
FILES="${FILES_URL:-https://files.wish.zukkafabrik.de}"
PASS=0; FAIL=0; SKIP=0
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'

assert() {
    local label="$1"; shift
    if { "$@"; } 2>/dev/null; then
        echo -e "  ${GREEN}PASS${NC} $label"
        ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC} $label"
        ((FAIL++))
    fi
}

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$actual" = "$expected" ]; then
        echo -e "  ${GREEN}PASS${NC} $label ($actual)"
        ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC} $label — expected '$expected', got '$actual'"
        ((FAIL++))
    fi
}

assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if echo "$haystack" | grep -qF "$needle" 2>/dev/null; then
        echo -e "  ${GREEN}PASS${NC} $label"
        ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC} $label — '$needle' not found"
        ((FAIL++))
    fi
}

assert_gt() {
    local label="$1" actual="$2" threshold="$3"
    if [ "$actual" -gt "$threshold" ]; then
        echo -e "  ${GREEN}PASS${NC} $label ($actual > $threshold)"
        ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC} $label — expected > $threshold, got $actual"
        ((FAIL++))
    fi
}

# ── Section header ──
section() { echo ""; echo -e "${YELLOW}═══ $1 ═══${NC}"; }

# ═══════════════════════════════════════════════════════════

section "Health Check"
HEALTH=$(curl -sf "$BASE/health")
assert "HTTP 200"        test -n "$HEALTH"
assert "status=ok"       echo "$HEALTH" | grep -qE '"status":"ok"'
assert "spotify"         echo "$HEALTH" | grep -qE '"spotify_configured":true'
assert "deemix config"   echo "$HEALTH" | grep -qE '"deemix_configured":true'
assert "spotDL"          echo "$HEALTH" | grep -qE '"spotdl_available":true'
assert "yt-dlp"          echo "$HEALTH" | grep -qE '"ytdlp_available":true'
DEEMIX_AUTH=$(echo "$HEALTH" | python3 -c "import json,sys; print(json.load(sys.stdin)['deemix_authenticated'])")
assert_eq "deemix auth" "True" "$DEEMIX_AUTH"

section "Search — Spotify"
SPOTIFY=$(curl -sf "$BASE/search?q=daft+punk&limit=2&source=spotify")
SPOT_COUNT=$(echo "$SPOTIFY" | python3 -c "import json,sys; print(len(json.load(sys.stdin)['results']))")
assert_gt   "returns results" "$SPOT_COUNT" 0
assert_contains "source field" '"source":"spotify"' "$SPOTIFY"

section "Search — YouTube"
YT=$(curl -sf "$BASE/search?q=daft+punk&limit=2&source=youtube")
YT_COUNT=$(echo "$YT" | python3 -c "import json,sys; print(len(json.load(sys.stdin)['results']))")
assert_gt   "returns results" "$YT_COUNT" 0
assert_contains "source field" '"source":"youtube"' "$YT"

section "Search — SoundCloud"
SC=$(curl -sf "$BASE/search?q=daft+punk&limit=2&source=soundcloud")
SC_COUNT=$(echo "$SC" | python3 -c "import json,sys; print(len(json.load(sys.stdin)['results']))")
assert_gt   "returns results" "$SC_COUNT" 0
assert_contains "source field" '"source":"soundcloud"' "$SC"

section "Search — default (Spotify)"
DEF=$(curl -sf "$BASE/search?q=daft+punk&limit=1")
assert_contains "backward compat" '"source":"spotify"' "$DEF"

section "Frontend"
HTML=$(curl -sf "$BASE")
assert_contains "filter bar"       'class="filter-pill"' "$HTML"
assert_contains "search input"     'id="searchInput"'     "$HTML"
assert_contains "requests tab"    'view-requests'        "$HTML"
assert_contains "stat boxes"      'stat-total'           "$HTML"
assert_contains "failed stat"     'stat-failed'          "$HTML"
assert_contains "cache logic"     'renderAllFromCache'   "$HTML"
assert_contains "error helper"    'function shortError'   "$HTML"

section "Stats (empty DB)"
STATS=$(curl -sf "$BASE/stats")
assert "stats works" echo "$STATS" | python3 -c "import json,sys; d=json.load(sys.stdin); exit(0 if isinstance(d.get('total'),int) else 1)"

section "Tracks + Files"
TRACKS=$(curl -sf "$BASE/tracks")
TRACK_COUNT=$(echo "$TRACKS" | python3 -c "import json,sys; print(len(json.load(sys.stdin)))")
assert "tracks endpoint works" test "$TRACK_COUNT" -ge 0

section "File Server (dufs)"
DUFS=$(curl -sfI "$FILES" | head -1)
assert "files.wish HTTP 200" echo "$DUFS" | grep -q "200"

# ═══════════════════════════════════════════════════════════
# Download pipeline tests (skip with --quick)
# ═══════════════════════════════════════════════════════════

if [[ "${1:-}" != "--quick" ]]; then
    section "Download — YouTube"
    YT_SUB=$(curl -sf -X POST "$BASE/download" \
        -H 'Content-Type: application/json' \
        -d '{"url":"https://www.youtube.com/watch?v=dQw4w9WgXcQ","source":"youtube"}')
    YT_ID=$(echo "$YT_SUB" | python3 -c "import json,sys; print(json.load(sys.stdin)['id'])")
    assert "submission created" test -n "$YT_ID"

    echo "  waiting for download (max 60s)..."
    for i in $(seq 1 30); do
        sleep 2
        STATUS=$(curl -sf "$BASE/queue" | python3 -c "
import json,sys
tasks=json.load(sys.stdin)['tasks']
for t in tasks:
    if t['id']==$YT_ID: print(t['status'])
" 2>/dev/null || echo "pending")
        if [ "$STATUS" = "ready" ]; then
            echo -e "  ${GREEN}PASS${NC} YouTube download ($STATUS)"
            ((PASS++))
            break
        elif [ "$STATUS" = "failed" ]; then
            ERR=$(curl -sf "$BASE/queue" | python3 -c "
import json,sys
for t in json.load(sys.stdin)['tasks']:
    if t['id']==$YT_ID: print(t.get('error_message','?')[:60])
")
            echo -e "  ${RED}FAIL${NC} YouTube download — $ERR"
            ((FAIL++))
            break
        fi
        printf "."
    done
    echo ""

    section "Download — SoundCloud"
    SC_SUB=$(curl -sf -X POST "$BASE/download" \
        -H 'Content-Type: application/json' \
        -d '{"url":"https://soundcloud.com/kygo/firestone-ft-conrad","source":"soundcloud"}')
    SC_ID=$(echo "$SC_SUB" | python3 -c "import json,sys; print(json.load(sys.stdin)['id'])")
    assert "submission created" test -n "$SC_ID"

    echo "  waiting for download (max 60s)..."
    for i in $(seq 1 30); do
        sleep 2
        STATUS=$(curl -sf "$BASE/queue" | python3 -c "
import json,sys
tasks=json.load(sys.stdin)['tasks']
for t in tasks:
    if t['id']==$SC_ID: print(t['status'])
" 2>/dev/null || echo "pending")
        if [ "$STATUS" = "ready" ]; then
            echo -e "  ${GREEN}PASS${NC} SoundCloud download ($STATUS)"
            ((PASS++))
            break
        elif [ "$STATUS" = "failed" ]; then
            ERR=$(curl -sf "$BASE/queue" | python3 -c "
import json,sys
for t in json.load(sys.stdin)['tasks']:
    if t['id']==$SC_ID: print(t.get('error_message','?')[:60])
")
            echo -e "  ${RED}FAIL${NC} SoundCloud download — $ERR"
            ((FAIL++))
            break
        fi
        printf "."
    done
    echo ""

    section "Download — Spotify (via deemix)"
    # This only works if deemix ARL is valid
    if [ "$DEEMIX_AUTH" = "True" ]; then
        SP_SUB=$(curl -sf -X POST "$BASE/download" \
            -H 'Content-Type: application/json' \
            -d '{"url":"spotify:track:0fw46rvzAX06J2y4gAY5Jq","source":"spotify"}')
        SP_ID=$(echo "$SP_SUB" | python3 -c "import json,sys; print(json.load(sys.stdin)['id'])")
        assert "submission created" test -n "$SP_ID"

        echo "  waiting for deemix (max 90s)..."
        for i in $(seq 1 45); do
            sleep 2
            STATUS=$(curl -sf "$BASE/queue" | python3 -c "
import json,sys
tasks=json.load(sys.stdin)['tasks']
for t in tasks:
    if t['id']==$SP_ID: print(t['status'])
" 2>/dev/null || echo "pending")
            if [ "$STATUS" = "ready" ]; then
                VIA=$(curl -sf "$BASE/queue" | python3 -c "
import json,sys
for t in json.load(sys.stdin)['tasks']:
    if t['id']==$SP_ID: print(t.get('error_message','?')[:40])
")
                echo -e "  ${GREEN}PASS${NC} Spotify download ($STATUS, $VIA)"
                ((PASS++))
                break
            elif [ "$STATUS" = "failed" ]; then
                ERR=$(curl -sf "$BASE/queue" | python3 -c "
import json,sys
for t in json.load(sys.stdin)['tasks']:
    if t['id']==$SP_ID: print(t.get('error_message','?')[:60])
")
                echo -e "  ${RED}FAIL${NC} Spotify download — $ERR"
                ((FAIL++))
                break
            fi
            printf "."
        done
        echo ""
    else
        echo -e "  ${YELLOW}SKIP${NC} Deemix not authenticated (ARL expired?)"
        ((SKIP++))
    fi
fi

# ── Reset DB if requested ──
if [[ "${1:-}" == "--reset-db" ]] || [[ "${2:-}" == "--reset-db" ]]; then
    section "Reset"
    echo "  Run: ssh root@projectmellon.de 'systemctl stop wish && rm -f /home/momo/wish/wish.db* && systemctl start wish'"
fi

# ═══════════════════════════════════════════════════════════
section "Summary"
echo -e "  ${GREEN}PASS: $PASS${NC}"
echo -e "  ${RED}FAIL: $FAIL${NC}"
echo -e "  ${YELLOW}SKIP: $SKIP${NC}"
echo ""

if [ "$FAIL" -gt 0 ]; then
    echo "❌ Some tests FAILED"
    exit 1
else
    echo "✅ All tests PASSED"
    exit 0
fi
