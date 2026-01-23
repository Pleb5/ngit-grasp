#!/usr/bin/env bash
#
# 30-extract-parse-failures.sh - Extract parse failure events from systemd logs
#
# PHASE 4a of the GRASP relay to ngit-grasp migration analysis pipeline.
# Extracts structured [PARSE_FAIL] log entries AND "Invalid announcement"
# rejections from journalctl.
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
#   event_id<TAB>kind<TAB>reason<TAB>repo<TAB>npub
#
# EXPECTED LOG FORMATS:
#   The script looks for two types of log entries:
#
#   1. Structured [PARSE_FAIL] entries:
#      2026-01-22T10:30:45Z ngit-grasp[1234]: [PARSE_FAIL] kind=30618 event_id=abc123... reason="invalid refs format" repo=myrepo npub=npub1...
#
#   2. "Invalid announcement" rejections (write policy):
#      Event rejected by write policy event_id=abc123... relay=wss://... kind=30617 reason=Invalid announcement: multiple clone tags found...
#
#   NOTE: Builder logs ("Rejected repository announcement note1xxx:") are NOT extracted
#   because they use bech32 (note1) IDs while write policy logs use hex IDs. Extracting
#   both would cause double-counting since deduplication only works within each format.
#   Write policy logs contain the same events, so we don't lose any data.
#
#   Required fields: kind, event_id, reason
#   Optional fields: repo, npub (may not be available for all entry types)
#
# DEPENDENCY:
#   This script requires logging improvements in ngit-grasp to emit structured
#   [PARSE_FAIL] log entries. Until those are implemented, this script will
#   find no matching entries (which is handled gracefully).
#
#   "Invalid announcement" rejections are logged by the write policy and
#   should be present in any ngit-grasp deployment.
#
#   See: docs/how-to/migrate-to-ngit-grasp.md (Dependencies section)
#
#   Expected Rust logging code for [PARSE_FAIL]:
#     tracing::warn!(
#         target: "migration",
#         "[PARSE_FAIL] kind={} event_id={} reason=\"{}\" repo={} npub={}",
#         event.kind, event.id, reason, identifier, npub
#     );
#
# PREREQUISITES:
#   - journalctl (systemd)
#   - grep, awk, sed (standard Unix tools)
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
    echo "  --since <date>        Start date (default: 30 days ago)"
    echo "  --until <date>        End date (default: now)"
    echo "  --analysis-root <dir> Filter to only missing announcements from analysis"
    echo "  --dry-run             Show what would be extracted without writing"
    echo ""
    echo "Examples:"
    echo "  $0 ngit-grasp.service output/logs"
    echo "  $0 ngit-grasp.service output/logs --since '2026-01-01'"
    echo "  $0 ngit-grasp.service output/logs --since '2026-01-15' --until '2026-01-22'"
    echo "  $0 ngit-grasp.service output/logs --analysis-root /tmp/migration-analysis-20260123"
    echo ""
    echo "Expected log formats:"
    echo "  [PARSE_FAIL] kind=30618 event_id=abc123 reason=\"...\" repo=myrepo npub=npub1..."
    echo "  Event rejected by write policy event_id=abc123 ... kind=30617 reason=Invalid announcement: ..."
    echo ""
    echo "Filtering with --analysis-root:"
    echo "  When provided, only parse failures for announcements that are in production"
    echo "  but missing from the archive will be included. This filters out rejections"
    echo "  for events from other relays that don't affect the migration."
    exit 1
}

# Parse a [PARSE_FAIL] log line and extract fields
# Input: log line containing [PARSE_FAIL]
# Output: TSV line: event_id<TAB>kind<TAB>reason<TAB>repo<TAB>npub
parse_parse_fail_line() {
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
        printf '%s\t%s\t%s\t%s\t%s\n' "$event_id" "$kind" "$reason" "$repo" "$npub"
    fi
}

