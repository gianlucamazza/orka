#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
demo_dir="$repo_root/demo"
build_dir="$demo_dir/.build"
readme_file="$repo_root/README.md"
tunnel_log="$build_dir/demo-tunnel.log"

direct_server_url_default="${ORKA_SERVER_URL:-http://orka-odroid:18080}"
direct_adapter_url_default="${ORKA_ADAPTER_URL:-http://orka-odroid:18081}"

demo_server_url="${ORKA_DEMO_SERVER_URL:-$direct_server_url_default}"
demo_adapter_url="${ORKA_DEMO_ADAPTER_URL:-$direct_adapter_url_default}"
demo_use_ssh_tunnel="${ORKA_DEMO_USE_SSH_TUNNEL:-1}"
demo_ssh_host="${ORKA_DEMO_SSH_HOST:-odroid}"
demo_remote_server_port="${ORKA_DEMO_REMOTE_SERVER_PORT:-18080}"
demo_remote_adapter_port="${ORKA_DEMO_REMOTE_ADAPTER_PORT:-18081}"
gif_fps="${ORKA_DEMO_GIF_FPS:-12}"
gif_width="${ORKA_DEMO_GIF_WIDTH:-960}"
webm_crf="${ORKA_DEMO_WEBM_CRF:-32}"
tunnel_ready=0
tunnel_pid=""

scenarios=(chat dashboard status send)

trim_trailing_slash() {
    local value="$1"
    while [[ "$value" == */ ]]; do
        value="${value%/}"
    done
    printf '%s\n' "$value"
}

demo_server_url="$(trim_trailing_slash "$demo_server_url")"
demo_adapter_url="$(trim_trailing_slash "$demo_adapter_url")"

die() {
    echo "error: $*" >&2
    exit 1
}

log() {
    printf '[demo] %s\n' "$*"
}

usage() {
    cat <<EOF
Usage: ./scripts/demo.sh <command> [scenario]

Commands:
  list                  List available demo scenarios.
  check                 Validate prerequisites and live backend health.
  record [scenario]     Record master .mp4 files with VHS.
  render [scenario]     Derive .mp4/.webm/.gif assets from master recordings.
  verify [scenario]     Verify generated assets and README references.
  build [scenario]      Run record + render + verify.
  clean                 Remove staged master recordings under demo/.build.

Scenario:
  chat | dashboard | status | send | all (default)

Environment:
  ORKA_DEMO_USE_SSH_TUNNEL  Open a dedicated SSH tunnel first (default: ${demo_use_ssh_tunnel})
  ORKA_DEMO_SSH_HOST        SSH host used for the tunnel (default: ${demo_ssh_host})
  ORKA_DEMO_REMOTE_SERVER_PORT   Remote Orka API port (default: ${demo_remote_server_port})
  ORKA_DEMO_REMOTE_ADAPTER_PORT  Remote adapter port (default: ${demo_remote_adapter_port})
  ORKA_DEMO_SERVER_URL      Direct demo server URL when tunnel is disabled
  ORKA_DEMO_ADAPTER_URL     Direct demo adapter URL when tunnel is disabled
  ORKA_DEMO_API_KEY      API key for protected demo environments
  ORKA_DEMO_ORKA_BIN     Explicit path to the orka CLI binary
  ORKA_DEMO_GIF_FPS      GIF frame rate (default: ${gif_fps})
  ORKA_DEMO_GIF_WIDTH    GIF width in pixels (default: ${gif_width})
  ORKA_DEMO_WEBM_CRF     libvpx-vp9 CRF for WebM output (default: ${webm_crf})
EOF
}

require_command() {
    local cmd="$1"
    command -v "$cmd" >/dev/null 2>&1 || die "required command not found: $cmd"
}

resolve_demo_api_key() {
    printf '%s\n' "${ORKA_DEMO_API_KEY:-${ORKA_ODROID_API_KEY:-${ORKA_API_KEY:-}}}"
}

curl_demo() {
    local url="$1"
    local timeout_secs="${2:-5}"
    local api_key

    api_key="$(resolve_demo_api_key)"
    if [[ -n "$api_key" ]]; then
        curl --fail --silent --show-error --max-time "$timeout_secs" \
            -H "Authorization: Bearer $api_key" \
            "$url"
    else
        curl --fail --silent --show-error --max-time "$timeout_secs" "$url"
    fi
}

