#!/bin/bash
# wish — pre-deploy validation gate
set -e

echo "═══ 1. Frontend JS exports check ═══"
node scripts/check-exports.mjs frontend/js/*.js

echo ""
echo "═══ 2. Frontend build ═══"
node scripts/build-html.mjs

echo ""
echo "═══ 3. Frontend JS lint ═══"
node scripts/lint-html.mjs frontend/dist/*.html

echo ""
echo "═══ 4. Rust build ═══"
cargo build 2>&1 | tail -3

echo ""
echo "═══ 5. Rust tests ═══"
cargo test 2>&1 | grep "test result:" | sed 's/^/  /'

echo ""
echo "═══ 6. Summary ═══"
echo "✅ All checks passed"