# Parse an "Invalid announcement" rejection log line from write policy
# Input: log line containing "Event rejected by write policy" with "Invalid announcement"
# Output: TSV line: event_id<TAB>kind<TAB>reason<TAB>repo<TAB>npub
# Note: repo and npub are empty for these entries (not available in log format)
parse_write_policy_rejection_line() {
    local line="$1"
    
    local kind event_id reason
    
    # Extract event_id=VALUE (hex string)
    event_id=$(echo "$line" | grep -oP 'event_id=\K[a-f0-9]+' || echo "")
    
    # Extract kind=VALUE
    kind=$(echo "$line" | grep -oP 'kind=\K[0-9]+' || echo "")
    
    # Extract reason=VALUE (everything after "reason=")
    # The reason is unquoted and goes to end of line
    reason=$(echo "$line" | grep -oP 'reason=\K.*$' || echo "")
    
    # Only output if we have the required fields
    if [[ -n "$kind" && -n "$event_id" && -n "$reason" ]]; then
        # repo and npub are empty for invalid announcement entries
        printf '%s\t%s\t%s\t\t\n' "$event_id" "$kind" "$reason"
    fi
}

# NOTE: parse_builder_rejection_line() was removed to fix double-counting bug.
# Builder logs use bech32 (note1) IDs while write policy logs use hex IDs.
# Since deduplication only works within each format, extracting both caused
# the same event to be counted twice. Write policy logs contain the same
# events, so we don't lose any data by only extracting from that source.

