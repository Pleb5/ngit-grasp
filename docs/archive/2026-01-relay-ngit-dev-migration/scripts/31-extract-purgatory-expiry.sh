#!/usr/bin/env bash
#
# 31-extract-purgatory-expiry.sh - Extract purgatory expiry events from systemd logs
#
# PHASE 4b of the GRASP relay to ngit-grasp migration analysis pipeline.
# Extracts structured [PURGATORY_EXPIRED] log entries from journalctl.
#
# USAGE:
#   ./31-extract-purgatory-expiry.sh <service-name> <output-dir> [options]
#
# EXAMPLES:
#   # Extract from ngit-grasp service (last 30 days, default)
#   ./31-extract-purgatory-expiry.sh ngit-grasp.service output/logs
#
#   # Extract with custom time range
#   ./31-extract-purgatory-expiry.sh ngit-grasp.service output/logs --since "2026-01-01"
#
#   # Extract from specific time window
#   ./31-extract-purgatory-expiry.sh ngit-grasp.service output/logs --since "2026-01-15" --until "2026-01-22"
#
# OPTIONS:
#   --since <date>   Start date for log extraction (default: 30 days ago)
#   --until <date>   End date for log extraction (default: now)
#   --dry-run        Show what would be extracted without writing files
#
# OUTPUT:
#   <output-dir>/purgatory-expired.txt
#
# OUTPUT FORMAT (TSV):
#   repo<TAB>npub<TAB>timestamp<TAB>reason
#
# EXPECTED LOG FORMAT:
#   The script looks for structured log entries in this format:
#
#   2026-01-22T10:30:45Z ngit-grasp[1234]: [PURGATORY_EXPIRED] repo=myrepo npub=npub1... reason="clone URL unreachable after 7 days"
#
#   Required fields: repo, npub
#   Optional fields: reason (explains why purgatory expired)
#
# BACKGROUND:
#   "Purgatory" is the state where ngit-grasp has received an announcement event
#   but cannot yet sync the git data (e.g., clone URL unreachable, git server down).
#   After a configurable timeout (default 7 days), the repository is marked as
#   expired and removed from purgatory.
#
#   Purgatory expiry during migration analysis indicates repositories that:
#   - Had valid announcements on the production relay
#   - Could not be synced to the archive relay
#   - May need manual intervention or investigation
#
# DEPENDENCY:
#   This script requires logging improvements in ngit-grasp to emit structured
#   [PURGATORY_EXPIRED] log entries. Until those are implemented, this script
#   will find no matching entries (which is handled gracefully).
#
#   See: docs/how-to/migrate-to-ngit-grasp.md (Dependencies section)
#
#   Expected Rust logging code:
#     tracing::warn!(
#         target: "migration",
#         "[PURGATORY_EXPIRED] repo={} npub={} reason=\"{}\"",
#         identifier, npub, reason
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
#   30-extract-parse-failures.sh - Companion script for parse failure logs
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
    echo "  [PURGATORY_EXPIRED] repo=myrepo npub=npub1... reason=\"...\""
    exit 1
}

