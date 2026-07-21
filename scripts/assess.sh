#!/bin/bash
# wish — quickly assess the current state of the project
# Run before doing anything else.
echo "═══ Git ═══"
echo "  branch: $(git branch --show-current)"
DIRTY=$(git status --short | wc -l | tr -d ' ')
echo "  dirty:  $DIRTY files"
echo "  last:   $(git log --oneline -1 2>/dev/null || echo 'no commits')"
if [ "$DIRTY" -gt 10 ]; then
  echo "  ⚠ $DIRTY dirty files — commit before proceeding"
fi
echo ""
echo "═══ Build ═══"
cargo build 2>&1 | tail -2 | sed 's/^/  /'
echo ""
echo "═══ Tests ═══"
cargo test 2>&1 | grep "test result:" | sed 's/^/  /'
echo ""
echo "═══ Health ═══"
HEALTH=$(curl -sf --max-time 3 https://wish.zukkafabrik.de/health 2>/dev/null || echo 'unreachable')
echo "  server: $HEALTH"
echo ""
echo "═══ Deploy ═══"
echo "  rsync src/ Cargo.toml → root@projectmellon.de:/home/momo/wish/"
echo "  ssh → cargo build --release → systemctl restart wish"
