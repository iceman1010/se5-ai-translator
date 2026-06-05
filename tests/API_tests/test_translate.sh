#!/usr/bin/env bash
# Note: no `set -e` — run every probe even if one fails, so we see all responses.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TESTS_DIR="$(dirname "$SCRIPT_DIR")"
ENV_FILE="$TESTS_DIR/.env"
EXAMPLE_SRT="$TESTS_DIR/example_srt/test_archer_2min.srt"
OUTPUT_DIR="$TESTS_DIR/out"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
DIM='\033[2m'
NC='\033[0m'

info()  { echo -e "${YELLOW}[INFO]${NC} $*"; }
ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
fail()  { echo -e "${RED}[FAIL]${NC} $*"; }

# --- Config (overridable via env) -------------------------------------------
SOURCE_LANG="${OS_TRANSLATE_FROM:-en}"
TARGET_LANG="${OS_TRANSLATE_TO:-es}"
ENGINE="${OS_TRANSLATE_API:-deepl2}"
POLL_INTERVAL="${OS_POLL_INTERVAL:-3}"   # seconds between polls
POLL_MAX="${OS_POLL_MAX:-100}"           # ~5 minutes max

# Globals populated by submit/poll helpers (declared empty for `set -u`).
SUBMIT_HTTP_CODE=""
SUBMIT_BODY=""
POLL_BODY=""
CORRELATION_ID=""
INLINE_TRANSLATION=""
FINAL_STATUS=""

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

