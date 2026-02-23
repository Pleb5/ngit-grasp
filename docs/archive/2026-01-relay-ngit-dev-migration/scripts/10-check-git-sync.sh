#!/usr/bin/env bash
#
# 10-check-git-sync.sh - Compare state events to actual git data on disk
#
# PHASE 2 of the GRASP relay to ngit-grasp migration analysis pipeline.
# Compares kind 30618 state events against actual git refs on disk.
#
# USAGE:
#   ./10-check-git-sync.sh <state-events.json> <git-base-dir> <output-dir> [--categorize]
#
# EXAMPLES:
#   # Check source relay against source git data
#   ./10-check-git-sync.sh output/prod/raw/state-events.json /var/lib/grasp-relay/git output/prod
#
#   # Check target relay against target git data
#   ./10-check-git-sync.sh output/archive/raw/state-events.json /var/lib/ngit-grasp/git output/archive
#
#   # Check and categorize in one step (convenience mode)
#   ./10-check-git-sync.sh output/prod/raw/state-events.json /var/lib/grasp-relay/git output/prod --categorize
#
# INPUT:
#   state-events.json  - JSONL file from Phase 1 (01-fetch-events.sh)
#                        One kind 30618 event per line
#   git-base-dir       - Base directory containing git repos
#                        Structure: <git-base>/<npub>/<repo>.git/
#
# OUTPUT:
#   <output-dir>/git-sync-status.tsv - Tab-separated values:
#     repo<TAB>npub<TAB>state_refs<TAB>git_refs<TAB>matches<TAB>reason
#
#   With --categorize flag, also outputs:
#     <output-dir>/category1-complete-match.txt
#     <output-dir>/category2-empty-blank.txt
#     <output-dir>/category3-partial-match.txt
#     <output-dir>/category4-no-match.txt
#
# CATEGORIES:
#   1. Complete Match - All refs in state event match git data perfectly
#   2. Empty/Blank - No git data available (directory missing or empty)
#   3. Partial Match - Some refs match, some don't
#   4. No Match - Git data exists but commit hashes don't match
#
# PREREQUISITES:
#   - nak (for npub encoding) - https://github.com/fiatjaf/nak
#   - jq (for JSON parsing)
#   - Read access to git directories (may need sudo)
#
# RUNTIME: ~20 minutes on VPS (git operations are slow)
#
# NOTES:
#   - Must run on VPS with access to git directories
#   - Progress indicator updates every 10 events
#   - Handles packed refs (git show-ref) and loose refs
#
# SEE ALSO:
#   docs/how-to/migrate-to-ngit-grasp.md - Full migration guide
#   01-fetch-events.sh - Phase 1 script that produces input for this script
#   20-categorize.sh - Phase 3a script that consumes output from this script
#

set -euo pipefail

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

log_progress() {
    # Overwrite current line for progress updates
    echo -ne "\r${BLUE}[PROGRESS]${NC} $*" >&2
}

usage() {
    echo "Usage: $0 <state-events.json> <git-base-dir> <output-dir> [--categorize]"
    echo ""
    echo "Arguments:"
    echo "  state-events.json  JSONL file from Phase 1 (kind 30618 events)"
    echo "  git-base-dir       Base directory for git repos (e.g., /var/lib/grasp-relay/git)"
    echo "  output-dir         Directory to store output files"
    echo "  --categorize       Optional: also output category files (like Phase 3)"
    echo ""
    echo "Examples:"
    echo "  $0 output/prod/raw/state-events.json /var/lib/grasp-relay/git output/prod"
    echo "  $0 output/archive/raw/state-events.json /var/lib/ngit-grasp/git output/archive"
    echo ""
    echo "Output:"
    echo "  git-sync-status.tsv - TSV with: repo, npub, state_refs, git_refs, matches, reason"
    exit 1
}

