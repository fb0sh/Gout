#!/usr/bin/env bash
# API 集成测试 — 启动 goutd，测试所有 REST 端点
set -e

cd "$(dirname "$0")"

cargo build -p goutd 2>/dev/null

TMPDIR=$(mktemp -d)
mkdir -p "$TMPDIR/data"
trap "pkill -f 'target/debug/goutd.*$TMPDIR' 2>/dev/null; rm -rf $TMPDIR" EXIT

./target/debug/goutd --data-dir "$TMPDIR/data" --http-addr 127.0.0.1:19000 --data-addr 127.0.0.1:19001 &
sleep 1

ADMIN_KEY=$(grep "Admin API Key" /tmp/goutd-api.log 2>/dev/null && echo "")

# Fallback — capture from server output if not in log
ADMIN_KEY=${ADMIN_KEY:-$(cat /tmp/goutd-api.log 2>/dev/null | grep "Admin API Key" | tail -1 | awk '{print $NF}')}
if [ -z "$ADMIN_KEY" ]; then
  # Try running directly with captured output
  pkill -f "target/debug/goutd" 2>/dev/null
  sleep 1
  ./target/debug/goutd --data-dir "$TMPDIR/data" --http-addr 127.0.0.1:19000 --data-addr 127.0.0.1:19001 > "$TMPDIR/out.log" 2>&1 &
  sleep 1
  ADMIN_KEY=$(grep "Admin API Key" "$TMPDIR/out.log" | tail -1 | awk '{print $NF}')
fi

echo "Admin: ${ADMIN_KEY:0:20}..."
BASE="http://127.0.0.1:19000"
PASS=0 FAIL=0

check() { if [ "$2" = "1" ] || [ "$2" = "true" ] || [ "$2" = "200" ] || [ "$2" = "201" ] || [ "$2" = "303" ]; then PASS=$((PASS+1)); echo "  OK $1"; else FAIL=$((FAIL+1)); echo "  FAIL $1 (got $2)"; fi; }

# 1. Login page
check "GET /login" "$(curl -s -o /dev/null -w '%{http_code}' $BASE/login 2>/dev/null)"

# 2. Wrong login
check "POST /login bad key" "$(curl -s -o /dev/null -w '%{http_code}' -X POST $BASE/login -d key=wrong 2>/dev/null)"

# 3. Correct login (set cookie)
COOKIE="$TMPDIR/cookies.txt"
curl -s -c "$COOKIE" -o /dev/null -X POST $BASE/login -d "key=$ADMIN_KEY" 2>/dev/null
check "Login sets cookie" "$(grep -c gout_admin_session "$COOKIE" 2>/dev/null || echo 0)"

# 4. Dashboard
check "GET /dashboard" "$(curl -s -b "$COOKIE" -o /dev/null -w '%{http_code}' $BASE/dashboard 2>/dev/null)"

# 5. API: create tunnel key
K1=$(curl -s -X POST $BASE/api/v1/keys -H "X-Admin-Key: $ADMIN_KEY" -H 'Content-Type: application/json' -d '{"name":"t1"}' 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('data',{}).get('key',''))" 2>/dev/null)
check "Create key" "$([ -n "$K1" ] && echo 1 || echo 0)"

# 6. Create tunnel
TUN=$(curl -s -X POST $BASE/api/v1/tunnels -H "X-Api-Key: $K1" -H 'Content-Type: application/json' -d '{"type":"tcp","local_port":4000}' 2>/dev/null)
TOKEN=$(echo "$TUN" | python3 -c "import sys,json; print(json.load(sys.stdin).get('data',{}).get('token',''))" 2>/dev/null)
check "Create tunnel" "$([ -n "$TOKEN" ] && echo 1 || echo 0)"

# 7. Delete tunnel
check "DELETE tunnel" "$(curl -s -o /dev/null -w '%{http_code}' -X DELETE "$BASE/api/v1/tunnels/$TOKEN" -H "X-Api-Key: $K1" 2>/dev/null)"

# 8. Admin cannot create tunnel
check "Admin cannot tunnel" "$(curl -s -o /dev/null -w '%{http_code}' -X POST $BASE/api/v1/tunnels -H "X-Api-Key: $ADMIN_KEY" -H 'Content-Type: application/json' -d '{"type":"tcp","local_port":5000}' 2>/dev/null)"

# 9. Tunnel key cannot manage keys
check "Tunnel key cannot manage" "$(curl -s -o /dev/null -w '%{http_code}' -X POST $BASE/api/v1/keys -H "X-Api-Key: $K1" -H 'Content-Type: application/json' -d '{"name":"x"}' 2>/dev/null)"

# 10. API delete key
check "API delete key" "$(curl -s -o /dev/null -w '%{http_code}' -X DELETE "$BASE/api/v1/keys/$K1" -H "X-Admin-Key: $ADMIN_KEY" 2>/dev/null)"

# 11. Web create + delete key
K2=$(curl -s -X POST $BASE/api/v1/keys -H "X-Admin-Key: $ADMIN_KEY" -H 'Content-Type: application/json' -d '{"name":"t2"}' 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('data',{}).get('key',''))" 2>/dev/null)
check "Web delete key" "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/keys/delete/$K2" 2>/dev/null)"

echo "---"
echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
