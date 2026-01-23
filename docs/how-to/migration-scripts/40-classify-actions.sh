#!/usr/bin/env bash
#
# 40-classify-actions.sh - Final classification of repos for migration action
#
# PHASE 5 of the GRASP relay to ngit-grasp migration analysis pipeline.
# Combines all data sources from previous phases to produce actionable results.
#
# USAGE:
#   ./40-classify-actions.sh <analysis-dir>
#
# EXAMPLES:
#   ./40-classify-actions.sh work/migration-analysis-20260122-1430
#
# INPUT DIRECTORY STRUCTURE:
#   <analysis-dir>/
#   ├── prod/
#   │   ├── raw/
#   │   │   └── deletions.json          # Phase 1: kind 5 deletion events
#   │   ├── category1-complete-match.txt    # Phase 3: complete git sync
#   │   ├── category2-empty-blank.txt       # Phase 3: no git data
#   │   ├── category3-partial-match.txt     # Phase 3: partial git sync
#   │   └── category4-no-match.txt          # Phase 3: git exists, refs don't match
#   ├── archive/
#   │   ├── raw/
#   │   │   └── deletions.json
#   │   ├── category1-complete-match.txt
#   │   ├── category2-empty-blank.txt
#   │   ├── category3-partial-match.txt
#   │   └── category4-no-match.txt
#   ├── comparison/
#   │   ├── complete-in-both.txt            # Phase 3: no action needed
#   │   ├── complete-prod-missing-archive.txt   # Phase 3: needs investigation
#   │   ├── complete-prod-incomplete-archive.txt # Phase 3: sync in progress?
#   │   ├── incomplete-in-both.txt          # Phase 3: git incomplete
#   │   └── in-archive-not-prod.txt         # Phase 3: deleted or new
#   └── logs/
#       ├── parse-failures.txt              # Phase 4: events that failed to parse
#       └── purgatory-expired.txt           # Phase 4: repos that expired from purgatory
#
# OUTPUT:
#   <analysis-dir>/results/
#   ├── no-action-required.txt      # Repos that are fine as-is
#   ├── action-required.txt         # Repos needing intervention
#   ├── manual-investigation.txt    # Repos needing human review
#   └── summary.txt                 # Human-readable summary
#
# OUTPUT FORMATS:
#   no-action-required.txt:
#     repo | npub | reason
#
#   action-required.txt:
#     repo | npub | reason | suggested_action
#
#   manual-investigation.txt:
#     repo | npub | reason | context
#
# CLASSIFICATION LOGIC:
#
#   NO ACTION REQUIRED:
#   - Complete in both prod and archive (successfully migrated)
#   - Empty/blank in both (user never pushed any data)
#   - Deleted by user (kind 5 deletion event exists)
#   - In purgatory expiry logs (system already handled it)
#
#   ACTION REQUIRED:
#   - Complete in prod, missing from archive → Re-sync needed
#   - Complete in prod, incomplete in archive → Wait for sync or re-trigger
#   - Partial match in prod → Investigate why refs don't match
#   - No match (category 4) → Investigate git data corruption
#   - Parse failures → Fix event format or re-announce
#
#   MANUAL INVESTIGATION:
#   - Conflicting states (e.g., complete in prod but parse failure logged)
#   - In archive but not prod (deleted? or new announcement?)
#   - Multiple issues for same repo
#   - Unexpected state combinations
#
# PREREQUISITES:
#   - jq (for parsing JSON)
#   - awk, sort, comm (standard Unix tools)
#
# RUNTIME: < 5 seconds (local processing only)
#
# SEE ALSO:
#   docs/how-to/migrate-to-ngit-grasp.md - Full migration guide
#   01-fetch-events.sh - Phase 1 (fetch events)
#   10-check-git-sync.sh - Phase 2 (git sync check)
#   20-categorize.sh, 21-compare-relays.sh - Phase 3 (categorize and compare)
#   30-extract-parse-failures.sh, 31-extract-purgatory-expiry.sh - Phase 4 (logs)
#

