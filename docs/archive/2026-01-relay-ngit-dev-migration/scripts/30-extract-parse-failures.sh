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
# ENRICHMENT:
#   The script automatically enriches parse failures with repo/npub information
#   by extracting from "Added rejected announcement" log entries which include
#   pubkey and identifier fields. Hex pubkeys are converted to npub format using
#   `nak encode npub <hex-pubkey>` if the nak tool is available.
#
# OUTPUT:
#   <output-dir>/parse-failures.txt
#
# OUTPUT FORMAT (TSV):
#   event_id<TAB>kind<TAB>reason<TAB>repo<TAB>npub
#
# EXPECTED LOG FORMATS:
#   The script looks for three types of log entries:
#
#   1. Structured [PARSE_FAIL] entries:
#      2026-01-22T10:30:45Z ngit-grasp[1234]: [PARSE_FAIL] kind=30618 event_id=abc123... reason="invalid refs format" repo=myrepo npub=npub1...
#
#   2. "Invalid announcement" rejections (write policy):
#      Event rejected by write policy event_id=abc123... relay=wss://... kind=30617 reason=Invalid announcement: multiple clone tags found...
#
#   3. "Added rejected announcement" entries (for enrichment):
#      Added rejected announcement to two-tier index event_id=abc123... kind=30617 identifier=myrepo pubkey=hex...
#      These entries provide pubkey and identifier for enriching write policy rejections.
#
#   NOTE: Builder logs ("Rejected repository announcement note1xxx:") are NOT extracted
#   because they use bech32 (note1) IDs while write policy logs use hex IDs. Extracting
#   both would cause double-counting since deduplication only works within each format.
#   Write policy logs contain the same events, so we don't lose any data.
#
#   Required fields: kind, event_id, reason
#   Enrichment fields: repo (identifier), npub (converted from hex pubkey)
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
    echo "  --dry-run             Show what would be extracted without writing"
    echo ""
    echo "Examples:"
    echo "  $0 ngit-grasp.service output/logs"
    echo "  $0 ngit-grasp.service output/logs --since '2026-01-01'"
    echo "  $0 ngit-grasp.service output/logs --since '2026-01-15' --until '2026-01-22'"
    echo ""
    echo "Expected log formats:"
    echo "  [PARSE_FAIL] kind=30618 event_id=abc123 reason=\"...\" repo=myrepo npub=npub1..."
    echo "  Event rejected by write policy event_id=abc123 ... kind=30617 reason=Invalid announcement: ..."
    echo ""
    echo "Enrichment:"
    echo "  Parse failures are automatically enriched with repo/npub from"
    echo "  'Added rejected announcement' log entries. Hex pubkeys are converted"
    echo "  to npub format using 'nak encode npub' if available."
    exit 1
}

# =============================================================================
# AWK-BASED BATCH PARSING FUNCTIONS
# =============================================================================
# These functions use awk for efficient batch processing instead of per-line
# grep calls. This provides ~400x speedup for large log files.
#
# NOTE: parse_builder_rejection_line() was removed to fix double-counting bug.
# Builder logs use bech32 (note1) IDs while write policy logs use hex IDs.
# Since deduplication only works within each format, extracting both caused
# the same event to be counted twice. Write policy logs contain the same
# events, so we don't lose any data by only extracting from that source.

# Parse [PARSE_FAIL] log lines in batch using awk
# Input: file containing log lines with [PARSE_FAIL]
# Output: TSV lines: event_id<TAB>kind<TAB>reason<TAB>repo<TAB>npub
parse_parse_fail_batch() {
    local input_file="$1"
    awk '
    {
        # Extract kind=VALUE
        kind = ""
        if (match($0, /kind=([0-9]+)/, m)) kind = m[1]
        
        # Extract event_id=VALUE (hex string)
        event_id = ""
        if (match($0, /event_id=([a-f0-9]+)/, m)) event_id = m[1]
        
        # Extract reason="VALUE" (quoted string)
        reason = ""
        if (match($0, /reason="([^"]*)"/, m)) reason = m[1]
        
        # Extract repo=VALUE (optional)
        repo = ""
        if (match($0, /repo=([^ ]+)/, m)) repo = m[1]
        
        # Extract npub=VALUE (optional)
        npub = ""
        if (match($0, /npub=([^ ]+)/, m)) npub = m[1]
        
        # Output if we have required fields
        if (kind != "" && event_id != "" && reason != "") {
            print event_id "\t" kind "\t" reason "\t" repo "\t" npub
        }
    }
    ' "$input_file"
}

