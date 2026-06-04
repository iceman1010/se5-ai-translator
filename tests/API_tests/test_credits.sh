#!/usr/bin/env bash
# Note: we deliberately do NOT use `set -e` here — we want every test
# to run even if one fails, so we can see all endpoint responses.
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

# Raw POST helper — prints full response (HTTP code + body)
api_post_raw() {
    local endpoint="$1"
    local content_type="$2"
    local extra_args=("${@:3}")

    curl -s -w "\n[HTTP %{http_code}]\n" -X POST "https://api.opensubtitles.com/api/v1$endpoint" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Accept: application/json" \
        -H "Content-Type: $content_type" \
        -H "User-Agent: se-ai-translator-test v0.1.0" \
        "${extra_args[@]}"
}

# --- Tests -----------------------------------------------------------------

test_get_credits() {
    echo ""
    echo -e "${CYAN}=== Test 1: POST /ai/credits (raw response) ===${NC}"

    local response
    response=$(api_post_raw "/ai/credits" "application/json" -d "{}")

    echo "$response" | sed 's/\[HTTP /\n[HTTP /'
    echo ""

    # Extract the body (everything before the [HTTP ...] marker)
    local body
    body=$(echo "$response" | sed 's/\[HTTP .*//' )

    echo -e "${CYAN}=== Test 1: Parsed fields ===${NC}"
    echo "$body" | jq -e . >/dev/null 2>&1 || {
        fail "Response is not valid JSON"
        return 0
    }

    echo "$body" | jq '{
        success: .success,
        credits: .credits,
        data_credits: .data.credits,
        error: .error,
        keys: (keys),
        data_keys: (.data | keys)
    }'

    echo ""
    local data_credits
    data_credits=$(echo "$body" | jq -r '.data.credits // empty')
    if [[ -n "$data_credits" ]]; then
        ok "data.credits=$data_credits"
    else
        fail "No data.credits field"
    fi
    return 0
}

test_get_credits_empty_body() {
    echo ""
    echo -e "${CYAN}=== Test 2: POST /ai/credits (no body) ===${NC}"

    local response
    response=$(api_post_raw "/ai/credits" "application/json")

    echo "$response" | sed 's/\[HTTP /\n[HTTP /'
    echo ""
    ok "No-body request completed"
}

test_get_credits_wrong_token() {
    echo ""
    echo -e "${CYAN}=== Test 3: POST /ai/credits with invalid token ===${NC}"

    local response
    response=$(curl -s -w "\n[HTTP %{http_code}]" -X POST "https://api.opensubtitles.com/api/v1/ai/credits" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Authorization: Bearer INVALID_TOKEN_$(date +%s)" \
        -H "Accept: application/json" \
        -H "Content-Type: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0" \
        -d "{}")

    echo "$response" | sed 's/\[HTTP /\n[HTTP /'
    echo ""
    ok "Invalid token probe completed"
}

test_get_credit_packages_no_email() {
    echo ""
    echo -e "${CYAN}=== Test 4: POST /ai/credits/buy (no email, JSON body) ===${NC}"

    local response
    response=$(api_post_raw "/ai/credits/buy" "application/json" -d "{}")

    echo "$response" | sed 's/\[HTTP /\n[HTTP /'
    echo ""

    local body
    body=$(echo "$response" | sed 's/\[HTTP .*//')
    echo -e "${CYAN}=== Test 4: Parsed ===${NC}"
    if echo "$body" | jq -e . >/dev/null 2>&1; then
        echo "$body" | jq '{
            success: .success,
            data_count: (.data | if type == "array" then length else "?" end),
            keys: keys
        }'
    else
        fail "Response is not JSON"
    fi
}

test_get_credit_packages_form_no_email() {
    echo ""
    echo -e "${CYAN}=== Test 5: POST /ai/credits/buy (multipart form, no email) ===${NC}"

    local response
    response=$(curl -s -w "\n[HTTP %{http_code}]" -X POST "https://api.opensubtitles.com/api/v1/ai/credits/buy" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Accept: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0" \
        -F "dummy=value")

    echo "$response" | sed 's/\[HTTP /\n[HTTP /'
    echo ""

    local body
    body=$(echo "$response" | sed 's/\[HTTP .*//')
    if echo "$body" | jq -e . >/dev/null 2>&1; then
        echo "$body" | jq '{
            success: .success,
            data_count: (.data | if type == "array" then length else "?" end),
            first_pkg: (.data | if type == "array" then .[0] else null end)
        }'
    fi
    ok "Multipart probe completed"
}

test_get_credit_packages_with_email() {
    echo ""
    echo -e "${CYAN}=== Test 6: POST /ai/credits/buy (multipart form, email=$OS_USERNAME) ===${NC}"

    local response
    response=$(curl -s -w "\n[HTTP %{http_code}]" -X POST "https://api.opensubtitles.com/api/v1/ai/credits/buy" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Accept: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0" \
        -F "email=$OS_USERNAME")

    echo "$response" | sed 's/\[HTTP /\n[HTTP /'
    echo ""

    local body
    body=$(echo "$response" | sed 's/\[HTTP .*//')
    if echo "$body" | jq -e . >/dev/null 2>&1; then
        local count
        count=$(echo "$body" | jq '.data | if type == "array" then length else 0 end')
        ok "Got $count package(s) with email filter"

        echo "$body" | jq -r '
            if (.data | type) == "array" then
                .data[] | "  - " + (.name // "?") + ": " + (.value // "?") +
                " (discount=" + ((.discount_percent // 0) | tostring) + "%)"
            else
                "  No data array found"
            end
        '

        echo ""
        echo -e "${CYAN}=== Test 6: Package field schema ===${NC}"
        if [[ "$count" != "0" ]]; then
            echo "$body" | jq '.data[0] | {keys: (keys), values: .}'
        else
            echo "  (no packages to inspect)"
        fi
    else
        fail "Response is not JSON"
    fi
}

test_get_credit_packages_wrong_token() {
    echo ""
    echo -e "${CYAN}=== Test 7: POST /ai/credits/buy with invalid token ===${NC}"

    local response
    response=$(curl -s -w "\n[HTTP %{http_code}]" -X POST "https://api.opensubtitles.com/api/v1/ai/credits/buy" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Authorization: Bearer INVALID_TOKEN_$(date +%s)" \
        -H "Accept: application/json" \
        -H "Content-Type: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0" \
        -d "{}")

    echo "$response" | sed 's/\[HTTP /\n[HTTP /'
    echo ""
    ok "Invalid token probe completed"
}

# --- Main ------------------------------------------------------------------

echo "========================================"
echo "  API Test: Credits Endpoints"
echo "========================================"

check_deps
load_env

info "Logging in..."
TOKEN=$(fetch_auth_token)
ok "Authenticated"

test_get_credits
test_get_credits_empty_body
test_get_credits_wrong_token
test_get_credit_packages_no_email
test_get_credit_packages_form_no_email
test_get_credit_packages_with_email
test_get_credit_packages_wrong_token

echo ""
echo -e "${GREEN}All credits tests completed.${NC}"