# Parse a single log line and extract fields
# Input: log line containing [PURGATORY_EXPIRED]
# Output: TSV line: repo<TAB>npub<TAB>timestamp<TAB>reason
parse_log_line() {
    local line="$1"
    
    # Extract timestamp from the beginning of the log line
    # Format: 2026-01-22T10:30:45+0000 or similar ISO format
    local timestamp repo npub reason
    
    # Extract ISO timestamp from beginning of line
    timestamp=$(echo "$line" | grep -oP '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}' || echo "")
    
    # Extract repo=VALUE (unquoted identifier)
    repo=$(echo "$line" | grep -oP 'repo=\K[^ ]+' || echo "")
    
    # Extract npub=VALUE (npub1... format)
    npub=$(echo "$line" | grep -oP 'npub=\K[^ ]+' || echo "")
    
    # Extract reason="VALUE" (quoted string, optional)
    reason=$(echo "$line" | grep -oP 'reason="\K[^"]*' || echo "")
    
    # Only output if we have the required fields
    if [[ -n "$repo" && -n "$npub" ]]; then
        printf '%s\t%s\t%s\t%s\n' "$repo" "$npub" "$timestamp" "$reason"
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
            log_error "Structured logging ([PURGATORY_EXPIRED]) only exists in ngit-grasp services."
            log_error "Please use the ngit-grasp archive service instead."
            log_error ""
            log_error "To find the correct service:"
            log_error "  systemctl list-units 'ngit-grasp*' --all"
            exit 1
        fi
    fi
    
    log_info "Extracting purgatory expiry events from systemd logs"
    log_info "Service: $service"
    log_info "Output: $output_dir"
    log_info "Time range: ${since_date:-beginning} to ${until_date:-now}"
    
    # Check if journalctl is available
    if ! command -v journalctl &> /dev/null; then
        log_error "journalctl not found. This script requires systemd."
        exit 1
    fi
    
    # Validate service exists (check if journalctl can find any logs for it)
    # Note: We don't require the service to be running, just that it has logs
    if ! journalctl --no-pager -u "$service" -n 1 &>/dev/null; then
        log_warn "Could not query logs for service: $service"
        log_warn "This may indicate the service doesn't exist or you lack permissions."
        log_warn ""
        log_warn "To list available ngit-grasp services:"
        log_warn "  systemctl list-units 'ngit-grasp*' --all"
        log_warn "  journalctl --list-boots  # Check if you have journal access"
        log_warn ""
        # Continue anyway - the service might exist but have no logs yet
    fi
    
    # Build journalctl command
    local journal_cmd="journalctl -u $service --no-pager -o short-iso"
    
    if [[ -n "$since_date" ]]; then
        journal_cmd="$journal_cmd --since '$since_date'"
    fi
    
    if [[ -n "$until_date" ]]; then
        journal_cmd="$journal_cmd --until '$until_date'"
    fi
    
    log_info "Running: $journal_cmd | grep '\\[PURGATORY_EXPIRED\\]'"
    
    if [[ "$dry_run" == true ]]; then
        log_info "[DRY RUN] Would extract to: $output_dir/purgatory-expired.txt"
        
        # Show sample of what would be extracted
        log_info "Checking for matching log entries..."
        local sample_count
        sample_count=$(eval "$journal_cmd" 2>/dev/null | grep -c '\[PURGATORY_EXPIRED\]' || echo "0")
        sample_count="${sample_count//[^0-9]/}"  # Strip non-numeric characters
        sample_count="${sample_count:-0}"
        log_info "Found $sample_count matching log entries"
        
        if [[ "$sample_count" -eq 0 ]]; then
            log_warn "No [PURGATORY_EXPIRED] entries found in logs."
            log_warn "This is expected if ngit-grasp logging improvements are not yet deployed."
            log_warn "See: docs/how-to/migrate-to-ngit-grasp.md (Dependencies section)"
        fi
        
        exit 0
    fi
    
    # Create output directory
    mkdir -p "$output_dir"
    
    local output_file="$output_dir/purgatory-expired.txt"
    local temp_file
    temp_file=$(mktemp)
    
    # Extract and parse log entries
    log_info "Extracting log entries..."
    
    # Get raw log lines containing [PURGATORY_EXPIRED]
    # Capture stderr separately to detect journalctl errors
    local raw_lines journal_stderr journal_exit
    local temp_stderr
    temp_stderr=$(mktemp)
    
    raw_lines=$(eval "$journal_cmd" 2>"$temp_stderr" | grep '\[PURGATORY_EXPIRED\]' || true)
    journal_exit=$?
    journal_stderr=$(cat "$temp_stderr" 2>/dev/null || true)
    rm -f "$temp_stderr"
    
    # Report any journalctl errors (but don't fail - empty logs are valid)
    if [[ -n "$journal_stderr" ]]; then
        log_warn "journalctl reported: $journal_stderr"
    fi
    
    if [[ -z "$raw_lines" ]]; then
        log_warn "No [PURGATORY_EXPIRED] entries found in logs."
        log_warn ""
        log_warn "This is expected if ngit-grasp logging improvements are not yet deployed."
        log_warn "The structured log format required by this script:"
        log_warn ""
        log_warn "  [PURGATORY_EXPIRED] repo=myrepo npub=npub1... reason=\"...\""
        log_warn ""
        log_warn "See: docs/how-to/migrate-to-ngit-grasp.md (Dependencies section)"
        log_warn ""
        
        # Create empty output file with header comment
        {
            echo "# Purgatory expiry events extracted from $service"
            echo "# Time range: ${since_date:-beginning} to ${until_date:-now}"
            echo "# Extracted: $(date -Iseconds)"
            echo "# Format: repo<TAB>npub<TAB>timestamp<TAB>reason"
            echo "#"
            echo "# NOTE: No [PURGATORY_EXPIRED] entries found."
            echo "# This is expected if ngit-grasp logging improvements are not yet deployed."
        } > "$output_file"
        
        log_info "Created empty output file: $output_file"
        exit 0
    fi
    
    # Write header
    {
        echo "# Purgatory expiry events extracted from $service"
        echo "# Time range: ${since_date:-beginning} to ${until_date:-now}"
        echo "# Extracted: $(date -Iseconds)"
        echo "# Format: repo<TAB>npub<TAB>timestamp<TAB>reason"
    } > "$output_file"
    
    # Parse each line
    local count=0
    while IFS= read -r line; do
        local parsed
        parsed=$(parse_log_line "$line")
        if [[ -n "$parsed" ]]; then
            echo "$parsed" >> "$output_file"
            count=$((count + 1))
        fi
    done <<< "$raw_lines"
    
    rm -f "$temp_file"
    
    # Summary
    echo ""
    log_info "=== Extraction Summary ==="
    log_info "Service: $service"
    log_info "Time range: ${since_date:-beginning} to ${until_date:-now}"
    log_success "Extracted $count purgatory expiry entries"
    echo ""
    log_info "Output file: $output_file"
    
    if [[ $count -gt 0 ]]; then
        echo ""
        log_info "Sample entries (first 5):"
        # Use a subshell to avoid SIGPIPE issues with set -e
        (tail -n +5 "$output_file" | head -5 | while IFS=$'\t' read -r repo npub timestamp reason; do
            echo "  repo=$repo npub=${npub:0:20}... timestamp=$timestamp"
        done) || true
    fi
    
    # Show unique repos affected
    if [[ $count -gt 0 ]]; then
        echo ""
        local unique_repos
        unique_repos=$(tail -n +5 "$output_file" | awk -F'\t' '{print $1}' | sort -u | wc -l)
        log_info "Unique repositories affected: $unique_repos"
        
        echo ""
        log_info "Repositories with purgatory expiry:"
        # Use a subshell to avoid SIGPIPE issues with set -e
        (tail -n +5 "$output_file" | awk -F'\t' '{print $1}' | sort | uniq -c | sort -rn | head -10 | while read -r cnt repo; do
            echo "  $repo: $cnt expiry events"
        done) || true
        
        local total_repos
        total_repos=$(tail -n +5 "$output_file" | awk -F'\t' '{print $1}' | sort -u | wc -l)
        if [[ $total_repos -gt 10 ]]; then
            echo "  ... and $((total_repos - 10)) more repositories"
        fi
    fi
    
    # Explicit success exit
    exit 0
}

main "$@"
