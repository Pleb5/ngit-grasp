#!/usr/bin/env bash
#
# 30-extract-parse-failures.sh - Extract parse failure events from systemd logs
#
# PHASE 4a of the GRASP relay to ngit-grasp migration analysis pipeline.
# Extracts structured [PARSE_FAIL] log entries from journalctl.
#
# USAGE:
#   ./30-extract-parse-failures.sh <service-name> <output-dir> [options]
#
# EXAMPLES:
#   # Extract from ngit-grasp service (last 30 days, default)
#   ./30-extract-parse-failures.sh ngit-grasp.service output/logs
#
#   # Extract with custom time range
#   ./30-extract-parse-failures.sh ngit-grasp.service output/logs --since "2026-01-01"
#
#   # Extract from specific time window
#   ./30-extract-parse-failures.sh ngit-grasp.service output/logs --since "2026-01-15" --until "2026-01-22"
#
# OPTIONS:
#   --since <date>   Start date for log extraction (default: 30 days ago)
#   --until <date>   End date for log extraction (default: now)
#   --dry-run        Show what would be extracted without writing files
#
# OUTPUT:
#   <output-dir>/parse-failures.txt
#
# OUTPUT FORMAT (TSV):
#   repo<TAB>npub<TAB>kind<TAB>event_id<TAB>reason
#
# EXPECTED LOG FORMAT:
#   The script looks for structured log entries in this format:
#
#   2026-01-22T10:30:45Z ngit-grasp[1234]: [PARSE_FAIL] kind=30618 event_id=abc123... reason="invalid refs format" repo=myrepo npub=npub1...
#
#   Required fields: kind, event_id, reason
#   Optional fields: repo, npub (may not be available if parsing failed early)
#
# DEPENDENCY:
#   This script requires logging improvements in ngit-grasp to emit structured
#   [PARSE_FAIL] log entries. Until those are implemented, this script will
#   find no matching entries (which is handled gracefully).
#
#   See: docs/how-to/migrate-to-ngit-grasp.md (Dependencies section)
#
#   Expected Rust logging code:
#     tracing::warn!(
#         target: "migration",
#         "[PARSE_FAIL] kind={} event_id={} reason=\"{}\" repo={} npub={}",
#         event.kind, event.id, reason, identifier, npub
#     );
#
# PREREQUISITES:
#   - journalctl (systemd)
#   - grep, awk (standard Unix tools)
#   - Access to systemd journal (may require sudo or journal group membership)
#
# RUNTIME: Depends on log volume, typically < 30 seconds
#
# SEE ALSO:
#   docs/how-to/migrate-to-ngit-grasp.md - Full migration guide
#   31-extract-purgatory-expiry.sh - Companion script for purgatory expiry logs
#

set -euo pipefail

# Get script directory for sourcing helpers
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Source the service validation helper
if [[ -f "$SCRIPT_DIR/validate-service.sh" ]]; then
    source "$SCRIPT_DIR/validate-service.sh"
fi

# Colors for output (disabled if not a terminal)
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    NC='\033[0m'
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
    echo "Usage: $0 <service-name> <output-dir> [options]"
    echo ""
    echo "Arguments:"
    echo "  service-name  Systemd service name (e.g., ngit-grasp.service)"
    echo "  output-dir    Directory to store extracted log data"
    echo ""
    echo "Options:"
    echo "  --since <date>   Start date (default: 30 days ago)"
    echo "  --until <date>   End date (default: now)"
    echo "  --dry-run        Show what would be extracted without writing"
    echo ""
    echo "Examples:"
    echo "  $0 ngit-grasp.service output/logs"
    echo "  $0 ngit-grasp.service output/logs --since '2026-01-01'"
    echo "  $0 ngit-grasp.service output/logs --since '2026-01-15' --until '2026-01-22'"
    echo ""
    echo "Expected log format:"
    echo "  [PARSE_FAIL] kind=30618 event_id=abc123 reason=\"...\" repo=myrepo npub=npub1..."
    exit 1
}