set -euo pipefail

# Colors for output (disabled if not a terminal)
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    BOLD=''
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
    echo "Usage: $0 <analysis-dir>"
    echo ""
    echo "Arguments:"
    echo "  analysis-dir  Directory containing Phase 1-4 output"
    echo ""
    echo "Examples:"
    echo "  $0 work/migration-analysis-20260122-1430"
    echo ""
    echo "Required input structure:"
    echo "  <analysis-dir>/prod/category*.txt"
    echo "  <analysis-dir>/archive/category*.txt"
    echo "  <analysis-dir>/comparison/*.txt"
    echo "  <analysis-dir>/logs/*.txt (optional)"
    echo "  <analysis-dir>/prod/raw/deletions.json"
    echo ""
    echo "Output:"
    echo "  <analysis-dir>/results/no-action-required.txt"
    echo "  <analysis-dir>/results/action-required.txt"
    echo "  <analysis-dir>/results/manual-investigation.txt"
    echo "  <analysis-dir>/results/summary.txt"
    exit 1
}

# Extract repo|npub key from category line
# Input: "repo | npub | state_refs=N | ..."
# Output: "repo|npub"
extract_key() {
    awk -F' \\| ' '{print $1 "|" $2}'
}

# Extract repo from category line
# Input: "repo | npub | ..."
# Output: "repo"
extract_repo() {
    awk -F' \\| ' '{print $1}'
}

# Extract npub from category line
# Input: "repo | npub | ..."
# Output: "npub"
extract_npub() {
    awk -F' \\| ' '{print $2}'
}

# Check if a file exists and has content (ignoring comment lines)
file_has_content() {
    local file="$1"
    if [[ ! -f "$file" ]]; then
        return 1
    fi
    # Check for non-comment, non-empty lines
    grep -v '^#' "$file" 2>/dev/null | grep -q '.' 2>/dev/null
}

# Count non-comment lines in a file
count_lines() {
    local file="$1"
    if [[ ! -f "$file" ]]; then
        echo "0"
        return
    fi
    local count
    count=$(grep -v '^#' "$file" 2>/dev/null | grep -c '.' 2>/dev/null) || count=0
    # Ensure we return a clean integer
    echo "${count:-0}"
}

# Parse deletions.json to extract deleted repo identifiers
# Kind 5 events have "e" tags pointing to the deleted event
# We need to cross-reference with announcements to get repo/npub
# For now, we extract the pubkey and any "a" tags (addressable event references)
parse_deletions() {
    local deletions_file="$1"
    local output_file="$2"
    
    if [[ ! -f "$deletions_file" ]]; then
        touch "$output_file"
        return
    fi
    
    # Extract deletion targets from kind 5 events
    # Kind 5 events can reference:
    # - "e" tag: specific event ID
    # - "a" tag: addressable event (kind:pubkey:identifier)
    # For 30617 announcements, "a" tag format is: 30617:<pubkey>:<repo-identifier>
    jq -r '
        select(.kind == 5) |
        .pubkey as $pubkey |
        .tags[] |
        select(.[0] == "a") |
        .[1] |
        split(":") |
        select(.[0] == "30617") |
        "\(.[2])|\($pubkey)"
    ' "$deletions_file" 2>/dev/null | sort -u > "$output_file" || touch "$output_file"
}

# Build a lookup set from a file (repo|npub format)
# Returns keys one per line
build_key_set() {
    local file="$1"
    if [[ ! -f "$file" ]]; then
        return 0
    fi
    # Use || true to prevent pipefail from exiting on empty grep
    { grep -v '^#' "$file" 2>/dev/null || true; } | extract_key | sort -u
}