# Parse "Invalid announcement" rejection log lines in batch using awk
# Input: file containing "Event rejected by write policy" log lines
# Output: TSV lines: event_id<TAB>kind<TAB>reason<TAB><empty><TAB><empty>
parse_write_policy_rejection_batch() {
    local input_file="$1"
    awk '
    {
        # Extract event_id=VALUE (hex string)
        event_id = ""
        if (match($0, /event_id=([a-f0-9]+)/, m)) event_id = m[1]
        
        # Extract kind=VALUE
        kind = ""
        if (match($0, /kind=([0-9]+)/, m)) kind = m[1]
        
        # Extract reason=VALUE (everything after "reason=")
        reason = ""
        if (match($0, /reason=(.*)$/, m)) reason = m[1]
        
        # Output if we have required fields (repo and npub are empty)
        if (kind != "" && event_id != "" && reason != "") {
            print event_id "\t" kind "\t" reason "\t\t"
        }
    }
    ' "$input_file"
}

# Parse "Added rejected announcement" log lines in batch using awk
# Input: file containing "Added rejected announcement to two-tier index" log lines
# Output: TSV lines: event_id<TAB>identifier<TAB>pubkey_hex
parse_rejected_announcement_batch() {
    local input_file="$1"
    awk '
    {
        # Extract event_id=VALUE (hex string)
        event_id = ""
        if (match($0, /event_id=([a-f0-9]+)/, m)) event_id = m[1]
        
        # Extract identifier=VALUE (repo name)
        identifier = ""
        if (match($0, /identifier=([^ ]+)/, m)) identifier = m[1]
        
        # Extract pubkey=VALUE (hex string)
        pubkey = ""
        if (match($0, /pubkey=([a-f0-9]+)/, m)) pubkey = m[1]
        
        # Output if we have all required fields
        if (event_id != "" && identifier != "" && pubkey != "") {
            print event_id "\t" identifier "\t" pubkey
        }
    }
    ' "$input_file"
}

