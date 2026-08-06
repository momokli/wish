#!/bin/bash
# Check current state of the last submission
# Run ON the wish server
echo "=== DB ==="
sqlite3 /home/momo/wish/wish.db "SELECT id, status, filename, substr(error_message,1,50) FROM submissions ORDER BY id DESC LIMIT 5;"

echo ""
echo "=== Pipeline summary (last 5 min) ==="
journalctl -u wish.service --no-pager -n 200 --since "5 min ago" 2>&1 | grep -E "\[[0-9]+\] L[123]|ready \[|REJECT|FAIL|deemix gave|enqueued|scanning" | tail -20