# Submit translation. Args: source_lang, target_lang, engine.
# Sets globals: SUBMIT_HTTP_CODE, SUBMIT_BODY.
submit_translation() {
    local src="$1"
    local tgt="$2"
    local api="$3"

    local resp
    resp=$(curl -s -w "\n%{http_code}" -X POST "https://api.opensubtitles.com/api/v1/ai/translate" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Accept: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0" \
        -F "file=@$EXAMPLE_SRT;type=text/plain" \
        -F "translate_from=$src" \
        -F "translate_to=$tgt" \
        -F "api=$api" \
        -F "return_content=true")

    SUBMIT_HTTP_CODE=$(echo "$resp" | tail -1)
    SUBMIT_BODY=$(echo "$resp" | sed '$d')
}

# Poll /ai/translation/{correlation_id} until COMPLETED/ERROR or timeout.
# Echoes the final status string. Final body is in $POLL_BODY.
poll_translation() {
    local correlation_id="$1"
    local attempt=0

    while [[ $attempt -lt $POLL_MAX ]]; do
        attempt=$((attempt + 1))

        POLL_BODY=$(curl -s -X POST "https://api.opensubtitles.com/api/v1/ai/translation/$correlation_id" \
            -H "Api-Key: $OS_API_KEY" \
            -H "Authorization: Bearer $TOKEN" \
            -H "Accept: application/json" \
            -H "Content-Type: application/json" \
            -H "User-Agent: se-ai-translator-test v0.1.0" \
            -d "{}")

        local status
        status=$(echo "$POLL_BODY" | jq -r '.status // "UNKNOWN"')

        if [[ "$status" == "COMPLETED" || "$status" == "ERROR" || "$status" == "TIMEOUT" ]]; then
            echo "$status"
            return 0
        fi

        # Progress dot so the user sees we're alive.
        printf "${DIM}.${NC}"
        sleep "$POLL_INTERVAL"
    done

    echo "TIMEOUT"
}

# --- Tests ------------------------------------------------------------------

test_submit_translation() {
    echo ""
    echo -e "${CYAN}=== Test 1: Submit translation ($SOURCE_LANG → $TARGET_LANG via $ENGINE) ===${NC}"

    if [[ ! -f "$EXAMPLE_SRT" ]]; then
        fail "Example SRT not found: $EXAMPLE_SRT"
        return 1
    fi

    info "Uploading $(basename "$EXAMPLE_SRT") ($(wc -l < "$EXAMPLE_SRT") lines)..."
    submit_translation "$SOURCE_LANG" "$TARGET_LANG" "$ENGINE"

    echo ""
    echo -e "${CYAN}-- submit response (HTTP $SUBMIT_HTTP_CODE) --${NC}"
    echo "$SUBMIT_BODY" | jq . 2>/dev/null || echo "$SUBMIT_BODY"

    if [[ "$SUBMIT_HTTP_CODE" != "200" && "$SUBMIT_HTTP_CODE" != "201" && "$SUBMIT_HTTP_CODE" != "202" ]]; then
        fail "Submit failed with HTTP $SUBMIT_HTTP_CODE"
        return 1
    fi

    CORRELATION_ID=$(echo "$SUBMIT_BODY" | jq -r '.correlation_id // empty')
    if [[ -z "$CORRELATION_ID" ]]; then
        fail "No correlation_id in submit response"
        return 1
    fi

    # Some happy paths return the translation inline (no polling needed).
    INLINE_TRANSLATION=$(echo "$SUBMIT_BODY" | jq -r '.translation // empty')

    ok "correlation_id=$CORRELATION_ID"
    return 0
}

test_poll_translation() {
    echo ""
    echo -e "${CYAN}=== Test 2: Poll until completion ===${NC}"

    if [[ -n "$INLINE_TRANSLATION" ]]; then
        ok "Translation returned inline; no polling needed."
        POLL_BODY="$SUBMIT_BODY"
        FINAL_STATUS="COMPLETED"
        return 0
    fi

    if [[ -z "${CORRELATION_ID:-}" ]]; then
        fail "No correlation_id from Test 1; skipping poll"
        return 1
    fi

    info "Polling every ${POLL_INTERVAL}s (max ${POLL_MAX} attempts)..."
    FINAL_STATUS=$(poll_translation "$CORRELATION_ID")
    echo ""

    echo -e "${CYAN}-- final poll response --${NC}"
    echo "$POLL_BODY" | jq . 2>/dev/null || echo "$POLL_BODY"

    if [[ "$FINAL_STATUS" != "COMPLETED" ]]; then
        fail "Translation did not complete (status=$FINAL_STATUS)"
        return 1
    fi

    ok "Translation completed"
}

test_save_and_inspect_output() {
    echo ""
    echo -e "${CYAN}=== Test 3: Save and inspect translated content ===${NC}"

    # API returns content under various keys depending on engine / build.
    local content
    content=$(echo "$POLL_BODY" | jq -r '
        .data.return_content //
        .data.translation //
        .translation //
        .data.content //
        empty
    ')

    if [[ -z "$content" || "$content" == "null" ]]; then
        fail "No translated content field found in response"
        echo "$POLL_BODY" | jq 'paths(scalars) as $p | select([$p[]] | join(".") | test("translation|return_content|content"; "i")) | {path: $p, value: getpath($p)}' 2>/dev/null
        return 1
    fi

    mkdir -p "$OUTPUT_DIR"
    local out_file="$OUTPUT_DIR/test_archer_2min.${TARGET_LANG}.srt"
    printf '%s\n' "$content" > "$out_file"
    ok "Saved translation to: $out_file"

    local src_lines dst_lines src_chars dst_chars
    src_lines=$(wc -l < "$EXAMPLE_SRT")
    dst_lines=$(wc -l < "$out_file")
    src_chars=$(wc -m < "$EXAMPLE_SRT")
    dst_chars=$(wc -m < "$out_file")

    echo ""
    echo -e "${CYAN}-- size comparison --${NC}"
    printf "  source (%s): %5d lines, %6d chars\n" "$SOURCE_LANG" "$src_lines" "$src_chars"
    printf "  target (%s): %5d lines, %6d chars\n" "$TARGET_LANG" "$dst_lines" "$dst_chars"

    if [[ "$dst_lines" -lt $((src_lines / 2)) ]]; then
        fail "Output has dramatically fewer lines than source — possibly truncated"
    fi
    if [[ "$src_lines" -eq "$dst_lines" && "$src_chars" -eq "$dst_chars" ]]; then
        fail "Output is byte-identical to source — translation may not have happened"
    fi

    echo ""
    echo -e "${CYAN}-- first 12 lines of translated SRT --${NC}"
    sed -n '1,12p' "$out_file" | sed "s/^/  ${DIM}/;s/$/${NC}/"

    # Credits / pricing info if present.
    echo ""
    echo -e "${CYAN}-- usage / pricing fields (if any) --${NC}"
    echo "$POLL_BODY" | jq '{
        character_count: .data.character_count,
        unit_price: .data.unit_price,
        total_price: .data.total_price,
        credits_left: .data.credits_left
    }' 2>/dev/null
}

test_auto_detect_source() {
    echo ""
    echo -e "${CYAN}=== Test 4: Auto-detect source language (translate_from=auto) ===${NC}"

    info "Submitting with translate_from=auto, translate_to=$TARGET_LANG, api=$ENGINE..."
    submit_translation "auto" "$TARGET_LANG" "$ENGINE"

    local auto_corr
    auto_corr=$(echo "$SUBMIT_BODY" | jq -r '.correlation_id // empty')

    if [[ -z "$auto_corr" ]]; then
        fail "No correlation_id returned for auto-detect submission"
        echo "$SUBMIT_BODY" | jq . 2>/dev/null
        return 1
    fi

    ok "Auto-detect submission accepted: correlation_id=$auto_corr"

    # Don't wait for full completion (saves credits / time). Just verify it
    # wasn't rejected immediately.
    if [[ "$SUBMIT_HTTP_CODE" == "200" || "$SUBMIT_HTTP_CODE" == "201" || "$SUBMIT_HTTP_CODE" == "202" ]]; then
        ok "Auto-detect path accepted (HTTP $SUBMIT_HTTP_CODE)"
    else
        fail "Auto-detect path rejected (HTTP $SUBMIT_HTTP_CODE)"
        echo "$SUBMIT_BODY" | jq . 2>/dev/null
    fi
}

test_invalid_target_language() {
    echo ""
    echo -e "${CYAN}=== Test 5: Invalid target language (translate_to=zzzz) ===${NC}"

    submit_translation "$SOURCE_LANG" "zzzz" "$ENGINE"

    echo ""
    echo -e "${CYAN}-- response (HTTP $SUBMIT_HTTP_CODE) --${NC}"
    echo "$SUBMIT_BODY" | jq . 2>/dev/null || echo "$SUBMIT_BODY"

    if [[ "$SUBMIT_HTTP_CODE" == "4"* ]]; then
        ok "Server rejected invalid language pair as expected (HTTP $SUBMIT_HTTP_CODE)"
    else
        # Some servers return 200 with status=ERROR in the body — accept that too.
        local status
        status=$(echo "$SUBMIT_BODY" | jq -r '.status // .data.status // empty')
        if [[ "$status" == "ERROR" ]]; then
            ok "Server reported status=ERROR for invalid language pair"
        else
            fail "Expected 4xx or status=ERROR, got HTTP $SUBMIT_HTTP_CODE"
        fi
    fi
}

test_no_auth_token() {
    echo ""
    echo -e "${CYAN}=== Test 6: Submit without auth token ===${NC}"

    local resp
    resp=$(curl -s -w "\n[HTTP %{http_code}]" -X POST "https://api.opensubtitles.com/api/v1/ai/translate" \
        -H "Api-Key: $OS_API_KEY" \
        -H "Accept: application/json" \
        -H "User-Agent: se-ai-translator-test v0.1.0" \
        -F "file=@$EXAMPLE_SRT;type=text/plain" \
        -F "translate_from=$SOURCE_LANG" \
        -F "translate_to=$TARGET_LANG" \
        -F "api=$ENGINE" \
        -F "return_content=true")

    echo "$resp" | sed 's/\[HTTP /\n[HTTP /'
    echo ""
    ok "No-auth probe completed"
}

# --- Main -------------------------------------------------------------------

echo "========================================"
echo "  API Test: Translation Endpoints"
echo "  Source: $SOURCE_LANG → Target: $TARGET_LANG"
echo "  Engine: $ENGINE"
echo "========================================"

check_deps
load_env

info "Logging in..."
TOKEN=$(fetch_auth_token)
ok "Authenticated"

test_submit_translation
test_poll_translation
test_save_and_inspect_output
test_auto_detect_source
test_invalid_target_language
test_no_auth_token

echo ""
echo -e "${GREEN}All translation tests completed.${NC}"
echo -e "${DIM}Translated files saved under: $OUTPUT_DIR${NC}"