# Main classification logic
main() {
    if [[ $# -ne 1 ]]; then
        usage
    fi
    
    local analysis_dir="$1"
    
    # Validate input directory
    if [[ ! -d "$analysis_dir" ]]; then
        log_error "Analysis directory not found: $analysis_dir"
        exit 1
    fi
    
    # Check for required subdirectories
    local prod_dir="$analysis_dir/prod"
    local archive_dir="$analysis_dir/archive"
    local comparison_dir="$analysis_dir/comparison"
    local logs_dir="$analysis_dir/logs"
    local results_dir="$analysis_dir/results"
    
    for dir in "$prod_dir" "$archive_dir" "$comparison_dir"; do
        if [[ ! -d "$dir" ]]; then
            log_error "Required directory not found: $dir"
            log_error "Run Phases 1-3 first to generate input data."
            exit 1
        fi
    done
    
    # Check for required category files
    if [[ ! -f "$prod_dir/category1-complete-match.txt" ]]; then
        log_error "Missing category files in $prod_dir"
        log_error "Run Phase 3 (20-categorize.sh) first."
        exit 1
    fi
    
    log_info "Starting final classification"
    log_info "Analysis directory: $analysis_dir"
    
    # Create output directory
    mkdir -p "$results_dir"
    
    # Create temp directory for intermediate files
    local tmp_dir
    tmp_dir=$(mktemp -d)
    # shellcheck disable=SC2064
    trap "rm -rf '$tmp_dir'" EXIT
    
    # Initialize output files
    local no_action="$results_dir/no-action-required.txt"
    local action_req="$results_dir/action-required.txt"
    local manual_inv="$results_dir/manual-investigation.txt"
    local summary="$results_dir/summary.txt"
    
    # Write headers
    {
        echo "# No Action Required - Repos that are fine as-is"
        echo "# Generated: $(date -Iseconds)"
        echo "# Format: repo | npub | reason"
        echo "#"
    } > "$no_action"
    
    {
        echo "# Action Required - Repos needing intervention"
        echo "# Generated: $(date -Iseconds)"
        echo "# Format: repo | npub | reason | suggested_action"
        echo "#"
    } > "$action_req"
    
    {
        echo "# Manual Investigation Required - Repos needing human review"
        echo "# Generated: $(date -Iseconds)"
        echo "# Format: repo | npub | reason | context"
        echo "#"
    } > "$manual_inv"
    
    # =========================================================================
    # STEP 1: Parse deletion events
    # =========================================================================
    log_info "Parsing deletion events..."
    
    parse_deletions "$prod_dir/raw/deletions.json" "$tmp_dir/prod_deletions.txt"
    parse_deletions "$archive_dir/raw/deletions.json" "$tmp_dir/archive_deletions.txt"
    
    # Combine deletions (union of both)
    cat "$tmp_dir/prod_deletions.txt" "$tmp_dir/archive_deletions.txt" 2>/dev/null | sort -u > "$tmp_dir/all_deletions.txt"
    
    local deletion_count
    deletion_count=$(wc -l < "$tmp_dir/all_deletions.txt" | tr -d ' ')
    log_info "Found $deletion_count deletion requests"
    
    # =========================================================================
    # STEP 2: Parse log-based categories (Phase 4)
    # =========================================================================
    log_info "Parsing log-based categories..."
    
    # Parse failures: repo<TAB>npub<TAB>kind<TAB>event_id<TAB>reason
    if [[ -f "$logs_dir/parse-failures.txt" ]] && file_has_content "$logs_dir/parse-failures.txt"; then
        grep -v '^#' "$logs_dir/parse-failures.txt" | awk -F'\t' '{print $1 "|" $2}' | sort -u > "$tmp_dir/parse_failures.txt"
        log_info "Found $(wc -l < "$tmp_dir/parse_failures.txt" | tr -d ' ') parse failure entries"
    else
        touch "$tmp_dir/parse_failures.txt"
        log_info "No parse failures found (logs may be empty or not yet generated)"
    fi
    
    # Purgatory expired: repo<TAB>npub<TAB>timestamp<TAB>reason
    if [[ -f "$logs_dir/purgatory-expired.txt" ]] && file_has_content "$logs_dir/purgatory-expired.txt"; then
        grep -v '^#' "$logs_dir/purgatory-expired.txt" | awk -F'\t' '{print $1 "|" $2}' | sort -u > "$tmp_dir/purgatory_expired.txt"
        log_info "Found $(wc -l < "$tmp_dir/purgatory_expired.txt" | tr -d ' ') purgatory expiry entries"
    else
        touch "$tmp_dir/purgatory_expired.txt"
        log_info "No purgatory expiry entries found (logs may be empty or not yet generated)"
    fi
    
    # =========================================================================
    # STEP 3: Build lookup tables from category files
    # =========================================================================
    log_info "Building lookup tables..."
    
    # Build key sets for each category (prod)
    build_key_set "$prod_dir/category1-complete-match.txt" > "$tmp_dir/prod_cat1.txt"
    build_key_set "$prod_dir/category2-empty-blank.txt" > "$tmp_dir/prod_cat2.txt"
    build_key_set "$prod_dir/category3-partial-match.txt" > "$tmp_dir/prod_cat3.txt"
    build_key_set "$prod_dir/category4-no-match.txt" > "$tmp_dir/prod_cat4.txt"
    
    # Build key sets for each category (archive)
    build_key_set "$archive_dir/category1-complete-match.txt" > "$tmp_dir/archive_cat1.txt"
    build_key_set "$archive_dir/category2-empty-blank.txt" > "$tmp_dir/archive_cat2.txt"
    build_key_set "$archive_dir/category3-partial-match.txt" > "$tmp_dir/archive_cat3.txt"
    build_key_set "$archive_dir/category4-no-match.txt" > "$tmp_dir/archive_cat4.txt"
    
    # All repos in prod
    cat "$tmp_dir"/prod_cat*.txt 2>/dev/null | sort -u > "$tmp_dir/all_prod.txt" || true
    
    # All repos in archive
    cat "$tmp_dir"/archive_cat*.txt 2>/dev/null | sort -u > "$tmp_dir/all_archive.txt" || true
    
    # =========================================================================
    # STEP 4: Process comparison files and apply classification
    # =========================================================================
    log_info "Applying classification logic..."
    
    # Track processed repos to detect duplicates/conflicts
    > "$tmp_dir/processed.txt"
    
    # Counters
    local count_no_action=0
    local count_action=0
    local count_manual=0
    
    # --- NO ACTION: Complete in both ---
    if [[ -f "$comparison_dir/complete-in-both.txt" ]]; then
        while IFS= read -r line; do
            [[ "$line" =~ ^#.*$ || -z "$line" ]] && continue
            
            repo=$(echo "$line" | extract_repo)
            npub=$(echo "$line" | extract_npub)
            key="${repo}|${npub}"
            
            # Check if deleted (still no action, but different reason)
            if grep -qF "$key" "$tmp_dir/all_deletions.txt" 2>/dev/null; then
                echo "$repo | $npub | deleted by user (also complete in both)" >> "$no_action"
            else
                echo "$repo | $npub | complete in both prod and archive" >> "$no_action"
            fi
            echo "$key" >> "$tmp_dir/processed.txt"
            ((count_no_action++)) || true
        done < "$comparison_dir/complete-in-both.txt"
    fi
    
    # --- NO ACTION: Deleted by user (not already processed) ---
    while IFS='|' read -r repo npub; do
        [[ -z "$repo" ]] && continue
        key="${repo}|${npub}"
        
        # Skip if already processed
        if grep -qF "$key" "$tmp_dir/processed.txt" 2>/dev/null; then
            continue
        fi
        
        # Convert pubkey to npub if needed (deletions use hex pubkey)
        # For now, just use the pubkey as-is since we're matching by repo
        echo "$repo | $npub | deleted by user" >> "$no_action"
        echo "$key" >> "$tmp_dir/processed.txt"
        ((count_no_action++)) || true
    done < "$tmp_dir/all_deletions.txt"
    
    # --- NO ACTION: Empty/blank in both ---
    # Find repos that are category 2 in both prod and archive
    comm -12 "$tmp_dir/prod_cat2.txt" "$tmp_dir/archive_cat2.txt" 2>/dev/null | while IFS='|' read -r repo npub; do
        [[ -z "$repo" ]] && continue
        key="${repo}|${npub}"
        
        if grep -qF "$key" "$tmp_dir/processed.txt" 2>/dev/null; then
            continue
        fi
        
        echo "$repo | $npub | empty/blank in both (user never pushed)" >> "$no_action"
        echo "$key" >> "$tmp_dir/processed.txt"
    done
    
    # --- NO ACTION: Purgatory expired (system handled it) ---
    while IFS='|' read -r repo npub; do
        [[ -z "$repo" ]] && continue
        key="${repo}|${npub}"
        
        if grep -qF "$key" "$tmp_dir/processed.txt" 2>/dev/null; then
            continue
        fi
        
        echo "$repo | $npub | purgatory expired (system already handled)" >> "$no_action"
        echo "$key" >> "$tmp_dir/processed.txt"
        ((count_no_action++)) || true
    done < "$tmp_dir/purgatory_expired.txt"
    
    # --- ACTION REQUIRED: Complete in prod, missing from archive ---
    if [[ -f "$comparison_dir/complete-prod-missing-archive.txt" ]]; then
        while IFS= read -r line; do
            [[ "$line" =~ ^#.*$ || -z "$line" ]] && continue
            
            repo=$(echo "$line" | extract_repo)
            npub=$(echo "$line" | extract_npub)
            key="${repo}|${npub}"
            
            if grep -qF "$key" "$tmp_dir/processed.txt" 2>/dev/null; then
                continue
            fi
            
            # Check for parse failure
            if grep -qF "$key" "$tmp_dir/parse_failures.txt" 2>/dev/null; then
                echo "$repo | $npub | complete in prod, missing from archive, parse failure logged | investigate parse failure, may need re-announcement" >> "$manual_inv"
                echo "$key" >> "$tmp_dir/processed.txt"
                ((count_manual++)) || true
            else
                echo "$repo | $npub | complete in prod, missing from archive | trigger re-sync or investigate why not archived" >> "$action_req"
                echo "$key" >> "$tmp_dir/processed.txt"
                ((count_action++)) || true
            fi
        done < "$comparison_dir/complete-prod-missing-archive.txt"
    fi
    
    # --- ACTION REQUIRED: Complete in prod, incomplete in archive ---
    if [[ -f "$comparison_dir/complete-prod-incomplete-archive.txt" ]]; then
        while IFS= read -r line; do
            [[ "$line" =~ ^#.*$ || -z "$line" ]] && continue
            
            repo=$(echo "$line" | extract_repo)
            npub=$(echo "$line" | extract_npub)
            key="${repo}|${npub}"
            
            if grep -qF "$key" "$tmp_dir/processed.txt" 2>/dev/null; then
                continue
            fi
            
            # Extract archive status from line
            archive_status=$(echo "$line" | grep -oP 'archive=\K[^ ]+' || echo "unknown")
            
            echo "$repo | $npub | complete in prod, $archive_status in archive | wait for sync to complete or trigger re-sync" >> "$action_req"
            echo "$key" >> "$tmp_dir/processed.txt"
            ((count_action++)) || true
        done < "$comparison_dir/complete-prod-incomplete-archive.txt"
    fi
    
    # --- ACTION REQUIRED: Incomplete in both ---
    if [[ -f "$comparison_dir/incomplete-in-both.txt" ]]; then
        while IFS= read -r line; do
            [[ "$line" =~ ^#.*$ || -z "$line" ]] && continue
            
            repo=$(echo "$line" | extract_repo)
            npub=$(echo "$line" | extract_npub)
            key="${repo}|${npub}"
            
            if grep -qF "$key" "$tmp_dir/processed.txt" 2>/dev/null; then
                continue
            fi
            
            # Extract statuses
            prod_status=$(echo "$line" | grep -oP 'prod=\K[^ ]+' | tr -d '|' || echo "unknown")
            archive_status=$(echo "$line" | grep -oP 'archive=\K[^ ]+' || echo "unknown")
            
            echo "$repo | $npub | incomplete in both (prod=$prod_status, archive=$archive_status) | investigate git data source, may need user to re-push" >> "$action_req"
            echo "$key" >> "$tmp_dir/processed.txt"
            ((count_action++)) || true
        done < "$comparison_dir/incomplete-in-both.txt"
    fi
    
    # --- MANUAL INVESTIGATION: In archive but not prod ---
    if [[ -f "$comparison_dir/in-archive-not-prod.txt" ]]; then
        while IFS= read -r line; do
            [[ "$line" =~ ^#.*$ || -z "$line" ]] && continue
            
            repo=$(echo "$line" | extract_repo)
            npub=$(echo "$line" | extract_npub)
            key="${repo}|${npub}"
            
            if grep -qF "$key" "$tmp_dir/processed.txt" 2>/dev/null; then
                continue
            fi
            
            archive_status=$(echo "$line" | grep -oP 'archive=\K[^ ]+' || echo "unknown")
            
            # Check if it was deleted
            if grep -qF "$key" "$tmp_dir/all_deletions.txt" 2>/dev/null; then
                echo "$repo | $npub | in archive not prod, deletion exists | verify deletion was intentional" >> "$manual_inv"
            else
                echo "$repo | $npub | in archive ($archive_status) but not in prod | may be new announcement or deleted from prod" >> "$manual_inv"
            fi
            echo "$key" >> "$tmp_dir/processed.txt"
            ((count_manual++)) || true
        done < "$comparison_dir/in-archive-not-prod.txt"
    fi
    
    # --- ACTION REQUIRED: Parse failures not yet processed ---
    while IFS='|' read -r repo npub; do
        [[ -z "$repo" ]] && continue
        key="${repo}|${npub}"
        
        if grep -qF "$key" "$tmp_dir/processed.txt" 2>/dev/null; then
            continue
        fi
        
        echo "$repo | $npub | parse failure logged | fix event format or request user to re-announce" >> "$action_req"
        echo "$key" >> "$tmp_dir/processed.txt"
        ((count_action++)) || true
    done < "$tmp_dir/parse_failures.txt"
    
    # --- MANUAL INVESTIGATION: Prod category 3/4 not yet processed ---
    for cat_file in "$tmp_dir/prod_cat3.txt" "$tmp_dir/prod_cat4.txt"; do
        [[ ! -f "$cat_file" ]] && continue
        cat_name=$(basename "$cat_file" .txt | sed 's/prod_//')
        while IFS='|' read -r repo npub; do
            [[ -z "$repo" ]] && continue
            key="${repo}|${npub}"
            
            if grep -qF "$key" "$tmp_dir/processed.txt" 2>/dev/null; then
                continue
            fi
            
            if [[ "$cat_name" == "cat3" ]]; then
                echo "$repo | $npub | partial match in prod, not in comparison results | investigate git ref mismatch" >> "$manual_inv"
            else
                echo "$repo | $npub | no match in prod (git exists but refs don't match) | investigate git data corruption" >> "$manual_inv"
            fi
            echo "$key" >> "$tmp_dir/processed.txt"
            ((count_manual++)) || true
        done < "$cat_file"
    done
    
    # =========================================================================
    # STEP 5: Count final results
    # =========================================================================
    count_no_action=$(count_lines "$no_action")
    count_action=$(count_lines "$action_req")
    count_manual=$(count_lines "$manual_inv")
    
    # Ensure counts are valid integers
    count_no_action=${count_no_action:-0}
    count_action=${count_action:-0}
    count_manual=${count_manual:-0}
    
    local total=$((count_no_action + count_action + count_manual))
    
    # Handle division by zero
    if [[ $total -eq 0 ]]; then
        total=1  # Avoid division by zero in percentage calculations
        log_warn "No repos were classified. Check input files."
    fi
    
    # =========================================================================
    # STEP 6: Generate summary
    # =========================================================================
    log_info "Generating summary..."
    
    cat > "$summary" << EOF
# Migration Classification Summary
Generated: $(date -Iseconds)
Analysis Directory: $analysis_dir

## Overview

| Category | Count | Percentage |
|----------|-------|------------|
| No Action Required | $count_no_action | $(awk "BEGIN {printf \"%.1f\", ($count_no_action/$total)*100}")% |
| Action Required | $count_action | $(awk "BEGIN {printf \"%.1f\", ($count_action/$total)*100}")% |
| Manual Investigation | $count_manual | $(awk "BEGIN {printf \"%.1f\", ($count_manual/$total)*100}")% |
| **Total** | **$total** | **100%** |

## No Action Required ($count_no_action repos)

These repositories are ready for migration or don't need migration:

EOF

    # Breakdown of no-action reasons
    echo "| Reason | Count |" >> "$summary"
    echo "|--------|-------|" >> "$summary"
    grep -v '^#' "$no_action" 2>/dev/null | awk -F' \\| ' '{print $3}' | sort | uniq -c | sort -rn > "$tmp_dir/no_action_breakdown.txt" || true
    while read -r cnt reason; do
        echo "| $reason | $cnt |" >> "$summary"
    done < "$tmp_dir/no_action_breakdown.txt"
    
    cat >> "$summary" << EOF

## Action Required ($count_action repos)

These repositories need intervention before migration:

EOF

    # Breakdown of action reasons
    echo "| Reason | Count | Suggested Action |" >> "$summary"
    echo "|--------|-------|------------------|" >> "$summary"
    grep -v '^#' "$action_req" 2>/dev/null | awk -F' \\| ' '{print $3 "|" $4}' | sort | uniq -c | sort -rn > "$tmp_dir/action_breakdown.txt" || true
    while read -r cnt reason_action; do
        reason=$(echo "$reason_action" | cut -d'|' -f1)
        action=$(echo "$reason_action" | cut -d'|' -f2)
        echo "| $reason | $cnt | $action |" >> "$summary"
    done < "$tmp_dir/action_breakdown.txt"
    
    cat >> "$summary" << EOF

## Manual Investigation ($count_manual repos)

These repositories have conflicting or unexpected states requiring human review:

EOF

    # Breakdown of manual investigation reasons
    echo "| Reason | Count |" >> "$summary"
    echo "|--------|-------|" >> "$summary"
    grep -v '^#' "$manual_inv" 2>/dev/null | awk -F' \\| ' '{print $3}' | sort | uniq -c | sort -rn > "$tmp_dir/manual_breakdown.txt" || true
    while read -r cnt reason; do
        echo "| $reason | $cnt |" >> "$summary"
    done < "$tmp_dir/manual_breakdown.txt"
    
    # Pre-compute counts from temp files before they might be cleaned up
    local prod_del_count archive_del_count
    local prod_cat1_count prod_cat2_count prod_cat3_count prod_cat4_count
    local archive_cat1_count archive_cat2_count archive_cat3_count archive_cat4_count
    local parse_fail_count purgatory_count
    
    prod_del_count=$(wc -l < "$tmp_dir/prod_deletions.txt" 2>/dev/null | tr -d ' ') || prod_del_count=0
    archive_del_count=$(wc -l < "$tmp_dir/archive_deletions.txt" 2>/dev/null | tr -d ' ') || archive_del_count=0
    prod_cat1_count=$(wc -l < "$tmp_dir/prod_cat1.txt" 2>/dev/null | tr -d ' ') || prod_cat1_count=0
    prod_cat2_count=$(wc -l < "$tmp_dir/prod_cat2.txt" 2>/dev/null | tr -d ' ') || prod_cat2_count=0
    prod_cat3_count=$(wc -l < "$tmp_dir/prod_cat3.txt" 2>/dev/null | tr -d ' ') || prod_cat3_count=0
    prod_cat4_count=$(wc -l < "$tmp_dir/prod_cat4.txt" 2>/dev/null | tr -d ' ') || prod_cat4_count=0
    archive_cat1_count=$(wc -l < "$tmp_dir/archive_cat1.txt" 2>/dev/null | tr -d ' ') || archive_cat1_count=0
    archive_cat2_count=$(wc -l < "$tmp_dir/archive_cat2.txt" 2>/dev/null | tr -d ' ') || archive_cat2_count=0
    archive_cat3_count=$(wc -l < "$tmp_dir/archive_cat3.txt" 2>/dev/null | tr -d ' ') || archive_cat3_count=0
    archive_cat4_count=$(wc -l < "$tmp_dir/archive_cat4.txt" 2>/dev/null | tr -d ' ') || archive_cat4_count=0
    parse_fail_count=$(wc -l < "$tmp_dir/parse_failures.txt" 2>/dev/null | tr -d ' ') || parse_fail_count=0
    purgatory_count=$(wc -l < "$tmp_dir/purgatory_expired.txt" 2>/dev/null | tr -d ' ') || purgatory_count=0
    
    cat >> "$summary" << EOF

## Input Data Summary

### Phase 1 (Events)
- Prod deletions: $prod_del_count
- Archive deletions: $archive_del_count

### Phase 3 (Categories)
**Prod:**
- Category 1 (complete): $prod_cat1_count
- Category 2 (empty): $prod_cat2_count
- Category 3 (partial): $prod_cat3_count
- Category 4 (no match): $prod_cat4_count

**Archive:**
- Category 1 (complete): $archive_cat1_count
- Category 2 (empty): $archive_cat2_count
- Category 3 (partial): $archive_cat3_count
- Category 4 (no match): $archive_cat4_count

### Phase 4 (Logs)
- Parse failures: $parse_fail_count
- Purgatory expired: $purgatory_count

## Recommended Next Steps

1. **Review action-required.txt** - Address these repos before migration
2. **Review manual-investigation.txt** - Investigate unusual states
3. **Verify no-action-required.txt** - Spot-check a few repos to confirm
4. **Plan migration window** - Schedule cutover when action items are resolved

## Output Files

- \`results/no-action-required.txt\` - $count_no_action repos ready for migration
- \`results/action-required.txt\` - $count_action repos needing intervention
- \`results/manual-investigation.txt\` - $count_manual repos needing human review
- \`results/summary.txt\` - This summary file
EOF

    # =========================================================================
    # STEP 7: Display results
    # =========================================================================
    echo ""
    log_info "=== Classification Complete ==="
    echo ""
    log_success "No Action Required: $count_no_action repos"
    log_warn "Action Required: $count_action repos"
    log_error "Manual Investigation: $count_manual repos"
    echo ""
    log_info "Total: $total repos classified"
    echo ""
    log_info "Output files:"
    echo "  $no_action"
    echo "  $action_req"
    echo "  $manual_inv"
    echo "  $summary"
    echo ""
    
    # Show top action items
    if [[ $count_action -gt 0 ]]; then
        log_info "Top action items:"
        grep -v '^#' "$action_req" 2>/dev/null | awk -F' \\| ' '{print $3}' | sort | uniq -c | sort -rn | head -5 | while read -r cnt reason; do
            echo "  - $reason: $cnt repos"
        done
        echo ""
    fi
    
    # Show top investigation items
    if [[ $count_manual -gt 0 ]]; then
        log_info "Top investigation items:"
        grep -v '^#' "$manual_inv" 2>/dev/null | awk -F' \\| ' '{print $3}' | sort | uniq -c | sort -rn | head -5 | while read -r cnt reason; do
            echo "  - $reason: $cnt repos"
        done
        echo ""
    fi
    
    log_info "See $summary for full details and recommended next steps."
}

main "$@"
