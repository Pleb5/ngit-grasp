#!/usr/bin/env bash
#
# 01-fetch-events.sh - Fetch nostr events from a relay for migration analysis
#
# PHASE 1 of the GRASP relay to ngit-grasp migration analysis pipeline.
# Fetches kind 30618 (state), 30617 (announcement), and 5 (deletion) events.
#
# USAGE:
#   ./01-fetch-events.sh <relay-url> <output-dir>
#
# EXAMPLES:
#   # Fetch from production relay
#   ./01-fetch-events.sh wss://relay.ngit.dev output/prod
#
#   # Fetch from archive relay
#   ./01-fetch-events.sh wss://archive.relay.ngit.dev output/archive
#
#   # Full migration analysis setup
#   mkdir -p work/migration-analysis-$(date +%Y%m%d-%H%M)
#   ./01-fetch-events.sh wss://relay.ngit.dev work/migration-analysis-*/prod
#   ./01-fetch-events.sh wss://archive.relay.ngit.dev work/migration-analysis-*/archive
#
# OUTPUT:
#   <output-dir>/raw/state-events.json      - kind 30618 events (one per line, JSONL)
#   <output-dir>/raw/announcements.json     - kind 30617 events (one per line, JSONL)
#   <output-dir>/raw/deletions.json         - kind 5 events (one per line, JSONL)
#
# OUTPUT FORMAT:
#   Each file contains one JSON event per line (JSONL format).
#   Events are the raw nostr event objects as returned by the relay.
#
# PREREQUISITES:
#   - nak (Nostr Army Knife) - https://github.com/fiatjaf/nak
#   - jq (for counting/validation)
#
# RUNTIME: ~30 seconds per relay (depends on network and event count)
#
# NOTES:
#   - Uses --paginate to ensure all events are fetched (not just first page)
#   - If event counts are exact multiples of 250, pagination may have failed
#   - Run Phase 1 and Phase 2 back-to-back for accurate snapshot
#
# SEE ALSO:
#   docs/how-to/migrate-to-ngit-grasp.md - Full migration guide
#

set -euo pipefail

# Colors for output (disabled if not a terminal)
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    NC='\033[0m' # No Color
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    NC=''
fi

log_info() {
    echo -e "${BLUE}[INFO]${NC} $*" >&2
}

log_success() {
    echo -e "${GREEN}[OK]${NC} $*" >&2
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $*" >&2
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*" >&2
}

usage() {
    echo "Usage: $0 <relay-url> <output-dir>"
    echo ""
    echo "Arguments:"
    echo "  relay-url   WebSocket URL of the relay (e.g., wss://relay.ngit.dev)"
    echo "  output-dir  Directory to store fetched events (e.g., output/prod)"
    echo ""
    echo "Examples:"
    echo "  $0 wss://relay.ngit.dev output/prod"
    echo "  $0 wss://archive.relay.ngit.dev output/archive"
    exit 1
}

# Check prerequisites
check_prerequisites() {
    local missing=0
    
    if ! command -v nak &> /dev/null; then
        log_error "nak not found. Install from: https://github.com/fiatjaf/nak"
        missing=1
    fi
    
    if ! command -v jq &> /dev/null; then
        log_error "jq not found. Install with your package manager."
        missing=1
    fi
    
    if [[ $missing -eq 1 ]]; then
        exit 1
    fi
}

# Fetch events of a specific kind
# Args: $1=relay, $2=kind, $3=output_file, $4=description
fetch_kind() {
    local relay="$1"
    local kind="$2"
    local output_file="$3"
    local description="$4"
    
    log_info "Fetching $description (kind $kind) from $relay..."
    
    local start_time
    start_time=$(date +%s)
    
    # Use --paginate to ensure we get all events, not just first page
    # nak outputs one event per line (JSONL format)
    if ! nak req -k "$kind" --paginate "$relay" > "$output_file" 2>/dev/null; then
        log_error "Failed to fetch $description from $relay"
        return 1
    fi
    
    local end_time
    end_time=$(date +%s)
    local duration=$((end_time - start_time))
    
    # Count events
    local count
    count=$(wc -l < "$output_file" | tr -d ' ')
    
    # Warn if count is suspicious (exact multiple of 250 suggests pagination issue)
    if [[ $count -gt 0 ]] && [[ $((count % 250)) -eq 0 ]]; then
        log_warn "$description count ($count) is exact multiple of 250 - pagination may have failed!"
    fi
    
    log_success "Fetched $count $description in ${duration}s -> $output_file"
    
    echo "$count"
}

# Main
main() {
    if [[ $# -ne 2 ]]; then
        usage
    fi
    
    local relay="$1"
    local output_dir="$2"
    
    # Validate relay URL
    if [[ ! "$relay" =~ ^wss?:// ]]; then
        log_error "Invalid relay URL: $relay (must start with ws:// or wss://)"
        exit 1
    fi
    
    check_prerequisites
    
    log_info "Starting event fetch from $relay"
    log_info "Output directory: $output_dir"
    
    # Create output directory structure
    local raw_dir="$output_dir/raw"
    mkdir -p "$raw_dir"
    
    local total_start
    total_start=$(date +%s)
    
    # Fetch each event type
    local state_count announcement_count deletion_count
    
    state_count=$(fetch_kind "$relay" 30618 "$raw_dir/state-events.json" "state events")
    announcement_count=$(fetch_kind "$relay" 30617 "$raw_dir/announcements.json" "announcements")
    deletion_count=$(fetch_kind "$relay" 5 "$raw_dir/deletions.json" "deletion requests")
    
    local total_end
    total_end=$(date +%s)
    local total_duration=$((total_end - total_start))
    
    # Summary
    echo ""
    log_info "=== Fetch Summary ==="
    log_info "Relay: $relay"
    log_info "Output: $output_dir"
    log_info "State events (30618):    $state_count"
    log_info "Announcements (30617):   $announcement_count"
    log_info "Deletions (5):           $deletion_count"
    log_info "Total time: ${total_duration}s"
    echo ""
    
    # Output file listing for easy copy/paste
    log_info "Output files:"
    echo "  $raw_dir/state-events.json"
    echo "  $raw_dir/announcements.json"
    echo "  $raw_dir/deletions.json"
}

main "$@"