# Parse a single log line and extract fields
# Input: log line containing [PARSE_FAIL]
# Output: TSV line: repo<TAB>npub<TAB>kind<TAB>event_id<TAB>reason
parse_log_line() {
    local line="$1"
    
    # Extract fields using grep -oP (Perl regex) or awk
    # Fields: kind, event_id, reason, repo (optional), npub (optional)
    
    local kind event_id reason repo npub
    
    # Extract kind=VALUE
    kind=$(echo "$line" | grep -oP 'kind=\K[0-9]+' || echo "")
    
    # Extract event_id=VALUE (hex string, possibly truncated with ...)
    event_id=$(echo "$line" | grep -oP 'event_id=\K[a-f0-9]+' || echo "")
    
    # Extract reason="VALUE" (quoted string)
    reason=$(echo "$line" | grep -oP 'reason="\K[^"]*' || echo "")
    
    # Extract repo=VALUE (optional, unquoted identifier)
    repo=$(echo "$line" | grep -oP 'repo=\K[^ ]+' || echo "")
    
    # Extract npub=VALUE (optional, npub1... format)
    npub=$(echo "$line" | grep -oP 'npub=\K[^ ]+' || echo "")
    
    # Only output if we have the required fields
    if [[ -n "$kind" && -n "$event_id" && -n "$reason" ]]; then
        printf '%s\t%s\t%s\t%s\t%s\n' "$repo" "$npub" "$kind" "$event_id" "$reason"
    fi
}