# Enrich parse failures with repo/npub by looking up event_id in "Added rejected announcement" log entries
# This is critical because "Invalid announcement" rejections only log event_id and kind,
# not the repo name or npub. Without enrichment, Phase 5 shows event_id|kind instead
# of repo|npub in action-required.txt, making the output unusable.
#
# Arguments:
#   $1 - parse failures file to enrich (modified in place)
#   $2 - lookup file containing event_id -> identifier|pubkey mappings from logs
#
# The function:
#   1. Uses the lookup table built from "Added rejected announcement" log entries
#   2. For each parse failure with empty repo/npub, looks up the event_id
#   3. Populates repo and npub columns from the lookup
#   4. Converts hex pubkeys to npub format using `nak encode npub` if available
#
# OPTIMIZATION: This function uses batch processing for efficiency:
#   - Uses awk for O(n) join instead of per-line grep (O(n*m))
#   - Batches all pubkey->npub conversions in a single nak call
#   - This reduces runtime from minutes to seconds for large datasets
enrich_with_repo_npub() {
    local parse_failures_file="$1"
    local lookup_file="$2"
    
    # Validate lookup file exists and has content
    if [[ ! -f "$lookup_file" ]] || [[ ! -s "$lookup_file" ]]; then
        log_warn "No enrichment data available - repo/npub columns will remain empty"
        return 0
    fi
    
    log_info "Enriching parse failures with repo/npub from log entries..."
    
    # Check if we have nak for pubkey->npub conversion
    local can_convert_npub=false
    if command -v nak &> /dev/null; then
        can_convert_npub=true
        log_info "  Using 'nak' for pubkey->npub conversion"
    else
        log_warn "  'nak' not found - will use hex pubkeys instead of npub"
    fi
    
    local lookup_count
    lookup_count=$(wc -l < "$lookup_file")
    lookup_count="${lookup_count//[^0-9]/}"
    log_info "  Lookup table has $lookup_count entries"
    
    # STEP 1: Extract unique pubkeys that need conversion
    # Get pubkeys from lookup file (column 3), deduplicate
    local unique_pubkeys_file npub_map_file
    unique_pubkeys_file=$(mktemp)
    npub_map_file=$(mktemp)
    
    cut -f3 "$lookup_file" | sort -u > "$unique_pubkeys_file"
    local unique_pubkey_count
    unique_pubkey_count=$(wc -l < "$unique_pubkeys_file")
    unique_pubkey_count="${unique_pubkey_count//[^0-9]/}"
    log_info "  Converting $unique_pubkey_count unique pubkeys to npub format..."
    
    # STEP 2: Batch convert all pubkeys to npub in a single nak call
    # nak reads hex pubkeys from stdin (one per line) and outputs npubs
    if [[ "$can_convert_npub" == true && "$unique_pubkey_count" -gt 0 ]]; then
        # Create mapping file: pubkey_hex<TAB>npub
        # nak encode npub reads from stdin and outputs one npub per line
        paste "$unique_pubkeys_file" <(nak encode npub < "$unique_pubkeys_file" 2>/dev/null) > "$npub_map_file" || {
            # Fallback: if batch conversion fails, use hex pubkeys
            log_warn "  Batch npub conversion failed, using hex pubkeys"
            awk '{print $1 "\t" $1}' "$unique_pubkeys_file" > "$npub_map_file"
        }
    else
        # No nak available, use hex pubkeys as-is
        awk '{print $1 "\t" $1}' "$unique_pubkeys_file" > "$npub_map_file"
    fi
    
    rm -f "$unique_pubkeys_file"
    
    # STEP 3: Use awk for efficient join (O(n) instead of O(n*m) grep per line)
    # This joins parse_failures with lookup_file on event_id, then with npub_map on pubkey
    local enriched_file
    enriched_file=$(mktemp)
    
    # Copy header lines
    grep '^#' "$parse_failures_file" > "$enriched_file" 2>/dev/null || true
    
    # Use awk to perform the join efficiently
    # Input files (order matters for ARGIND):
    #   1. npub_map_file: pubkey_hex<TAB>npub
    #   2. lookup_file: event_id<TAB>identifier<TAB>pubkey_hex
    #   3. parse_failures_file: event_id<TAB>kind<TAB>reason<TAB>repo<TAB>npub
    awk -F'\t' -v OFS='\t' '
        # Track which file we are processing
        FNR==1 { file_num++ }
        
        # First file: npub_map (pubkey_hex -> npub)
        file_num==1 {
            npub_map[$1] = $2
            next
        }
        # Second file: lookup (event_id -> identifier, pubkey_hex)
        file_num==2 {
            lookup_repo[$1] = $2
            lookup_pubkey[$1] = $3
            next
        }
        # Third file: parse_failures
        /^#/ { next }  # Skip headers (already copied)
        {
            event_id = $1
            kind = $2
            reason = $3
            repo = $4
            npub = $5
            
            # If repo/npub empty, try to enrich from lookup
            if (repo == "" && event_id in lookup_repo) {
                repo = lookup_repo[event_id]
            }
            if (npub == "" && event_id in lookup_pubkey) {
                pubkey = lookup_pubkey[event_id]
                if (pubkey in npub_map) {
                    npub = npub_map[pubkey]
                } else {
                    npub = pubkey  # Fallback to hex
                }
            }
            
            print event_id, kind, reason, repo, npub
        }
    ' "$npub_map_file" "$lookup_file" "$parse_failures_file" >> "$enriched_file"
    
    rm -f "$npub_map_file"
    
    # Count enriched entries
    local enriched_count total_count
    total_count=$(grep -v '^#' "$parse_failures_file" | wc -l)
    total_count="${total_count//[^0-9]/}"
    # Count entries that have non-empty repo AND npub after enrichment
    enriched_count=$(grep -v '^#' "$enriched_file" | awk -F'\t' '$4 != "" && $5 != ""' | wc -l)
    enriched_count="${enriched_count//[^0-9]/}"
    
    # Replace original with enriched version
    mv "$enriched_file" "$parse_failures_file"
    
    log_info "  Enriched $enriched_count of $total_count parse failures with repo/npub"
    log_success "Enrichment complete"
}

