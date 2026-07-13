#!/usr/bin/env bash
# Gout 性能基准测试 — 对标 rathole / frp
#
# 用法:
#   1. 启动本地服务:  iperf3 -s -p 80   (TCP)  或 nginx (HTTP)
#   2. 运行测试:       bash scripts/bench.sh tcp
#
# 子命令:
#   tcp        TCP 吞吐 (iperf3, 4 并行流, 30s)
#   udp        UDP 吞吐 (iperf3, 30s, 1000M)
#   http       HTTP 吞吐 (vegeta, rate=0, max-workers=48, 30s)
#   latency    HTTP 延迟 (vegeta, QPS 1/1000/2000/3000/4000)

set -euo pipefail

SERVER="${SERVER:-127.0.0.1:8080}"
DATA_PORT="${DATA_PORT:-8081}"
LOCAL_PORT="${LOCAL_PORT:-80}"
KEY="${GOUT_BENCH_KEY:-}"    # 提前设好 GOUT_BENCH_KEY 可跳过 login

cleanup() {
    [ -n "${TUNNEL_PORT:-}" ] && gout kill "$TUNNEL_PORT" 2>/dev/null && echo "[-] tunnel $TUNNEL_PORT killed"
}
trap cleanup EXIT

# 获取 API key（首次通过 gout login 或环境变量）
ensure_key() {
    if [ -n "$KEY" ]; then
        return
    fi
    # 尝试从 ~/.gout/config.toml 读取
    if [ -f "$HOME/.gout/config.toml" ]; then
        KEY=$(grep 'api_key' "$HOME/.gout/config.toml" 2>/dev/null | head -1 | sed 's/.*= *"\(.*\)"/\1/')
    fi
    if [ -z "$KEY" ]; then
        echo "[-] set GOUT_BENCH_KEY=<api-key> or run: gout login $SERVER <admin-key>"
        exit 1
    fi
}

# 通过 REST API 创建隧道，返回 public_port
create_tunnel() {
    local type=$1
    local resp
    resp=$(curl -sf -X POST "http://$SERVER/api/v1/tunnels" \
        -H "X-Api-Key: $KEY" \
        -H "Content-Type: application/json" \
        -d "{\"type\":\"$type\",\"local_port\":$LOCAL_PORT}")
    TUNNEL_PORT=$(echo "$resp" | python3 -c "import sys,json; print(json.load(sys.stdin)['data']['public_port'])")
    TUNNEL_TOKEN=$(echo "$resp" | python3 -c "import sys,json; print(json.load(sys.stdin)['data']['token'])")
    echo "[+] tunnel $type: 127.0.0.1:$LOCAL_PORT -> $SERVER:$TUNNEL_PORT (token: $TUNNEL_TOKEN)"
}

# 启动客户端隧道（前台；bench 结束后 kill 清理）
start_client() {
    local type=$1
    gout "$type" "$LOCAL_PORT" &
    GOUT_PID=$!
    sleep 2
}

cleanup_tunnel() {
    # 通过 REST API 删除（更可靠）
    curl -sf -X DELETE "http://$SERVER/api/v1/tunnels/$TUNNEL_TOKEN" \
        -H "X-Api-Key: $KEY" >/dev/null 2>&1 || true
    [ -n "${GOUT_PID:-}" ] && kill "$GOUT_PID" 2>/dev/null || true
}

# ─── TCP 吞吐 ────────────────────────────────────────────────

bench_tcp() {
    echo "=== TCP Throughput (iperf3) ==="
    echo "  server iperf3 -s -p $LOCAL_PORT (run in another terminal)"
    echo "  or: iperf3 -s -D -p $LOCAL_PORT"
    echo
    ensure_key
    create_tunnel tcp
    start_client tcp

    echo "[*] iperf3 -c 127.0.0.1 -p $TUNNEL_PORT -t 30 -P 4"
    iperf3 -c 127.0.0.1 -p "$TUNNEL_PORT" -t 30 -P 4
    cleanup_tunnel
}

# ─── UDP 吞吐 ────────────────────────────────────────────────

bench_udp() {
    echo "=== UDP Throughput (iperf3) ==="
    echo "  server iperf3 -s -p $LOCAL_PORT (run in another terminal)"
    echo
    ensure_key
    create_tunnel udp
    start_client udp

    echo "[*] iperf3 -c 127.0.0.1 -p $TUNNEL_PORT -t 30 -u -b 1000M"
    iperf3 -c 127.0.0.1 -p "$TUNNEL_PORT" -t 30 -u -b 1000M
    cleanup_tunnel
}

# ─── HTTP 吞吐 ───────────────────────────────────────────────

bench_http() {
    echo "=== HTTP Throughput (vegeta) ==="
    echo "  nginx listens on port $LOCAL_PORT"
    echo
    ensure_key
    create_tunnel http
    start_client http

    echo "[*] vegeta attack -rate 0 -duration 30s -max-workers 48"
    echo "GET http://127.0.0.1:$TUNNEL_PORT" | vegeta attack \
        -rate 0 -duration 30s -max-workers 48 | vegeta report
    cleanup_tunnel
}

# ─── HTTP 延迟 (QPS 阶梯) ───────────────────────────────────

bench_latency() {
    echo "=== HTTP Latency (vegeta QPS ladder) ==="
    echo "  nginx listens on port $LOCAL_PORT"
    echo
    ensure_key
    create_tunnel http
    start_client http

    for qps in 1 1000 2000 3000 4000; do
        echo "--- QPS=$qps ---"
        echo "GET http://127.0.0.1:$TUNNEL_PORT" | vegeta attack \
            -rate "$qps" -duration 15s -max-workers 48 | \
            vegeta report -type json 2>/dev/null | \
            python3 -c "
import sys,json
r=json.load(sys.stdin)
print(f'  latency p50={r[\"latencies\"][\"50th\"]/1e6:.3f}ms')
print(f'  latency p95={r[\"latencies\"][\"95th\"]/1e6:.3f}ms')
print(f'  latency p99={r[\"latencies\"][\"99th\"]/1e6:.3f}ms')
print(f'  success={r[\"success\"]*100:.1f}%')
print(f'  throughput={r[\"throughput\"]:.1f} req/s')
" 2>/dev/null || echo "  (install vegeta: go install github.com/tsenart/vegeta/v12@latest)"
    done
    cleanup_tunnel
}

# ─── 主入口 ──────────────────────────────────────────────────

cmd="${1:-all}"
case "$cmd" in
    tcp)     bench_tcp ;;
    udp)     bench_udp ;;
    http)    bench_http ;;
    latency) bench_latency ;;
    all)
        bench_tcp; echo
        bench_udp; echo
        bench_http; echo
        bench_latency
        ;;
    *) echo "usage: $0 {tcp|udp|http|latency|all}" >&2; exit 1 ;;
esac
