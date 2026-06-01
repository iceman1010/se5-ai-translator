#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTS_DIR="$(dirname "$SCRIPT_DIR")"
ENV_FILE="$TESTS_DIR/.env"
EXAMPLE_SRT="$TESTS_DIR/example_srt/test_archer_2min.srt"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${YELLOW}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }

check_deps() {
    local missing=()
    for cmd in curl jq; do
        if ! command -v "$cmd" &>/dev/null; then
            missing+=("$cmd")
        fi
    done
    if [[ ${#missing[@]} -gt 0 ]]; then
        fail "Missing required tools: ${missing[*]}"
        exit 1
    fi
    ok "Dependencies satisfied"
}

load_env() {
    if [[ ! -f "$ENV_FILE" ]]; then
        fail "No .env file at $ENV_FILE"
        exit 1
    fi
    set -a
    source "$ENV_FILE"
    set +a

    local missing=()
    [[ -z "${OS_API_KEY:-}" ]] && missing+=("OS_API_KEY")
    [[ -z "${OS_USERNAME:-}" ]] && missing+=("OS_USERNAME")
    [[ -z "${OS_PASSWORD:-}" ]] && missing+=("OS_PASSWORD")

    if [[ ${#missing[@]} -gt 0 ]]; then
        fail "Missing in .env: ${missing[*]}"
        exit 1
    fi
    ok "Loaded credentials from .env"
}

fetch_auth_token() {
    local body http_code
    body=$(curl -s -w "\n%{http_code}" -X POST "https://api.opensubtitles.com/api/v1/login" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Accept: application/json" \
        -H "Content-Type: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0" \
        -d "{\"username\":\"$OS_USERNAME\",\"password\":\"$OS_PASSWORD\"}")

    http_code=$(echo "$body" | tail -1)
    body=$(echo "$body" | sed '$d')

    if [[ "$http_code" != "200" ]]; then
        fail "Login failed (HTTP $http_code)"
        echo "$body" | jq . >&2
        exit 1
    fi

    local token
    token=$(echo "$body" | jq -r '.token // empty')

    if [[ -z "$token" ]]; then
        fail "No token in login response"
        exit 1
    fi

    echo "$token"
}

api_post() {
    local endpoint="$1"
    shift
    curl -s -X POST "https://api.opensubtitles.com/api/v1$endpoint" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Accept: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0" \
        "$@"
}

test_detect_language() {
    echo ""
    echo -e "${CYAN}=== Test: Detect Language from SRT file ===${NC}"

    if [[ ! -f "$EXAMPLE_SRT" ]]; then
        fail "Example SRT not found: $EXAMPLE_SRT"
        return 1
    fi

    info "Submitting file for language detection..."
    local resp
    resp=$(api_post "/ai/detect_language" \
        -F "file=@$EXAMPLE_SRT")

    echo ""
    info "Raw response:"
    echo "$resp" | jq .

    local lang
    lang=$(echo "$resp" | jq '.data.language // .language')

    if [[ "$lang" == "null" ]]; then
        fail "No language detected"
        return 1
    fi

    local iso_code w3c_code lang_name
    iso_code=$(echo "$lang" | jq -r '.ISO_639_1 // "N/A"')
    w3c_code=$(echo "$lang" | jq -r '.W3C // "N/A"')
    lang_name=$(echo "$lang" | jq -r '.name // "N/A"')
    ok "Detected: $lang_name (ISO_639_1=$iso_code, W3C=$w3c_code)"
}

test_detect_language_code_formats() {
    echo ""
    echo -e "${CYAN}=== Test: Detected language code format vs engine language codes ===${NC}"

    info "Fetching engine language lists..."

    local engines_resp
    engines_resp=$(api_post "/ai/info/translation_apis")
    local engines
    engines=$(echo "$engines_resp" | jq -r '.data[]')

    local langs_resp
    langs_resp=$(api_post "/ai/info/translation_languages" "{}")

    echo ""
    info "Language code format examples per engine:"
    while IFS= read -r engine; do
        echo "  $engine:"
        echo "$langs_resp" | jq -r --arg e "$engine" '
            .data[$e][:5][] | "    \(.language_name): code=\(.language_code)"
        '
    done <<< "$engines"

    echo ""
    info "Detection returns codes in this format:"
    echo "    ISO_639_1: two-letter code (e.g. 'en', 'fr', 'zh')"
    echo "    W3C: may include region (e.g. 'en-US', 'zh-CN')"
    echo "    Engine codes vary: some use 'zh', others 'zh-CN'"
}

# --- Main ---

echo "========================================"
echo "  API Test: Language Detection"
echo "========================================"

check_deps
load_env

info "Logging in..."
TOKEN=$(fetch_auth_token)
ok "Authenticated"

test_detect_language
test_detect_language_code_formats

echo ""
echo -e "${GREEN}All language detection tests completed.${NC}"