# Check prerequisites
check_prerequisites() {
    local missing=0
    
    if ! command -v git &> /dev/null; then
        log_error "git not found. Install with your package manager."
        missing=1
    fi
    
    if ! command -v nak &> /dev/null; then
        log_error "nak not found. Install from: https://github.com/fiatjaf/nak"
        log_error "Or run: nix-shell -p nak jq --run \"$0 $*\""
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

# Convert hex pubkey to npub
# Args: $1=hex_pubkey
# Returns: npub string or empty on error
hex_to_npub() {
    local hex="$1"
    nak encode npub "$hex" 2>/dev/null || echo ""
}

# Count refs in state event (only refs/heads/)
# Args: $1=event_json
# Returns: count
count_state_refs() {
    local event="$1"
    echo "$event" | jq '[.tags[] | select(.[0] | startswith("refs/heads/"))] | length' 2>/dev/null || echo "0"
}

# Get git refs from disk
# Args: $1=git_dir
# Returns: count of refs/heads/ refs
count_git_refs() {
    local git_dir="$1"
    
    if [[ ! -d "$git_dir" ]]; then
        echo "0"
        return
    fi
    
    # Try git show-ref first (handles packed refs correctly)
    # Note: We capture output separately to avoid pipefail issues
    local count
    if count=$(git --git-dir="$git_dir" show-ref --heads 2>/dev/null | wc -l); then
        echo "$count" | tr -d ' '
        return
    fi
    
    # Fallback: count loose refs (when git is not available or fails)
    if [[ -d "$git_dir/refs/heads" ]]; then
        find "$git_dir/refs/heads" -type f 2>/dev/null | wc -l | tr -d ' '
    else
        echo "0"
    fi
}

# Get ref hash from git directory
# Args: $1=git_dir, $2=ref_path (e.g., refs/heads/main)
# Returns: commit hash or empty
get_git_ref_hash() {
    local git_dir="$1"
    local ref_path="$2"
    
    # Try git show-ref first (handles packed refs)
    local hash
    hash=$(git --git-dir="$git_dir" show-ref --hash "$ref_path" 2>/dev/null | head -1 || echo "")
    
    if [[ -n "$hash" ]]; then
        echo "$hash"
        return
    fi
    
    # Fallback: read loose ref file
    local ref_file="$git_dir/$ref_path"
    if [[ -f "$ref_file" ]]; then
        cat "$ref_file" 2>/dev/null | tr -d '\n' || echo ""
    else
        echo ""
    fi
}

# Compare state event refs to git refs
# Args: $1=event_json, $2=git_dir
# Returns: count of matching refs
count_matching_refs() {
    local event="$1"
    local git_dir="$2"
    local matching=0
    
    # Extract refs/heads/ tags and compare
    while IFS= read -r ref_tag; do
        [[ -z "$ref_tag" ]] && continue
        
        local ref_path expected_hash
        ref_path=$(echo "$ref_tag" | jq -r '.[0]' 2>/dev/null || echo "")
        expected_hash=$(echo "$ref_tag" | jq -r '.[1]' 2>/dev/null || echo "")
        
        # Skip if not a heads ref or hash is missing
        [[ ! "$ref_path" =~ ^refs/heads/ ]] && continue
        [[ -z "$expected_hash" || "$expected_hash" == "null" ]] && continue
        
        # Get actual hash from git
        local actual_hash
        actual_hash=$(get_git_ref_hash "$git_dir" "$ref_path")
        
        if [[ "$expected_hash" == "$actual_hash" ]]; then
            matching=$((matching + 1))
        fi
    done < <(echo "$event" | jq -c '.tags[] | select(.[0] | startswith("refs/heads/"))' 2>/dev/null)
    
    echo "$matching"
}

# Categorize a single entry
# Args: $1=state_refs, $2=git_refs, $3=matches, $4=reason
# Returns: category number (1-4)
categorize_entry() {
    local state_refs="$1"
    local git_refs="$2"
    local matches="$3"
    local reason="$4"
    
    # Category 2: Empty/Blank
    if [[ -n "$reason" ]] || [[ "$git_refs" -eq 0 ]]; then
        echo "2"
        return
    fi
    
    # Category 1: Complete Match
    if [[ "$state_refs" -gt 0 ]] && [[ "$state_refs" -eq "$git_refs" ]] && [[ "$matches" -eq "$state_refs" ]]; then
        echo "1"
        return
    fi
    
    # Category 4: No Match
    if [[ "$git_refs" -gt 0 ]] && [[ "$matches" -eq 0 ]]; then
        echo "4"
        return
    fi
    
    # Category 3: Partial Match (default for anything else with matches > 0)
    if [[ "$matches" -gt 0 ]]; then
        echo "3"
        return
    fi
    
    # Fallback to category 2
    echo "2"
}

# Format entry for category file
# Args: $1=repo, $2=npub, $3=state_refs, $4=git_refs, $5=matches, $6=reason
format_category_line() {
    local repo="$1"
    local npub="$2"
    local state_refs="$3"
    local git_refs="$4"
    local matches="$5"
    local reason="$6"
    
    if [[ -n "$reason" ]]; then
        echo "$repo | $npub | state_refs=$state_refs | git_refs=$git_refs | matches=$matches | reason=$reason"
    else
        echo "$repo | $npub | state_refs=$state_refs | git_refs=$git_refs | matches=$matches"
    fi
}

# Process a single state event
# Args: $1=event_json, $2=git_base
# Outputs: TSV line to stdout
process_event() {
    local event="$1"
    local git_base="$2"
    
    # Extract repository identifier (d tag)
    local identifier
    identifier=$(echo "$event" | jq -r '.tags[] | select(.[0] == "d") | .[1]' 2>/dev/null | head -1 || echo "")
    
    if [[ -z "$identifier" ]]; then
        return 1
    fi
    
    # Extract maintainer pubkey (hex)
    local hex_pubkey
    hex_pubkey=$(echo "$event" | jq -r '.pubkey' 2>/dev/null || echo "")
    
    if [[ -z "$hex_pubkey" ]]; then
        return 1
    fi
    
    # Convert to npub
    local npub
    npub=$(hex_to_npub "$hex_pubkey")
    
    if [[ -z "$npub" ]]; then
        return 1
    fi
    
    # Count state refs
    local state_refs
    state_refs=$(count_state_refs "$event")
    
    # Find git directory
    local git_dir="$git_base/${npub}/${identifier}.git"
    
    # Check git directory status
    local git_refs=0
    local matches=0
    local reason=""
    
    if [[ ! -d "$git_dir" ]]; then
        reason="no_git_dir"
    elif [[ ! -d "$git_dir/refs/heads" ]] && [[ ! -f "$git_dir/packed-refs" ]]; then
        reason="empty_refs"
    else
        git_refs=$(count_git_refs "$git_dir")
        
        if [[ "$git_refs" -eq 0 ]]; then
            reason="empty_refs"
        elif [[ "$state_refs" -eq 0 ]]; then
            reason="no_state_refs"
        else
            matches=$(count_matching_refs "$event" "$git_dir")
        fi
    fi
    
    # Output TSV line: repo, npub, state_refs, git_refs, matches, reason
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$identifier" "$npub" "$state_refs" "$git_refs" "$matches" "$reason"
}

# Main
main() {
    local do_categorize=0
    local args=()
    
    # Parse arguments
    for arg in "$@"; do
        if [[ "$arg" == "--categorize" ]]; then
            do_categorize=1
        else
            args+=("$arg")
        fi
    done
    
    if [[ ${#args[@]} -ne 3 ]]; then
        usage
    fi
    
    local state_events_file="${args[0]}"
    local git_base="${args[1]}"
    local output_dir="${args[2]}"
    
    # Validate inputs
    if [[ ! -f "$state_events_file" ]]; then
        log_error "State events file not found: $state_events_file"
        exit 1
    fi
    
    if [[ ! -d "$git_base" ]]; then
        log_error "Git base directory not found: $git_base"
        log_error "This script must run on the VPS with access to git directories."
        exit 1
    fi
    
    # Check read permissions
    if ! ls "$git_base" >/dev/null 2>&1; then
        log_error "Cannot read git base directory (permission denied): $git_base"
        log_error "Try running with sudo or grant read permissions."
        exit 1
    fi
    
    check_prerequisites
    
    log_info "=== Git State Synchronization Check ==="
    log_info "State events: $state_events_file"
    log_info "Git base: $git_base"
    log_info "Output: $output_dir"
    if [[ $do_categorize -eq 1 ]]; then
        log_info "Mode: TSV + categorization"
    else
        log_info "Mode: TSV only (use 20-categorize.sh for categories)"
    fi
    log_info "Started: $(date)"
    echo ""
    
    # Create output directory
    mkdir -p "$output_dir"
    
    # Output files
    local tsv_file="$output_dir/git-sync-status.tsv"
    
    # Initialize TSV with header
    echo -e "repo\tnpub\tstate_refs\tgit_refs\tmatches\treason" > "$tsv_file"
    
    # Initialize category files if categorizing
    local cat1="" cat2="" cat3="" cat4=""
    if [[ $do_categorize -eq 1 ]]; then
        cat1="$output_dir/category1-complete-match.txt"
        cat2="$output_dir/category2-empty-blank.txt"
        cat3="$output_dir/category3-partial-match.txt"
        cat4="$output_dir/category4-no-match.txt"
        > "$cat1"
        > "$cat2"
        > "$cat3"
        > "$cat4"
    fi
    
    # Count total events
    local total_events
    total_events=$(wc -l < "$state_events_file" | tr -d ' ')
    log_info "Processing $total_events state events..."
    echo ""
    
    # Process each event
    local count=0
    local processed=0
    local skipped=0
    local count_cat1=0 count_cat2=0 count_cat3=0 count_cat4=0
    local start_time
    start_time=$(date +%s)
    
    while IFS= read -r event; do
        count=$((count + 1))
        
        # Skip empty lines
        [[ -z "$event" ]] && continue
        
        # Process event
        local result
        if result=$(process_event "$event" "$git_base"); then
            processed=$((processed + 1))
            
            # Write to TSV (skip header line)
            echo "$result" >> "$tsv_file"
            
            # Categorize if requested
            if [[ $do_categorize -eq 1 ]]; then
                # Parse result
                IFS=$'\t' read -r repo npub state_refs git_refs matches reason <<< "$result"
                
                local category
                category=$(categorize_entry "$state_refs" "$git_refs" "$matches" "$reason")
                
                local cat_line
                cat_line=$(format_category_line "$repo" "$npub" "$state_refs" "$git_refs" "$matches" "$reason")
                
                case "$category" in
                    1) echo "$cat_line" >> "$cat1"; count_cat1=$((count_cat1 + 1)) ;;
                    2) echo "$cat_line" >> "$cat2"; count_cat2=$((count_cat2 + 1)) ;;
                    3) echo "$cat_line" >> "$cat3"; count_cat3=$((count_cat3 + 1)) ;;
                    4) echo "$cat_line" >> "$cat4"; count_cat4=$((count_cat4 + 1)) ;;
                esac
            fi
        else
            skipped=$((skipped + 1))
        fi
        
        # Progress indicator every 10 events
        if [[ $((count % 10)) -eq 0 ]]; then
            local elapsed=$(($(date +%s) - start_time))
            local rate=0
            if [[ $elapsed -gt 0 ]]; then
                rate=$((count / elapsed))
            fi
            local eta="?"
            if [[ $rate -gt 0 ]]; then
                eta=$(( (total_events - count) / rate ))
            fi
            log_progress "Processed $count/$total_events events (~${rate}/s, ETA: ${eta}s)..."
        fi
    done < "$state_events_file"
    
    # Clear progress line
    echo "" >&2
    
    local end_time
    end_time=$(date +%s)
    local duration=$((end_time - start_time))
    
    # Summary
    echo ""
    log_info "=== Analysis Complete ==="
    log_info "Finished: $(date)"
    log_info "Duration: ${duration}s"
    log_info "Processed: $processed events"
    if [[ $skipped -gt 0 ]]; then
        log_warn "Skipped: $skipped events (missing identifier or pubkey)"
    fi
    echo ""
    
    if [[ $do_categorize -eq 1 ]]; then
        # Calculate percentages
        local total=$((count_cat1 + count_cat2 + count_cat3 + count_cat4))
        local pct1=0 pct2=0 pct3=0 pct4=0
        if [[ $total -gt 0 ]]; then
            pct1=$(awk "BEGIN {printf \"%.1f\", ($count_cat1/$total)*100}")
            pct2=$(awk "BEGIN {printf \"%.1f\", ($count_cat2/$total)*100}")
            pct3=$(awk "BEGIN {printf \"%.1f\", ($count_cat3/$total)*100}")
            pct4=$(awk "BEGIN {printf \"%.1f\", ($count_cat4/$total)*100}")
        fi
        
        log_info "=== Category Summary ==="
        log_success "Category 1 (Complete Match): $count_cat1 ($pct1%)"
        log_warn "Category 2 (Empty/Blank): $count_cat2 ($pct2%)"
        log_warn "Category 3 (Partial Match): $count_cat3 ($pct3%)"
        log_error "Category 4 (No Match): $count_cat4 ($pct4%)"
        echo ""
        
        # Validation warning
        if [[ $count_cat2 -eq $total ]] && [[ $total -gt 0 ]]; then
            log_error "WARNING: 100% of repos categorized as Empty/Blank"
            log_error "This usually indicates a permission or path issue."
            echo ""
            log_info "Troubleshooting:"
            echo "  1. Verify git data exists: sudo ls -la $git_base | head -10"
            echo "  2. Check sample repo: sudo find $git_base -name '*.git' -type d | head -1"
            echo "  3. Re-run with sudo if not already using it"
            echo ""
        fi
    fi
    
    log_info "Output files:"
    echo "  $tsv_file"
    if [[ $do_categorize -eq 1 ]]; then
        echo "  $cat1"
        echo "  $cat2"
        echo "  $cat3"
        echo "  $cat4"
    else
        echo ""
        log_info "Next step: Run 20-categorize.sh to categorize results"
        echo "  ./20-categorize.sh $tsv_file $output_dir"
    fi
}

main "$@"
