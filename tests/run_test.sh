#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_DIR="$(dirname "$SCRIPT_DIR")"
EXAMPLE_SRT="$SCRIPT_DIR/example_srt/test_archer_2min.srt"
ENV_FILE="$SCRIPT_DIR/.env"

DEV_MODE=false
TEST_FILTER="all"
for arg in "$@"; do
    case "$arg" in
        --dev) DEV_MODE=true ;;
        contract|gui|cancel|all) TEST_FILTER="$arg" ;;
    esac
done

if $DEV_MODE; then
    BINARY="$PLUGIN_DIR/target/debug/se-ai-translator"
else
    BINARY="$PLUGIN_DIR/target/release/se-ai-translator"
fi

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${YELLOW}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }

load_env() {
    if [[ -f "$ENV_FILE" ]]; then
        set -a
        source "$ENV_FILE"
        set +a
        ok "Loaded credentials from .env"
    else
        info "No .env file found at $ENV_FILE"
        info "Copy .env.example to .env and fill in your credentials:"
        info "  cp $SCRIPT_DIR/.env.example $ENV_FILE"
        return 1
    fi
}

require_env() {
    local missing=()
    [[ -z "${OS_API_KEY:-}" ]] && missing+=("OS_API_KEY")
    [[ -z "${OS_USERNAME:-}" ]] && missing+=("OS_USERNAME")
    [[ -z "${OS_PASSWORD:-}" ]] && missing+=("OS_PASSWORD")

    if [[ ${#missing[@]} -gt 0 ]]; then
        fail "Missing in .env: ${missing[*]}"
        return 1
    fi
}

check_binary() {
    if [[ ! -x "$BINARY" ]]; then
        if $DEV_MODE; then
            info "Binary not found. Building debug..."
            (cd "$PLUGIN_DIR" && cargo build)
        else
            info "Binary not found. Building release..."
            (cd "$PLUGIN_DIR" && cargo build --release)
        fi
    fi
    ok "Binary ready: $BINARY"
}

check_example() {
    if [[ ! -f "$EXAMPLE_SRT" ]]; then
        fail "Example subtitle not found: $EXAMPLE_SRT"
        exit 1
    fi
    ok "Example subtitle: $EXAMPLE_SRT"
}

build_request() {
    local test_dir="$1"
    local settings_json="$2"
    local srt_content
    srt_content="$(cat "$EXAMPLE_SRT")"

    local srt_escaped
    srt_escaped="$(echo "$srt_content" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')"

    cat > "$test_dir/request.json" << EOF
{
  "apiVersion": 1,
  "requestType": "run",
  "responseFilePath": "$test_dir/response.json",
  "tempDirectory": "$test_dir",
  "subtitle": {
    "format": "SubRip",
    "fileName": "test_archer_2min.srt",
    "native": $srt_escaped,
    "subRip": $srt_escaped
  },
  "selectedIndices": [],
  "videoFileName": null,
  "frameRate": 23.976,
  "videoDurationSeconds": 120.0,
  "videoWidth": 1920,
  "videoHeight": 1080,
  "uiLanguage": "English",
  "theme": "Dark",
  "seVersion": "5.0.0",
  "settings": $settings_json,
  "settingsVersion": null
}
EOF
}

build_settings_no_auth() {
    echo "null"
}

fetch_auth_token() {
    local response http_code
    response=$(curl -s -w "\n%{http_code}" -X POST "https://api.opensubtitles.com/api/v1/login" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Accept: application/json" \
        -H "Content-Type: application/json" \
        -H "User-Agent: aios v1" \
        -d "{\"username\":\"$OS_USERNAME\",\"password\":\"$OS_PASSWORD\"}" 2>/dev/null)

    http_code=$(echo "$response" | tail -1)
    local body=$(echo "$response" | sed '$d')

    if [[ "$http_code" != "200" ]]; then
        fail "Login failed (HTTP $http_code)."
        echo "$body" | python3 -m json.tool 2>/dev/null >&2 || echo "$body" >&2
        return 1
    fi

    local token
    token=$(echo "$body" | python3 -c "import json,sys; print(json.load(sys.stdin).get('token',''))" 2>/dev/null)

    if [[ -z "$token" ]]; then
        fail "No token in login response."
        echo "$body" | python3 -m json.tool 2>/dev/null >&2 || echo "$body" >&2
        return 1
    fi

    echo "$token"
}

build_settings_with_auth() {
    local token="$1"
    python3 -c "
import json
print(json.dumps({
    'apiKey': '$OS_API_KEY',
    'authToken': '$token',
    'lastSourceLang': 'auto',
    'lastTargetLang': None,
    'lastEngine': None
}))
"
}

test_contract_json() {
    local test_dir
    test_dir="$(mktemp -d /tmp/se-test-XXXXXX)"
    build_request "$test_dir" "$(build_settings_no_auth)"

    info "Validating request.json..."
    if python3 -m json.tool "$test_dir/request.json" > /dev/null 2>&1; then
        ok "request.json is valid JSON"
    else
        fail "request.json is invalid JSON"
        cat "$test_dir/request.json"
        return 1
    fi

    info "Checking subtitle content in request..."
    local line_count
    line_count=$(python3 -c "import json; d=json.load(open('$test_dir/request.json')); print(d['subtitle']['subRip'].count('\n'))")
    if [[ "$line_count" -gt 10 ]]; then
        ok "Subtitle content present ($line_count lines)"
    else
        fail "Subtitle content seems empty or too short"
        return 1
    fi

    rm -rf "$test_dir"
}

test_gui_launch() {
    local test_dir
    test_dir="$(mktemp -d /tmp/se-test-XXXXXX)"
    local settings_json="null"

    if load_env && require_env; then
        info "Logging in to get auth token..."
        if token=$(fetch_auth_token); then
            ok "Got auth token"
            settings_json=$(build_settings_with_auth "$token")
            info "Plugin will skip setup — go straight to translation dialog."
        else
            info "Could not get token. Plugin will show login screen."
        fi
    else
        info "No .env credentials. Plugin will show the full setup screen."
    fi

    build_request "$test_dir" "$settings_json"

    info "Launching plugin GUI..."
    info "  Request: $test_dir/request.json"
    info "  Response will be written to: $test_dir/response.json"
    info ""

    "$BINARY" "$test_dir/request.json"
    local exit_code=$?

    if [[ $exit_code -ne 0 ]]; then
        fail "Plugin exited with code $exit_code"
        return 1
    fi
    ok "Plugin exited cleanly (code 0)"

    if [[ -f "$test_dir/response.json" ]]; then
        ok "response.json written"

        local status
        status=$(python3 -c "import json; print(json.load(open('$test_dir/response.json'))['status'])")
        info "Response status: $status"

        if [[ "$status" == "ok" ]]; then
            local has_subtitle
            has_subtitle=$(python3 -c "import json; d=json.load(open('$test_dir/response.json')); print('yes' if d.get('subtitle',{}).get('native','') else 'no')")
            if [[ "$has_subtitle" == "yes" ]]; then
                ok "Translated subtitle present in response"
            else
                fail "No subtitle content in response"
            fi
        elif [[ "$status" == "cancelled" ]]; then
            info "User cancelled — no changes applied"
        elif [[ "$status" == "error" ]]; then
            local msg
            msg=$(python3 -c "import json; print(json.load(open('$test_dir/response.json')).get('message',''))")
            fail "Plugin error: $msg"
        fi

        echo ""
        info "--- response.json ---"
        python3 -m json.tool "$test_dir/response.json" 2>/dev/null || cat "$test_dir/response.json"
    else
        fail "No response.json written"
    fi

    echo ""
    info "Test artifacts left in: $test_dir"
    info "Clean up with: rm -rf $test_dir"
}

test_cancel() {
    local test_dir
    test_dir="$(mktemp -d /tmp/se-test-XXXXXX)"
    build_request "$test_dir" "$(build_settings_no_auth)"

    info "Launching plugin for CANCEL test..."
    info "Close the window without translating (or click Cancel if translating)."
    info ""

    "$BINARY" "$test_dir/request.json"

    if [[ -f "$test_dir/response.json" ]]; then
        local status
        status=$(python3 -c "import json; print(json.load(open('$test_dir/response.json'))['status'])")
        if [[ "$status" == "cancelled" ]]; then
            ok "Cancel test passed: status=cancelled"
        else
            info "Response status: $status (expected cancelled if you closed without translating)"
        fi
    else
        fail "No response.json written on cancel"
    fi

    rm -rf "$test_dir"
}

# --- Main ---

echo "========================================"
echo "  SE5 AI Translator Plugin Test Runner"
echo "========================================"
echo ""

check_binary
check_example
echo ""

case "$TEST_FILTER" in
    contract)
        info "Running: JSON contract validation"
        test_contract_json
        ;;
    gui)
        info "Running: GUI test (full interactive)"
        test_gui_launch
        ;;
    cancel)
        info "Running: Cancel test"
        test_cancel
        ;;
    all)
        info "Running: contract + gui"
        test_contract_json
        echo ""
        test_gui_launch
        ;;
    *)
        echo "Usage: $0 [--dev] [contract|gui|cancel|all]"
        echo ""
        echo "  --dev    Use debug build (target/debug/) instead of release"
        echo "  contract Validate request.json generation (no GUI, no .env needed)"
        echo "  gui      Launch plugin with test data (interactive, uses .env if present)"
        echo "  cancel   Test cancel/close behaviour (interactive)"
        echo "  all      Run contract then gui (default)"
        exit 1
        ;;
esac