find_free_port() {
    python - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

cleanup_tunnel() {
    if [[ -n "$tunnel_pid" ]] && kill -0 "$tunnel_pid" 2>/dev/null; then
        kill "$tunnel_pid" 2>/dev/null || true
        wait "$tunnel_pid" 2>/dev/null || true
    fi
}

setup_tunnel_if_needed() {
    if [[ "$tunnel_ready" -eq 1 ]]; then
        return
    fi

    if [[ "$demo_use_ssh_tunnel" != "1" ]]; then
        tunnel_ready=1
        return
    fi

    require_command ssh
    require_command python
    mkdir -p "$build_dir"

    local local_server_port
    local local_adapter_port
    local_server_port="$(find_free_port)"
    local_adapter_port="$(find_free_port)"

    : > "$tunnel_log"
    ssh \
        -o BatchMode=yes \
        -o ControlMaster=no \
        -o ControlPath=none \
        -o ExitOnForwardFailure=yes \
        -o ClearAllForwardings=yes \
        -N \
        -L "${local_server_port}:127.0.0.1:${demo_remote_server_port}" \
        -L "${local_adapter_port}:127.0.0.1:${demo_remote_adapter_port}" \
        "$demo_ssh_host" \
        </dev/null >"$tunnel_log" 2>&1 &
    tunnel_pid=$!
    trap cleanup_tunnel EXIT

    local attempts=20
    while (( attempts > 0 )); do
        if ! kill -0 "$tunnel_pid" 2>/dev/null; then
            sed -n '1,120p' "$tunnel_log" >&2 || true
            die "ssh tunnel failed to start for host '$demo_ssh_host'"
        fi

        if curl_demo "http://127.0.0.1:${local_server_port}/health" 1 >/dev/null 2>&1 \
            && curl_demo "http://127.0.0.1:${local_adapter_port}/api/v1/health" 1 >/dev/null 2>&1
        then
            demo_server_url="http://127.0.0.1:${local_server_port}"
            demo_adapter_url="http://127.0.0.1:${local_adapter_port}"
            tunnel_ready=1
            log "ssh tunnel ready via ${demo_ssh_host}: server=${demo_server_url} adapter=${demo_adapter_url}"
            return
        fi

        sleep 0.5
        attempts=$((attempts - 1))
    done

    sed -n '1,120p' "$tunnel_log" >&2 || true
    die "ssh tunnel started but backend did not answer on forwarded ports"
}

resolve_orka_bin() {
    if [[ -n "${ORKA_DEMO_ORKA_BIN:-}" ]]; then
        [[ -x "${ORKA_DEMO_ORKA_BIN}" ]] || die "ORKA_DEMO_ORKA_BIN is not executable: ${ORKA_DEMO_ORKA_BIN}"
        printf '%s\n' "${ORKA_DEMO_ORKA_BIN}"
        return
    fi

    local candidate
    for candidate in \
        "$repo_root/target/release/orka" \
        "$repo_root/target/debug/orka"
    do
        if [[ -x "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return
        fi
    done

    if candidate="$(command -v orka 2>/dev/null)"; then
        printf '%s\n' "$candidate"
        return
    fi

    die "could not find an orka CLI binary. Build one with 'cargo build -p orka-cli' or set ORKA_DEMO_ORKA_BIN."
}

scenario_tape() {
    case "$1" in
        chat) printf '%s\n' "$demo_dir/chat.tape" ;;
        dashboard) printf '%s\n' "$demo_dir/dashboard.tape" ;;
        status) printf '%s\n' "$demo_dir/status.tape" ;;
        send) printf '%s\n' "$demo_dir/send.tape" ;;
        *) die "unknown scenario: $1" ;;
    esac
}

scenario_asset_stem() {
    case "$1" in
        chat) printf 'orka-chat\n' ;;
        dashboard) printf 'orka-dashboard\n' ;;
        status) printf 'orka-status\n' ;;
        send) printf 'orka-send\n' ;;
        *) die "unknown scenario: $1" ;;
    esac
}

scenario_asset_gif() {
    printf '%s/%s.gif\n' "$demo_dir" "$(scenario_asset_stem "$1")"
}

scenario_asset_mp4() {
    printf '%s/%s.mp4\n' "$demo_dir" "$(scenario_asset_stem "$1")"
}

scenario_asset_webm() {
    printf '%s/%s.webm\n' "$demo_dir" "$(scenario_asset_stem "$1")"
}

scenario_master_mp4() {
    printf '%s/%s.master.mp4\n' "$build_dir" "$(scenario_asset_stem "$1")"
}

readme_reference() {
    printf 'demo/%s.gif\n' "$(scenario_asset_stem "$1")"
}

resolve_scenarios() {
    local requested="${1:-all}"
    if [[ "$requested" == "all" ]]; then
        printf '%s\n' "${scenarios[@]}"
        return
    fi

    local scenario
    for scenario in "${scenarios[@]}"; do
        if [[ "$scenario" == "$requested" ]]; then
            printf '%s\n' "$scenario"
            return
        fi
    done

    die "unknown scenario: $requested"
}

check_backend() {
    require_command curl
    setup_tunnel_if_needed

    local server_health
    server_health="$(curl_demo "$demo_server_url/health")"
    [[ "${server_health//[[:space:]]/}" == *'"status":"ok"'* ]] \
        || die "server health check failed at $demo_server_url/health: $server_health"

    local server_ready
    server_ready="$(curl_demo "$demo_server_url/health/ready")"
    [[ "${server_ready//[[:space:]]/}" == *'"status":"ready"'* ]] \
        || die "server readiness check failed at $demo_server_url/health/ready: $server_ready"

    local adapter_health
    adapter_health="$(curl_demo "$demo_adapter_url/api/v1/health")"
    [[ "${adapter_health//[[:space:]]/}" == *'"status":"ok"'* ]] \
        || die "adapter health check failed at $demo_adapter_url/api/v1/health: $adapter_health"

    log "backend ok: server=$demo_server_url adapter=$demo_adapter_url"
}