# Main
main() {
    if [[ $# -lt 2 ]]; then
        usage
    fi
    
    local service="$1"
    local output_dir="$2"
    shift 2
    
    # Default time range: last 30 days
    local since_date
    since_date=$(date -d "30 days ago" "+%Y-%m-%d" 2>/dev/null || date -v-30d "+%Y-%m-%d" 2>/dev/null || echo "")
    local until_date=""
    local dry_run=false
    
    # Parse options
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --since)
                since_date="$2"
                shift 2
                ;;
            --until)
                until_date="$2"
                shift 2
                ;;
            --dry-run)
                dry_run=true
                shift
                ;;
            *)
                log_error "Unknown option: $1"
                usage
                ;;
        esac
    done
    
    # Validate service name format
    if [[ ! "$service" =~ \.service$ ]]; then
        service="${service}.service"
    fi
    
    # Validate service is appropriate for structured logging
    # This prevents the common mistake of using ngit-relay instead of ngit-grasp
    if type validate_service_for_structured_logging &>/dev/null; then
        # Use non-interactive mode if not a terminal, skip log check (we'll do our own)
        local interactive="true"
        [[ ! -t 0 ]] && interactive="false"
        
        if ! validate_service_for_structured_logging "$service" "false" "$interactive"; then
            log_error "Service validation failed. Use an ngit-grasp service for structured logging."
            exit 1
        fi
    else
        # Fallback validation if helper not available
        if [[ "$service" == *"ngit-relay"* ]]; then
            log_error "Service name appears to be ngit-relay: $service"
            log_error "Structured logging ([PARSE_FAIL]) only exists in ngit-grasp services."
            log_error "Please use the ngit-grasp archive service instead."
            log_error ""
            log_error "To find the correct service:"
            log_error "  systemctl list-units 'ngit-grasp*' --all"
            exit 1
        fi
    fi
    
    log_info "Extracting parse failures from systemd logs"
    log_info "Service: $service"
    log_info "Output: $output_dir"
    log_info "Time range: ${since_date:-beginning} to ${until_date:-now}"
    
    # Check if journalctl is available
    if ! command -v journalctl &> /dev/null; then
        log_error "journalctl not found. This script requires systemd."
        exit 1
    fi
    
    # Build journalctl command
    local journal_cmd="journalctl -u $service --no-pager -o short-iso"
    
    if [[ -n "$since_date" ]]; then
        journal_cmd="$journal_cmd --since '$since_date'"
    fi
    
    if [[ -n "$until_date" ]]; then
        journal_cmd="$journal_cmd --until '$until_date'"
    fi
    
    log_info "Running: $journal_cmd | grep '\\[PARSE_FAIL\\]'"
    
    if [[ "$dry_run" == true ]]; then
        log_info "[DRY RUN] Would extract to: $output_dir/parse-failures.txt"
        
        # Show sample of what would be extracted
        log_info "Checking for matching log entries..."
        local sample_count
        sample_count=$(eval "$journal_cmd" 2>/dev/null | grep -c '\[PARSE_FAIL\]' || echo "0")
        sample_count="${sample_count//[^0-9]/}"  # Strip non-numeric characters
        sample_count="${sample_count:-0}"
        log_info "Found $sample_count matching log entries"
        
        if [[ "$sample_count" -eq 0 ]]; then
            log_warn "No [PARSE_FAIL] entries found in logs."
            log_warn "This is expected if ngit-grasp logging improvements are not yet deployed."
            log_warn "See: docs/how-to/migrate-to-ngit-grasp.md (Dependencies section)"
        fi
        
        exit 0
    fi
    
    # Create output directory
    mkdir -p "$output_dir"
    
    local output_file="$output_dir/parse-failures.txt"
    local temp_file
    temp_file=$(mktemp)
    
    # Extract and parse log entries
    log_info "Extracting log entries..."
    
    # Get raw log lines containing [PARSE_FAIL]
    local raw_lines
    raw_lines=$(eval "$journal_cmd" 2>/dev/null | grep '\[PARSE_FAIL\]' || true)
    
    if [[ -z "$raw_lines" ]]; then
        log_warn "No [PARSE_FAIL] entries found in logs."
        log_warn ""
        log_warn "This is expected if ngit-grasp logging improvements are not yet deployed."
        log_warn "The structured log format required by this script:"
        log_warn ""
        log_warn "  [PARSE_FAIL] kind=30618 event_id=abc123 reason=\"...\" repo=myrepo npub=npub1..."
        log_warn ""
        log_warn "See: docs/how-to/migrate-to-ngit-grasp.md (Dependencies section)"
        log_warn ""
        
        # Create empty output file with header comment
        {
            echo "# Parse failures extracted from $service"
            echo "# Time range: ${since_date:-beginning} to ${until_date:-now}"
            echo "# Extracted: $(date -Iseconds)"
            echo "# Format: repo<TAB>npub<TAB>kind<TAB>event_id<TAB>reason"
            echo "#"
            echo "# NOTE: No [PARSE_FAIL] entries found."
            echo "# This is expected if ngit-grasp logging improvements are not yet deployed."
        } > "$output_file"
        
        log_info "Created empty output file: $output_file"
        exit 0
    fi
    
    # Write header
    {
        echo "# Parse failures extracted from $service"
        echo "# Time range: ${since_date:-beginning} to ${until_date:-now}"
        echo "# Extracted: $(date -Iseconds)"
        echo "# Format: repo<TAB>npub<TAB>kind<TAB>event_id<TAB>reason"
    } > "$output_file"
    
    # Parse each line
    local count=0
    while IFS= read -r line; do
        local parsed
        parsed=$(parse_log_line "$line")
        if [[ -n "$parsed" ]]; then
            echo "$parsed" >> "$output_file"
            ((count++))
        fi
    done <<< "$raw_lines"
    
    rm -f "$temp_file"
    
    # Summary
    echo ""
    log_info "=== Extraction Summary ==="
    log_info "Service: $service"
    log_info "Time range: ${since_date:-beginning} to ${until_date:-now}"
    log_success "Extracted $count parse failure entries"
    echo ""
    log_info "Output file: $output_file"
    
    if [[ $count -gt 0 ]]; then
        echo ""
        log_info "Sample entries (first 5):"
        tail -n +5 "$output_file" | head -5 | while IFS=$'\t' read -r repo npub kind event_id reason; do
            echo "  kind=$kind repo=$repo reason=\"$reason\""
        done
    fi
    
    # Breakdown by kind
    if [[ $count -gt 0 ]]; then
        echo ""
        log_info "Breakdown by event kind:"
        tail -n +5 "$output_file" | awk -F'\t' '{print $3}' | sort | uniq -c | sort -rn | while read -r cnt kind; do
            echo "  kind $kind: $cnt failures"
        done
    fi
}

main "$@"