# Parse "Added rejected announcement" log entries to build enrichment lookup table
# Input: log line containing "Added rejected announcement to two-tier index"
# Output: TSV line: event_id<TAB>identifier<TAB>pubkey_hex
parse_rejected_announcement_line() {
    local line="$1"
    
    local event_id identifier pubkey_hex
    
    # Extract event_id=VALUE (hex string)
    event_id=$(echo "$line" | grep -oP 'event_id=\K[a-f0-9]+' || echo "")
    
    # Extract identifier=VALUE (repo name)
    identifier=$(echo "$line" | grep -oP 'identifier=\K[^ ]+' || echo "")
    
    # Extract pubkey=VALUE (hex string)
    pubkey_hex=$(echo "$line" | grep -oP 'pubkey=\K[a-f0-9]+' || echo "")
    
    # Only output if we have all required fields
    if [[ -n "$event_id" && -n "$identifier" && -n "$pubkey_hex" ]]; then
        printf '%s\t%s\t%s\n' "$event_id" "$identifier" "$pubkey_hex"
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
    local temp_stderr temp_parse_fail temp_write_policy_rejection temp_rejected_announcement
    temp_stderr=$(mktemp)
    temp_parse_fail=$(mktemp)
    temp_write_policy_rejection=$(mktemp)
    temp_rejected_announcement=$(mktemp)
    
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
    
    # Extract "Added rejected announcement" entries for enrichment (streaming)
    # These contain pubkey and identifier which we use to enrich write policy rejections
    log_info "  Searching for rejected announcement entries (for enrichment)..."
    eval "$journal_cmd" 2>/dev/null | grep 'Added rejected announcement to two-tier index' > "$temp_rejected_announcement" || true
    
    rm -f "$temp_stderr"
    
    # Check if we found anything
    local parse_fail_line_count write_policy_line_count rejected_announcement_line_count
    parse_fail_line_count=$(wc -l < "$temp_parse_fail")
    parse_fail_line_count="${parse_fail_line_count//[^0-9]/}"
    write_policy_line_count=$(wc -l < "$temp_write_policy_rejection")
    write_policy_line_count="${write_policy_line_count//[^0-9]/}"
    rejected_announcement_line_count=$(wc -l < "$temp_rejected_announcement")
    rejected_announcement_line_count="${rejected_announcement_line_count//[^0-9]/}"
    
    log_info "  Found $parse_fail_line_count [PARSE_FAIL] log lines"
    log_info "  Found $write_policy_line_count write policy rejection log lines"
    log_info "  Found $rejected_announcement_line_count rejected announcement log lines (for enrichment)"
    
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
        
        rm -f "$temp_parse_fail" "$temp_write_policy_rejection" "$temp_rejected_announcement"
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
    
    # Parse [PARSE_FAIL] entries using batch awk processing
    log_info "  Parsing [PARSE_FAIL] entries..."
    local parse_fail_count=0
    if [[ "$parse_fail_line_count" -gt 0 ]]; then
        parse_parse_fail_batch "$temp_parse_fail" >> "$output_file"
        parse_fail_count=$(grep -v '^#' "$output_file" | wc -l)
        parse_fail_count="${parse_fail_count//[^0-9]/}"
    fi
    
    # Parse write policy rejection entries using batch awk processing
    log_info "  Parsing write policy rejection entries..."
    local write_policy_count=0
    if [[ "$write_policy_line_count" -gt 0 ]]; then
        local before_count
        before_count=$(grep -v '^#' "$output_file" 2>/dev/null | wc -l || echo "0")
        before_count="${before_count//[^0-9]/}"
        before_count="${before_count:-0}"
        parse_write_policy_rejection_batch "$temp_write_policy_rejection" >> "$output_file"
        local after_count
        after_count=$(grep -v '^#' "$output_file" 2>/dev/null | wc -l || echo "0")
        after_count="${after_count//[^0-9]/}"
        after_count="${after_count:-0}"
        write_policy_count=$((after_count - before_count))
    fi
    
    local invalid_announcement_count=$write_policy_count
    
    # Build enrichment lookup table from "Added rejected announcement" entries
    local enrichment_lookup_file
    enrichment_lookup_file=$(mktemp)
    
    log_info "  Building enrichment lookup table..."
    if [[ "$rejected_announcement_line_count" -gt 0 ]]; then
        parse_rejected_announcement_batch "$temp_rejected_announcement" > "$enrichment_lookup_file"
    fi
    
    rm -f "$temp_parse_fail" "$temp_write_policy_rejection" "$temp_rejected_announcement"
    
    # Deduplicate by event_id (first column) - keep first occurrence
    log_info "  Deduplicating entries..."
    local deduped_file
    deduped_file=$(mktemp)
    # Preserve header lines (starting with #) and deduplicate data lines
    grep '^#' "$output_file" > "$deduped_file"
    grep -v '^#' "$output_file" | sort -t$'\t' -k1,1 -u >> "$deduped_file"
    mv "$deduped_file" "$output_file"
    
    # Deduplicate enrichment lookup table by event_id
    if [[ -s "$enrichment_lookup_file" ]]; then
        sort -t$'\t' -k1,1 -u "$enrichment_lookup_file" > "$enrichment_lookup_file.deduped"
        mv "$enrichment_lookup_file.deduped" "$enrichment_lookup_file"
    fi
    
    # Enrich with repo/npub from "Added rejected announcement" log entries
    # This is critical for usability - without it, action-required.txt shows
    # event_id|kind instead of repo|npub, making parse failures unidentifiable
    enrich_with_repo_npub "$output_file" "$enrichment_lookup_file"
    
    rm -f "$enrichment_lookup_file"
    
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
    log_success "Extracted $count total entries"
    log_info "  - [PARSE_FAIL] entries: $parse_fail_count"
    log_info "  - Invalid announcement rejections: $invalid_announcement_count"
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
