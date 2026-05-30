#!/usr/bin/env bash
set -euo pipefail

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

api_post() {
    local endpoint="$1"
    local body="${2:-{}}"
    curl -s -X POST "https://api.opensubtitles.com/api/v1$endpoint" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Accept: application/json" \
        -H "Content-Type: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0" \
        -d "$body"
}

test_engines() {
    echo ""
    echo -e "${CYAN}=== Test: Fetch Engines ===${NC}"
    local response
    response=$(api_post "/ai/info/translation_apis")

    local engines
    engines=$(echo "$response" | jq -r '.data[]')

    if [[ -z "$engines" ]]; then
        fail "No engines returned"
        echo "$response" | jq . >&2
        return 1
    fi

    local count
    count=$(echo "$engines" | wc -l)
    ok "Got $count engines:"
    while IFS= read -r engine; do
        echo "  - $engine"
    done <<< "$engines"
}

test_languages_unfiltered() {
    echo ""
    echo -e "${CYAN}=== Test: Languages (unfiltered) ===${NC}"
    local response
    response=$(api_post "/ai/info/translation_languages" "{}")

    echo "$response" | jq -r '
        .data | to_entries[] |
        "  \(.key): \(.value | length) languages"
    '

    local total
    total=$(echo "$response" | jq '[.data[][] | .language_code] | unique | length')
    echo "  Total unique language codes: $total"
}

test_languages_per_engine() {
    echo ""
    echo -e "${CYAN}=== Test: Languages per engine (filtered) ===${NC}"

    local engines
    engines=$(api_post "/ai/info/translation_apis" | jq -r '.data[]')

    local failed=0
    while IFS= read -r engine; do
        local response
        response=$(api_post "/ai/info/translation_languages" "{\"api\":\"$engine\"}")

        local count
        count=$(echo "$response" | jq ".data[\"$engine\"] | length")

        if [[ "$count" == "0" || "$count" == "null" ]]; then
            fail "$engine: no languages returned"
            failed=$((failed + 1))
            continue
        fi

        ok "$engine: $count languages"
        echo "$response" | jq -r --arg e "$engine" '
            .data[$e][:3][] | "    \(.language_name) (\(.language_code))"
        '
        if [[ "$count" -gt 3 ]]; then
            echo "    ... and $((count - 3)) more"
        fi

        local other_count
        other_count=$(echo "$response" | jq "[.data | keys[] | select(. != \"$engine\")] | length")
        if [[ "$other_count" != "0" ]]; then
            echo "    NOTE: API also returned $other_count other engine(s) despite filter"
        fi
    done <<< "$engines"

    if [[ $failed -gt 0 ]]; then
        fail "$failed engine(s) had no languages"
        return 1
    fi
}

test_engine_language_diff() {
    echo ""
    echo -e "${CYAN}=== Test: Language differences between engines ===${NC}"

    local response
    response=$(api_post "/ai/info/translation_languages" "{}")

    echo "$response" | jq -r '
        .data | to_entries[] |
        "  \(.key): \(.value | length)"
    '

    echo ""

    local engines
    engines=$(echo "$response" | jq -r '.data | keys[]')

    while IFS= read -r engine; do
        local unique
        unique=$(echo "$response" | jq -r --arg e "$engine" '
            [(.data | to_entries[] | select(.key != $e) | .value[].language_code)] as $others |
            [.data[$e][] | .language_code] - $others |
            length
        ')

        if [[ "$unique" != "0" ]]; then
            echo "  $engine has $unique language(s) not in others:"
            echo "$response" | jq -r --arg e "$engine" '
                [(.data | to_entries[] | select(.key != $e) | .value[].language_code)] as $others |
                [.data[$e][] | .language_code] - $others | .[:10][]
            '
        else
            echo "  $engine: no unique languages (all shared)"
        fi
    done <<< "$engines"
}

# --- Main ---

echo "========================================"
echo "  API Test: Translation Languages"
echo "========================================"

check_deps
load_env

info "Logging in..."
TOKEN=$(fetch_auth_token)
ok "Authenticated"

test_engines
test_languages_unfiltered
test_languages_per_engine
test_engine_language_diff

echo ""
echo -e "${GREEN}All language tests completed.${NC}"