check_recorder() {
    require_command vhs

    local orka_bin
    orka_bin="$(resolve_orka_bin)"
    [[ -x "$orka_bin" ]] || die "resolved orka binary is not executable: $orka_bin"

    mkdir -p "$build_dir"
    vhs validate "$demo_dir"/*.tape >/dev/null

    log "recorder ok: orka=$orka_bin vhs=$(command -v vhs)"
}

check_renderer() {
    require_command ffmpeg
    require_command ffprobe
    mkdir -p "$build_dir"
    log "renderer ok: ffmpeg=$(command -v ffmpeg) ffprobe=$(command -v ffprobe)"
}

check_verifier() {
    require_command ffprobe
    log "verifier ok: ffprobe=$(command -v ffprobe)"
}

record_one() {
    local scenario="$1"
    local tape
    local master
    local orka_bin
    local shim_dir
    local demo_api_key

    tape="$(scenario_tape "$scenario")"
    master="$(scenario_master_mp4 "$scenario")"
    orka_bin="$(resolve_orka_bin)"
    shim_dir="$(mktemp -d)"
    demo_api_key="$(resolve_demo_api_key)"
    mkdir -p "$build_dir"
    rm -f "$master"

    trap 'rm -rf "$shim_dir"' RETURN
    ln -sf "$orka_bin" "$shim_dir/orka"

    log "recording $scenario -> $master"
    (
        export PATH="$shim_dir:$PATH"
        export ORKA_SERVER_URL="$demo_server_url"
        export ORKA_ADAPTER_URL="$demo_adapter_url"
        if [[ -n "$demo_api_key" ]]; then
            export ORKA_API_KEY="$demo_api_key"
        else
            unset ORKA_API_KEY
        fi
        vhs "$tape" -o "$master"
    )
}

render_one() {
    local scenario="$1"
    local master
    local mp4
    local webm
    local gif

    master="$(scenario_master_mp4 "$scenario")"
    mp4="$(scenario_asset_mp4 "$scenario")"
    webm="$(scenario_asset_webm "$scenario")"
    gif="$(scenario_asset_gif "$scenario")"

    [[ -s "$master" ]] || die "missing master recording for $scenario: $master"

    log "rendering $scenario assets"
    ffmpeg -y -loglevel error -i "$master" -c copy -movflags +faststart "$mp4"
    ffmpeg -y -loglevel error -i "$master" -an -c:v libvpx-vp9 -b:v 0 -crf "$webm_crf" "$webm"
    ffmpeg -y -loglevel error -i "$master" \
        -vf "fps=${gif_fps},scale=${gif_width}:-1:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=sierra2_4a" \
        "$gif"
}

verify_duration() {
    local file="$1"
    local duration
    duration="$(ffprobe -v error -show_entries format=duration -of default=nw=1:nk=1 "$file")"
    [[ -n "$duration" ]] || die "could not read duration for $file"

    awk -v value="$duration" 'BEGIN { exit !(value >= 1 && value <= 90) }' \
        || die "unexpected duration for $file: ${duration}s"
}

verify_one() {
    local scenario="$1"
    local mp4
    local webm
    local gif
    local reference

    mp4="$(scenario_asset_mp4 "$scenario")"
    webm="$(scenario_asset_webm "$scenario")"
    gif="$(scenario_asset_gif "$scenario")"
    reference="$(readme_reference "$scenario")"

    [[ -s "$mp4" ]] || die "missing mp4 asset for $scenario: $mp4"
    [[ -s "$webm" ]] || die "missing webm asset for $scenario: $webm"
    [[ -s "$gif" ]] || die "missing gif asset for $scenario: $gif"
    verify_duration "$mp4"
    grep -Fq "$reference" "$readme_file" || die "README is missing $reference"
}

cmd="${1:-help}"
scenario="${2:-all}"
selected_scenarios=()

if [[ "$cmd" =~ ^(record|render|verify|build)$ ]]; then
    mapfile -t selected_scenarios < <(resolve_scenarios "$scenario")
fi

case "$cmd" in
    list)
        printf '%s\n' "${scenarios[@]}"
        ;;
    check)
        check_backend
        check_recorder
        check_renderer
        ;;
    record)
        check_backend
        check_recorder
        for item in "${selected_scenarios[@]}"; do
            record_one "$item"
        done
        ;;
    render)
        check_renderer
        for item in "${selected_scenarios[@]}"; do
            render_one "$item"
        done
        ;;
    verify)
        check_verifier
        for item in "${selected_scenarios[@]}"; do
            verify_one "$item"
        done
        log "assets verified"
        ;;
    build)
        check_backend
        check_recorder
        check_renderer
        for item in "${selected_scenarios[@]}"; do
            record_one "$item"
            render_one "$item"
            verify_one "$item"
        done
        log "build complete"
        ;;
    clean)
        rm -rf "$build_dir"
        log "removed $build_dir"
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        usage >&2
        die "unknown command: $cmd"
        ;;
esac