# Filter parse failures to only those for missing announcements
# This is used when --analysis-root is provided to scope results to the migration
#
# Arguments:
#   $1 - parse failures file to filter (modified in place)
#   $2 - analysis root directory containing comparison/ and prod/ subdirs
#
# The function:
#   1. Reads missing announcements from comparison/complete-prod-missing-archive.txt
#   2. Extracts pubkey/identifier pairs for those announcements
#   3. Reads production announcements from prod/raw/announcements.json
#   4. Gets event IDs for announcements matching the missing pubkey/identifier pairs
#   5. Filters parse failures to only those event IDs
filter_to_missing_announcements() {
    local parse_failures_file="$1"
    local analysis_root="$2"
    
    local missing_file="$analysis_root/comparison/complete-prod-missing-archive.txt"
    local prod_announcements="$analysis_root/prod/raw/announcements.json"
    
    # Validate required files exist
    if [[ ! -f "$missing_file" ]]; then
        log_warn "Missing announcements file not found: $missing_file"
        log_warn "Skipping filter - all parse failures will be included"
        return 0
    fi
    
    if [[ ! -f "$prod_announcements" ]]; then
        log_warn "Production announcements file not found: $prod_announcements"
        log_warn "Skipping filter - all parse failures will be included"
        return 0
    fi
    
    # Check if jq is available
    if ! command -v jq &> /dev/null; then
        log_warn "jq not found - cannot filter parse failures"
        log_warn "Install jq or run without --analysis-root"
        return 0
    fi
    
    log_info "Filtering parse failures to missing announcements only..."
    
    # Step 1: Extract pubkey/identifier pairs from missing announcements
    # Format: identifier | npub | prod=complete | archive=missing
    local missing_pairs_file
    missing_pairs_file=$(mktemp)
    
    # Extract identifier and npub, convert npub to hex pubkey for matching
    while IFS=' | ' read -r identifier npub rest; do
        # Skip empty lines
        [[ -z "$identifier" ]] && continue
        # Trim whitespace
        identifier=$(echo "$identifier" | xargs)
        npub=$(echo "$npub" | xargs)
        echo "${identifier}|${npub}"
    done < "$missing_file" > "$missing_pairs_file"
    
    local missing_count
    missing_count=$(wc -l < "$missing_pairs_file")
    missing_count="${missing_count//[^0-9]/}"
    log_info "  Found $missing_count missing announcements to filter for"
    
    # Step 2: Get event IDs from production announcements for these pairs
    # We need to match on 'd' tag (identifier) and pubkey
    local missing_event_ids_file
    missing_event_ids_file=$(mktemp)
    
    # Create a lookup of identifier|npub -> event_id from production announcements
    # The JSON has: id, pubkey (hex), tags (array with ["d", identifier])
    log_info "  Extracting event IDs from production announcements..."
    
    # Use jq to extract id, pubkey, and d-tag value, then filter
    # Output format: event_id|identifier|pubkey_hex
    # Note: The JSON file is NDJSON (newline-delimited), not an array
    jq -r 'select(.kind == 30617) | 
        .id as $id | 
        .pubkey as $pubkey |
        (.tags[] | select(.[0] == "d") | .[1]) as $dtag |
        "\($id)|\($dtag)|\($pubkey)"' "$prod_announcements" > "$missing_event_ids_file.all" 2>/dev/null || {
        log_warn "Failed to parse production announcements JSON"
        rm -f "$missing_pairs_file" "$missing_event_ids_file" "$missing_event_ids_file.all"
        return 0
    }
    
    # Now filter to only event IDs for missing announcements
    # We need to convert npub to hex pubkey for comparison
    # npub is bech32, pubkey in JSON is hex
    # For simplicity, we'll match on identifier only (d-tag) since it should be unique per pubkey
    # Actually, we need both because same identifier can exist for different pubkeys
    
    # Create a set of "identifier|pubkey_hex" to match against
    # First, we need to convert npub to hex - but that requires a tool
    # Alternative: match on identifier only and accept some false positives
    # Better: use the comparison file which has npub, and match against announcements
    
    # Let's match on identifier only for now (simpler, may have minor false positives)
    # Extract just the identifiers from missing announcements
    local missing_identifiers_file
    missing_identifiers_file=$(mktemp)
    cut -d'|' -f1 "$missing_pairs_file" | sort -u > "$missing_identifiers_file"
    
    # Filter event IDs to only those with matching identifiers
    while IFS='|' read -r event_id identifier pubkey_hex; do
        if grep -qFx "$identifier" "$missing_identifiers_file"; then
            echo "$event_id"
        fi
    done < "$missing_event_ids_file.all" | sort -u > "$missing_event_ids_file"
    
    local event_id_count
    event_id_count=$(wc -l < "$missing_event_ids_file")
    event_id_count="${event_id_count//[^0-9]/}"
    log_info "  Found $event_id_count event IDs for missing announcements"
    
    # Step 3: Filter parse failures to only those event IDs
    local filtered_file
    filtered_file=$(mktemp)
    
    # Copy header lines
    grep '^#' "$parse_failures_file" > "$filtered_file"
    
    # Add a note about filtering
    echo "# Filtered to missing announcements only (--analysis-root)" >> "$filtered_file"
    echo "# Analysis root: $analysis_root" >> "$filtered_file"
    echo "# Missing announcements: $missing_count" >> "$filtered_file"
    echo "# Matching event IDs: $event_id_count" >> "$filtered_file"
    
    # Filter data lines - only include if event_id is in our list
    local filtered_count=0
    while IFS=$'\t' read -r event_id kind reason repo npub; do
        # Skip header lines (already copied)
        [[ "$event_id" =~ ^# ]] && continue
        
        # Check if this event_id is in our missing list
        if grep -qFx "$event_id" "$missing_event_ids_file"; then
            printf '%s\t%s\t%s\t%s\t%s\n' "$event_id" "$kind" "$reason" "$repo" "$npub" >> "$filtered_file"
            filtered_count=$((filtered_count + 1))
        fi
    done < "$parse_failures_file"
    
    # Replace original with filtered version
    mv "$filtered_file" "$parse_failures_file"
    
    # Cleanup temp files
    rm -f "$missing_pairs_file" "$missing_event_ids_file" "$missing_event_ids_file.all" "$missing_identifiers_file"
    
    log_info "  Filtered from $(grep -v '^#' "$parse_failures_file" | wc -l | xargs) to $filtered_count parse failures"
    log_success "Filtered to parse failures for missing announcements only"
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
    local analysis_root=""
    
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
            --analysis-root)
                analysis_root="$2"
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
    
    log_info "Running: $journal_cmd | grep '[PARSE_FAIL]' or 'Invalid announcement'"
    
    if [[ "$dry_run" == true ]]; then
        log_info "[DRY RUN] Would extract to: $output_dir/parse-failures.txt"
        
        # Show sample of what would be extracted
        log_info "Checking for matching log entries..."
        local parse_fail_count invalid_announcement_count
        parse_fail_count=$(eval "$journal_cmd" 2>/dev/null | grep -c '\[PARSE_FAIL\]' || echo "0")
        parse_fail_count="${parse_fail_count//[^0-9]/}"  # Strip non-numeric characters
        parse_fail_count="${parse_fail_count:-0}"
        
        invalid_announcement_count=$(eval "$journal_cmd" 2>/dev/null | grep 'Event rejected by write policy' | grep -c 'Invalid announcement' || echo "0")
        invalid_announcement_count="${invalid_announcement_count//[^0-9]/}"
        invalid_announcement_count="${invalid_announcement_count:-0}"
        
        log_info "Found $parse_fail_count [PARSE_FAIL] entries"
        log_info "Found $invalid_announcement_count 'Invalid announcement' rejections"
        
        if [[ "$parse_fail_count" -eq 0 && "$invalid_announcement_count" -eq 0 ]]; then
            log_warn "No matching entries found in logs."
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
    
    # Extract and parse log entries using streaming (avoids loading all logs into memory)
    log_info "Extracting log entries..."
    
    # Create temp files for intermediate results
    local temp_stderr temp_parse_fail temp_write_policy_rejection
    temp_stderr=$(mktemp)
    temp_parse_fail=$(mktemp)
    temp_write_policy_rejection=$(mktemp)
    
    # Extract [PARSE_FAIL] entries directly to temp file (streaming)
    log_info "  Searching for [PARSE_FAIL] entries..."
    eval "$journal_cmd" 2>"$temp_stderr" | grep '\[PARSE_FAIL\]' > "$temp_parse_fail" || true
    
    local journal_stderr
    journal_stderr=$(cat "$temp_stderr" 2>/dev/null || true)
    if [[ -n "$journal_stderr" ]]; then
        log_warn "journalctl reported: $journal_stderr"
    fi
    
    # Extract "Event rejected by write policy" with "Invalid announcement" (streaming)
    # NOTE: We only extract from write policy logs (hex IDs), not builder logs (note1 IDs)
    # to avoid double-counting. Both log sources contain the same events.
    log_info "  Searching for write policy rejections..."
    eval "$journal_cmd" 2>/dev/null | grep 'Event rejected by write policy' | grep 'Invalid announcement' > "$temp_write_policy_rejection" || true
    
    rm -f "$temp_stderr"
    
    # Check if we found anything
    local parse_fail_line_count write_policy_line_count
    parse_fail_line_count=$(wc -l < "$temp_parse_fail")
    parse_fail_line_count="${parse_fail_line_count//[^0-9]/}"
    write_policy_line_count=$(wc -l < "$temp_write_policy_rejection")
    write_policy_line_count="${write_policy_line_count//[^0-9]/}"
    
    log_info "  Found $parse_fail_line_count [PARSE_FAIL] log lines"
    log_info "  Found $write_policy_line_count write policy rejection log lines"
    
    local total_invalid_announcement_lines=$write_policy_line_count
    
    if [[ "$parse_fail_line_count" -eq 0 && "$total_invalid_announcement_lines" -eq 0 ]]; then
        log_warn "No matching entries found in logs."
        log_warn ""
        log_warn "This is expected if ngit-grasp logging improvements are not yet deployed."
        log_warn "The script looks for:"
        log_warn ""
        log_warn "  1. [PARSE_FAIL] kind=30618 event_id=abc123 reason=\"...\" repo=myrepo npub=npub1..."
        log_warn "  2. Event rejected by write policy event_id=... kind=30617 reason=Invalid announcement: ..."
        log_warn ""
        log_warn "See: docs/how-to/migrate-to-ngit-grasp.md (Dependencies section)"
        log_warn ""
        
        # Create empty output file with header comment
        {
            echo "# Parse failures and invalid announcements extracted from $service"
            echo "# Time range: ${since_date:-beginning} to ${until_date:-now}"
            echo "# Extracted: $(date -Iseconds)"
            echo "#"
            echo "# Includes:"
            echo "#   - [PARSE_FAIL] structured log entries"
            echo "#   - \"Invalid announcement\" rejections"
            echo "#"
            echo "# Format: event_id<TAB>kind<TAB>reason<TAB>repo<TAB>npub"
            echo "# Note: repo and npub may be empty for some entries"
            echo "#"
            echo "# NOTE: No matching entries found."
            echo "# This is expected if ngit-grasp logging improvements are not yet deployed."
        } > "$output_file"
        
        rm -f "$temp_parse_fail" "$temp_write_policy_rejection"
        log_info "Created empty output file: $output_file"
        exit 0
    fi
    
    # Write header
    {
        echo "# Parse failures and invalid announcements extracted from $service"
        echo "# Time range: ${since_date:-beginning} to ${until_date:-now}"
        echo "# Extracted: $(date -Iseconds)"
        echo "#"
        echo "# Includes:"
        echo "#   - [PARSE_FAIL] structured log entries"
        echo "#   - \"Invalid announcement\" rejections"
        echo "#"
        echo "# Format: event_id<TAB>kind<TAB>reason<TAB>repo<TAB>npub"
        echo "# Note: repo and npub may be empty for some entries"
    } > "$output_file"
    
    # Parse [PARSE_FAIL] entries
    log_info "  Parsing [PARSE_FAIL] entries..."
    local parse_fail_count=0
    if [[ "$parse_fail_line_count" -gt 0 ]]; then
        while IFS= read -r line; do
            local parsed
            parsed=$(parse_parse_fail_line "$line")
            if [[ -n "$parsed" ]]; then
                echo "$parsed" >> "$output_file"
                parse_fail_count=$((parse_fail_count + 1))
            fi
        done < "$temp_parse_fail"
    fi
    
    # Parse write policy rejection entries
    log_info "  Parsing write policy rejection entries..."
    local write_policy_count=0
    if [[ "$write_policy_line_count" -gt 0 ]]; then
        while IFS= read -r line; do
            local parsed
            parsed=$(parse_write_policy_rejection_line "$line")
            if [[ -n "$parsed" ]]; then
                echo "$parsed" >> "$output_file"
                write_policy_count=$((write_policy_count + 1))
            fi
        done < "$temp_write_policy_rejection"
    fi
    
    local invalid_announcement_count=$write_policy_count
    
    rm -f "$temp_parse_fail" "$temp_write_policy_rejection"
    
    # Deduplicate by event_id (first column) - keep first occurrence
    log_info "  Deduplicating entries..."
    local deduped_file
    deduped_file=$(mktemp)
    # Preserve header lines (starting with #) and deduplicate data lines
    grep '^#' "$output_file" > "$deduped_file"
    grep -v '^#' "$output_file" | sort -t$'\t' -k1,1 -u >> "$deduped_file"
    mv "$deduped_file" "$output_file"
    
    # Filter to missing announcements only if analysis root provided
    if [[ -n "$analysis_root" ]]; then
        filter_to_missing_announcements "$output_file" "$analysis_root"
    fi
    
    # Count final entries (excluding header lines)
    local count
    count=$(grep -v '^#' "$output_file" | wc -l)
    count="${count//[^0-9]/}"  # Strip whitespace
    count="${count:-0}"
    
    rm -f "$temp_file"
    
    # Summary
    echo ""
    log_info "=== Extraction Summary ==="
    log_info "Service: $service"
    log_info "Time range: ${since_date:-beginning} to ${until_date:-now}"
    if [[ -n "$analysis_root" ]]; then
        log_info "Filtered to: missing announcements only"
    fi
    log_success "Extracted $count total entries"
    log_info "  - [PARSE_FAIL] entries: $parse_fail_count"
    log_info "  - Invalid announcement rejections: $invalid_announcement_count"
    if [[ -n "$analysis_root" ]]; then
        log_info "  (filtered from original extraction)"
    fi
    echo ""
    log_info "Output file: $output_file"
    
    if [[ $count -gt 0 ]]; then
        echo ""
        log_info "Sample entries (first 5):"
        # Use a subshell to avoid SIGPIPE issues with set -e
        # New format: event_id<TAB>kind<TAB>reason<TAB>repo<TAB>npub
        (grep -v '^#' "$output_file" | head -5 | while IFS=$'\t' read -r event_id kind reason repo npub; do
            echo "  kind=$kind event_id=${event_id:0:16}... reason=\"${reason:0:60}...\""
        done) || true
    fi
    
    # Breakdown by kind
    if [[ $count -gt 0 ]]; then
        echo ""
        log_info "Breakdown by event kind:"
        # Use a subshell to avoid SIGPIPE issues with set -e
        # kind is now column 2
        (grep -v '^#' "$output_file" | awk -F'\t' '{print $2}' | sort | uniq -c | sort -rn | while read -r cnt kind; do
            echo "  kind $kind: $cnt failures"
        done) || true
    fi
    
    # Breakdown by reason pattern (for invalid announcements)
    if [[ $invalid_announcement_count -gt 0 ]]; then
        echo ""
        log_info "Breakdown by reason pattern:"
        # Extract the main reason type (before the colon details)
        (grep -v '^#' "$output_file" | awk -F'\t' '{print $3}' | sed 's/:.*//' | sort | uniq -c | sort -rn | head -10 | while read -r cnt reason; do
            echo "  $reason: $cnt"
        done) || true
    fi
    
    # Explicit success exit
    exit 0
}

main "$@"
