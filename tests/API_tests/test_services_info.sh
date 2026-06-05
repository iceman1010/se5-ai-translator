#!/usr/bin/env bash
# Note: no `set -e` — run every probe even if one fails, so we see all responses.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTS_DIR="$(dirname "$SCRIPT_DIR")"
ENV_FILE="$TESTS_DIR/.env"

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
        echo "Copy tests/.env.example to tests/.env and fill in your credentials."
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

# Raw GET helper — prints body + [HTTP code]
api_get_raw() {
    local endpoint="$1"
    curl -s -w "\n[HTTP %{http_code}]\n" -X GET "https://api.opensubtitles.com/api/v1$endpoint" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Accept: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0"
}

# --- Tests -----------------------------------------------------------------

test_get_services_raw() {
    echo ""
    echo -e "${CYAN}=== Test 1: GET /ai/info/services (raw response) ===${NC}"

    local response
    response=$(api_get_raw "/ai/info/services")

    echo "$response" | sed 's/\[HTTP /\n[HTTP /'
    echo ""
}

test_get_services_schema() {
    echo ""
    echo -e "${CYAN}=== Test 2: Schema inspection ===${NC}"

    local response body
    response=$(api_get_raw "/ai/info/services")
    body=$(echo "$response" | sed 's/\[HTTP .*//')

    if ! echo "$body" | jq -e . >/dev/null 2>&1; then
        fail "Response is not valid JSON"
        return 0
    fi

    echo -e "${CYAN}-- top-level keys --${NC}"
    echo "$body" | jq 'keys'

    echo ""
    echo -e "${CYAN}-- data shape --${NC}"
    echo "$body" | jq '{
        data_keys: (.data | if type == "object" then keys else type end),
        data_type: (.data | type)
    }'

    echo ""
    echo -e "${CYAN}-- counts --${NC}"
    echo "$body" | jq '{
        translation_count: (.data.Translation | if type == "array" then length else null end),
        transcription_count: (.data.Transcription | if type == "array" then length else null end)
    }'
}

test_get_services_translation_summary() {
    echo ""
    echo -e "${CYAN}=== Test 3: Translation models summary ===${NC}"

    local response body
    response=$(api_get_raw "/ai/info/services")
    body=$(echo "$response" | sed 's/\[HTTP .*//')

    if ! echo "$body" | jq -e '.data.Translation' >/dev/null 2>&1; then
        fail "No .data.Translation array"
        return 0
    fi

    echo "$body" | jq -r '.data.Transscription[]? // .data.Translation[]? |
        "  - name=\(.name // "?")  display=\(.display_name // "?")  price=\(.pricing // "?")  reliability=\(.reliability // "?")  langs=\((.languages_supported // []) | length)"
    ' | sed 's/^/Scansion: /' >/dev/null 2>&1 || true

    echo "$body" | jq -r '
        .data.Translation[] |
        "  - \(.display_name // .name // "?")  [\(.name // "?")]  price=\(.pricing // "n/a")  reliability=\(.reliability // "n/a")  langs=\((.languages_supported // []) | length)"
    '

    echo ""
    echo -e "${CYAN}-- first translation model full sample --${NC}"
    echo "$body" | jq '.data.Transscription[0]? // .data.Translation[0]?'
}

test_get_services_transcription_summary() {
    echo ""
    echo -e "${CYAN}=== Test 4: Transcription models summary ===${NC}"

    local response body
    response=$(api_get_raw "/ai/info/services")
    body=$(echo "$response" | sed 's/\[HTTP .*//')

    if ! echo "$body" | jq -e '.data.Transcription' >/dev/null 2>&1; then
        fail "No .data.Transcription array"
        return 0
    fi

    echo "$body" | jq -r '
        .data.Transcription[] |
        "  - \(.display_name // .name // "?")  [\(.name // "?")]  price=\(.pricing // "n/a")  reliability=\(.reliability // "n/a")  langs=\((.languages_supported // []) | length)"
    '

    echo ""
    echo -e "${CYAN}-- first transcription model full sample --${NC}"
    echo "$body" | jq '.data.Transcription[0]'
}

test_get_services_fields_present() {
    echo ""
    echo -e "${CYAN}=== Test 5: Per-model field presence (translation) ===${NC}"

    local response body
    response=$(api_get_raw "/ai/info/services")
    body=$(echo "$response" | sed 's/\[HTTP .*//')

    echo "$body" | jq -r '
        .data.Transscription[]? // .data.Translation[]? |
        "  \(.name // "?"):\n" +
        "    has name:                \((has("name") | tostring))\n" +
        "    has display_name:        \((has("display_name") | tostring))\n" +
        "    has description:         \((has("description") | tostring))\n" +
        "    has pricing:             \((has("pricing") | tostring))\n" +
        "    has reliability:         \((has("reliability") | tostring))\n" +
        "    has price (numeric):     \((has("price") | tostring))\n" +
        "    has languages_supported: \((has("languages_supported") | tostring))\n" +
        "    extra keys:              \([keys[] | select(. != "name" and . != "display_name" and . != "description" and . != "pricing" and . != "reliability" and . != "price" and . != "languages_supported")])
        "
    ' | head -120
}

test_get_services_no_auth() {
    echo ""
    echo -e "${CYAN}=== Test 6: GET /ai/info/services without auth token ===${NC}"

    local response
    response=$(curl -s -w "\n[HTTP %{http_code}]\n" -X GET "https://api.opensubtitles.com/api/v1/ai/info/services" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Accept: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0")

    echo "$response" | sed 's/\[HTTP /\n[HTTP /'
    echo ""
    ok "No-auth probe completed"
}

test_get_services_wrong_token() {
    echo ""
    echo -e "${CYAN}=== Test 7: GET /ai/info/services with invalid token ===${NC}"

    local response
    response=$(curl -s -w "\n[HTTP %{http_code}]\n" -X GET "https://api.opensubtitles.com/api/v1/ai/info/services" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Authorization: Bearer INVALID_TOKEN_$(date +%s)" \
        -H "Accept: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0")

    echo "$response" | sed 's/\[HTTP /\n[HTTP /'
    echo ""
    ok "Invalid token probe completed"
}

# --- Main ------------------------------------------------------------------

echo "========================================"
echo "  API Test: GET /ai/info/services"
echo "========================================"

check_deps
load_env

info "Logging in..."
TOKEN=$(fetch_auth_token)
ok "Authenticated"

test_get_services_raw
test_get_services_schema
test_get_services_translation_summary
test_get_services_transcription_summary
test_get_services_fields_present
test_get_services_no_auth
test_get_services_wrong_token

echo ""
echo -e "${GREEN}All services-info tests completed.${NC}"
