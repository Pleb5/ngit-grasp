#!/usr/bin/env bash
#
# 40-classify-actions.sh - Classify repos by migration action required
#
# Implements the redesigned classification system (Option B) with user feedback:
#
# Tier 1: No Action Required (ready-for-migration.txt)
#   - Complete in both (prod=cat1, archive=cat1)
#   - Deleted by user (kind 5 event)
#   - Empty in prod (prod=cat2, any archive status)
#   - Archive-only (archive=any, prod=missing)
#   - Not in prod (purgatory-only, prod=missing)
#
# Tier 2: Action Required (needs-resync.txt)
#   - Complete in prod, missing from archive (with purgatory context)
#   - Complete in prod, incomplete in archive (with purgatory context)
#
# Tier 3: Manual Investigation (manual-review.txt)
#   - Partial in prod (prod=cat3)
#   - No-match in prod (prod=cat4)
#   - Parse failures
#   - Conflicting states
#
# Usage: ./40-classify-actions.sh <analysis-dir>
#
# Output format: repo | npub | prod_status | archive_status | context | action
#

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() { echo -e "${BLUE}[INFO]${NC} $*"; }
log_success() { echo -e "${GREEN}[OK]${NC} $*"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# Check arguments
if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <analysis-dir>"
    echo "Example: $0 work/migration-analysis-20260123-200701"
    exit 1
fi

ANALYSIS_DIR="$1"

# Validate analysis directory
if [[ ! -d "$ANALYSIS_DIR" ]]; then
    log_error "Analysis directory not found: $ANALYSIS_DIR"
    exit 1
fi

# Define paths
PROD_DIR="$ANALYSIS_DIR/prod"
ARCHIVE_DIR="$ANALYSIS_DIR/archive"
COMPARISON_DIR="$ANALYSIS_DIR/comparison"
LOGS_DIR="$ANALYSIS_DIR/logs"
RESULTS_DIR="$ANALYSIS_DIR/results"

# Validate required directories
for dir in "$PROD_DIR" "$ARCHIVE_DIR" "$COMPARISON_DIR" "$LOGS_DIR"; do
    if [[ ! -d "$dir" ]]; then
        log_error "Required directory not found: $dir"
        exit 1
    fi
done

# Create results directory
mkdir -p "$RESULTS_DIR"

# Output files
READY_FILE="$RESULTS_DIR/ready-for-migration.txt"
RESYNC_FILE="$RESULTS_DIR/needs-resync.txt"
REVIEW_FILE="$RESULTS_DIR/manual-review.txt"
SUMMARY_FILE="$RESULTS_DIR/summary.txt"

# Temporary files for processing
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

log_info "Starting classification with revised system (Option B)"
log_info "Analysis directory: $ANALYSIS_DIR"

# ============================================================================
# Phase 1: Build lookup tables from source data
# ============================================================================

log_info "Building lookup tables..."

# Build prod category lookup: repo|npub -> category
declare -A PROD_CAT
while IFS='|' read -r repo npub rest || [[ -n "$repo" ]]; do
    repo="${repo// /}"  # Remove all spaces
    npub="${npub// /}"  # Remove all spaces
    [[ -z "$repo" || -z "$npub" ]] && continue
    PROD_CAT["$repo|$npub"]="cat1"
done < "$PROD_DIR/category1-complete-match.txt"

while IFS='|' read -r repo npub rest || [[ -n "$repo" ]]; do
    repo="${repo// /}"
    npub="${npub// /}"
    [[ -z "$repo" || -z "$npub" ]] && continue
    PROD_CAT["$repo|$npub"]="cat2"
done < "$PROD_DIR/category2-empty-blank.txt"

while IFS='|' read -r repo npub rest || [[ -n "$repo" ]]; do
    repo="${repo// /}"
    npub="${npub// /}"
    [[ -z "$repo" || -z "$npub" ]] && continue
    PROD_CAT["$repo|$npub"]="cat3"
done < "$PROD_DIR/category3-partial-match.txt"

while IFS='|' read -r repo npub rest || [[ -n "$repo" ]]; do
    repo="${repo// /}"
    npub="${npub// /}"
    [[ -z "$repo" || -z "$npub" ]] && continue
    PROD_CAT["$repo|$npub"]="cat4"
done < "$PROD_DIR/category4-no-match.txt"

log_info "Loaded ${#PROD_CAT[@]} prod entries"

# Build archive category lookup: repo|npub -> category
declare -A ARCHIVE_CAT
while IFS='|' read -r repo npub rest; do
    repo="${repo// /}"
    npub="${npub// /}"
    [[ -z "$repo" || -z "$npub" ]] && continue
    ARCHIVE_CAT["$repo|$npub"]="cat1"
done < "$ARCHIVE_DIR/category1-complete-match.txt"

while IFS='|' read -r repo npub rest; do
    repo="${repo// /}"
    npub="${npub// /}"
    [[ -z "$repo" || -z "$npub" ]] && continue
    ARCHIVE_CAT["$repo|$npub"]="cat2"
done < "$ARCHIVE_DIR/category2-empty-blank.txt"

while IFS='|' read -r repo npub rest; do
    repo="${repo// /}"
    npub="${npub// /}"
    [[ -z "$repo" || -z "$npub" ]] && continue
    ARCHIVE_CAT["$repo|$npub"]="cat3"
done < "$ARCHIVE_DIR/category3-partial-match.txt"

while IFS='|' read -r repo npub rest; do
    repo="${repo// /}"
    npub="${npub// /}"
    [[ -z "$repo" || -z "$npub" ]] && continue
    ARCHIVE_CAT["$repo|$npub"]="cat4"
done < "$ARCHIVE_DIR/category4-no-match.txt"

log_info "Loaded ${#ARCHIVE_CAT[@]} archive entries"

# Build purgatory lookup: repo|npub -> 1 (if purgatory expired)
declare -A PURGATORY
PURGATORY_COUNT=0
if [[ -f "$LOGS_DIR/purgatory-expired.txt" ]]; then
    while IFS=$'\t' read -r repo npub timestamp reason || [[ -n "$repo" ]]; do
        # Skip comments and empty lines
        [[ "$repo" =~ ^# ]] && continue
        [[ -z "$repo" || -z "$npub" ]] && continue
        PURGATORY["$repo|$npub"]=1
        PURGATORY_COUNT=$((PURGATORY_COUNT + 1))
    done < "$LOGS_DIR/purgatory-expired.txt"
fi
log_info "Loaded $PURGATORY_COUNT purgatory entries"

# Build parse failure lookup: repo|npub -> 1 (if parse failure logged)
# Parse failures file format: event_id<TAB>kind<TAB>reason<TAB>repo<TAB>npub
declare -A PARSE_FAIL
PARSE_FAIL_COUNT=0
if [[ -f "$LOGS_DIR/parse-failures.txt" ]]; then
    while IFS=$'\t' read -r event_id kind reason repo npub || [[ -n "$event_id" ]]; do
        # Skip comments and empty lines
        [[ "$event_id" =~ ^# ]] && continue
        [[ -z "$repo" || -z "$npub" ]] && continue
        PARSE_FAIL["$repo|$npub"]=1
        PARSE_FAIL_COUNT=$((PARSE_FAIL_COUNT + 1))
    done < "$LOGS_DIR/parse-failures.txt"
fi
log_info "Loaded $PARSE_FAIL_COUNT parse failure entries"

# Build deletion lookup: repo|npub -> 1 (if kind 5 deletion event)
# Deletions are in NDJSON format with "a" tags like "30617:pubkey_hex:repo"
# We need to convert hex pubkeys to npub format using nak
declare -A DELETED

# Helper function to process deletion file (NDJSON format)
# Extracts unique pubkey_hex:repo pairs and converts to npub
process_deletions() {
    local file="$1"
    [[ ! -f "$file" ]] && return
    
    # Extract unique pubkey_hex|repo pairs from NDJSON
    # Each line is a JSON object, extract "a" tags
    local pairs
    pairs=$(jq -r '.tags[] | select(.[0] == "a") | .[1]' "$file" 2>/dev/null | \
            sed 's/^30617://' | awk -F: '{print $1 "|" $2}' | sort -u)
    
    # Get unique hex pubkeys for batch conversion
    local hex_keys
    hex_keys=$(echo "$pairs" | cut -d'|' -f1 | sort -u)
    
    # Build hex->npub lookup via batch nak call
    declare -A HEX_TO_NPUB
    while read -r hex; do
        [[ -z "$hex" ]] && continue
        local npub
        npub=$(nak encode npub "$hex" 2>/dev/null || echo "")
        [[ -n "$npub" ]] && HEX_TO_NPUB["$hex"]="$npub"
    done <<< "$hex_keys"
    
    # Now process pairs with cached npub values
    while IFS='|' read -r pubkey_hex repo; do
        [[ -z "$repo" || -z "$pubkey_hex" ]] && continue
        local npub="${HEX_TO_NPUB[$pubkey_hex]:-}"
        [[ -z "$npub" ]] && continue
        DELETED["$repo|$npub"]=1
    done <<< "$pairs"
}

# Process prod and archive deletions
process_deletions "$PROD_DIR/raw/deletions.json"
process_deletions "$ARCHIVE_DIR/raw/deletions.json"
DELETED_COUNT=0
[[ ${#DELETED[@]} -gt 0 ]] && DELETED_COUNT=${#DELETED[@]}
log_info "Loaded $DELETED_COUNT deletion entries"

# ============================================================================
# Phase 2: Build unique repo list from all sources
# ============================================================================

log_info "Building unique repo list..."

declare -A ALL_REPOS
for key in "${!PROD_CAT[@]}"; do
    ALL_REPOS["$key"]=1
done
for key in "${!ARCHIVE_CAT[@]}"; do
    ALL_REPOS["$key"]=1
done
for key in "${!PURGATORY[@]}"; do
    ALL_REPOS["$key"]=1
done

log_info "Total unique repos: ${#ALL_REPOS[@]}"

# ============================================================================
# Phase 3: Classify each repo according to revised decision tree
# ============================================================================

log_info "Classifying repos..."

# Counters for summary
declare -A COUNTS
COUNTS[ready_complete_both]=0
COUNTS[ready_deleted]=0
COUNTS[ready_empty_prod]=0
COUNTS[ready_archive_only]=0
COUNTS[ready_not_in_prod]=0
COUNTS[resync_missing_archive]=0
COUNTS[resync_incomplete_archive]=0
COUNTS[review_partial_prod]=0
COUNTS[review_nomatch_prod]=0
COUNTS[review_parse_failure]=0
COUNTS[review_conflicting]=0

# Output arrays
declare -a READY_LINES
declare -a RESYNC_LINES
declare -a REVIEW_LINES

# Helper function to get context string
get_context() {
    local key="$1"
    local prod_status="$2"
    local archive_status="$3"
    local context=""
    
    # Check purgatory
    if [[ -n "${PURGATORY[$key]:-}" ]]; then
        context="purgatory-expired"
    fi
    
    # Check parse failure
    if [[ -n "${PARSE_FAIL[$key]:-}" ]]; then
        if [[ -n "$context" ]]; then
            context="$context, parse-failure"
        else
            context="parse-failure"
        fi
    fi
    
    # Add archive context for unexpected states
    if [[ "$prod_status" == "empty" && "$archive_status" != "missing" && "$archive_status" != "empty" ]]; then
        if [[ -n "$context" ]]; then
            context="$context, archive-has-data"
        else
            context="archive-has-data"
        fi
    fi
    
    echo "${context:-none}"
}

# Helper to convert category to human-readable status
cat_to_status() {
    case "$1" in
        cat1) echo "complete" ;;
        cat2) echo "empty" ;;
        cat3) echo "partial" ;;
        cat4) echo "no-match" ;;
        missing) echo "missing" ;;
        *) echo "$1" ;;
    esac
}

LOOP_COUNT=0
for key in "${!ALL_REPOS[@]}"; do
    LOOP_COUNT=$((LOOP_COUNT + 1))
    [[ $((LOOP_COUNT % 100)) -eq 0 ]] && log_info "Processed $LOOP_COUNT repos..."
    IFS='|' read -r repo npub <<< "$key"
    
    prod_cat="${PROD_CAT[$key]:-missing}"
    archive_cat="${ARCHIVE_CAT[$key]:-missing}"
    prod_status=$(cat_to_status "$prod_cat")
    archive_status=$(cat_to_status "$archive_cat")
    
    # Decision tree implementation
    
    # 1. Is there a kind 5 deletion event?
    if [[ -n "${DELETED[$key]:-}" ]]; then
        context=$(get_context "$key" "$prod_status" "$archive_status")
        READY_LINES+=("$repo | $npub | $prod_status | $archive_status | $context | deleted by user")
        COUNTS[ready_deleted]=$((COUNTS[ready_deleted] + 1))
        continue
    fi
    
    # 2. What is the prod status?
    case "$prod_cat" in
        missing)
            # Not in prod
            if [[ "$archive_cat" != "missing" ]]; then
                # In archive but not in prod -> no action (archive-only)
                context=$(get_context "$key" "$prod_status" "$archive_status")
                READY_LINES+=("$repo | $npub | $prod_status | $archive_status | $context | archive-only (not in prod)")
                COUNTS[ready_archive_only]=$((COUNTS[ready_archive_only] + 1))
            elif [[ -n "${PURGATORY[$key]:-}" ]]; then
                # Purgatory only, not in prod -> no action
                context="purgatory-expired"
                READY_LINES+=("$repo | $npub | $prod_status | $archive_status | $context | purgatory-only (not in prod)")
                COUNTS[ready_not_in_prod]=$((COUNTS[ready_not_in_prod] + 1))
            fi
            # Otherwise skip (not a real repo - no data anywhere)
            ;;
            
        cat2)
            # Empty in prod -> ALWAYS no action required
            context=$(get_context "$key" "$prod_status" "$archive_status")
            READY_LINES+=("$repo | $npub | $prod_status | $archive_status | $context | empty in prod (user never pushed)")
            COUNTS[ready_empty_prod]=$((COUNTS[ready_empty_prod] + 1))
            ;;
            
        cat1)
            # Complete in prod
            if [[ "$archive_cat" == "cat1" ]]; then
                # Complete in both -> no action
                context=$(get_context "$key" "$prod_status" "$archive_status")
                READY_LINES+=("$repo | $npub | $prod_status | $archive_status | $context | complete in both")
                COUNTS[ready_complete_both]=$((COUNTS[ready_complete_both] + 1))
            else
                # Complete in prod, missing/incomplete in archive
                # Check for parse failure - if so, needs manual review
                if [[ -n "${PARSE_FAIL[$key]:-}" ]]; then
                    context=$(get_context "$key" "$prod_status" "$archive_status")
                    REVIEW_LINES+=("$repo | $npub | $prod_status | $archive_status | $context | complete in prod with parse failure")
                    COUNTS[review_parse_failure]=$((COUNTS[review_parse_failure] + 1))
                else
                    # Needs resync - include purgatory context
                    context=$(get_context "$key" "$prod_status" "$archive_status")
                    if [[ "$archive_cat" == "missing" ]]; then
                        RESYNC_LINES+=("$repo | $npub | $prod_status | $archive_status | $context | trigger re-sync to archive")
                        COUNTS[resync_missing_archive]=$((COUNTS[resync_missing_archive] + 1))
                    else
                        RESYNC_LINES+=("$repo | $npub | $prod_status | $archive_status | $context | trigger re-sync (archive incomplete)")
                        COUNTS[resync_incomplete_archive]=$((COUNTS[resync_incomplete_archive] + 1))
                    fi
                fi
            fi
            ;;
            
        cat3)
            # Partial in prod -> ALWAYS manual investigation
            context=$(get_context "$key" "$prod_status" "$archive_status")
            REVIEW_LINES+=("$repo | $npub | $prod_status | $archive_status | $context | partial in prod (investigate git data)")
            COUNTS[review_partial_prod]=$((COUNTS[review_partial_prod] + 1))
            ;;
            
        cat4)
            # No-match in prod -> ALWAYS manual investigation
            context=$(get_context "$key" "$prod_status" "$archive_status")
            REVIEW_LINES+=("$repo | $npub | $prod_status | $archive_status | $context | no-match in prod (git corruption)")
            COUNTS[review_nomatch_prod]=$((COUNTS[review_nomatch_prod] + 1))
            ;;
    esac
done

# ============================================================================
# Phase 4: Write output files
# ============================================================================

log_info "Writing output files..."

TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%S+00:00")

# Write ready-for-migration.txt
{
    echo "# Ready for Migration - No action required"
    echo "# Generated: $TIMESTAMP"
    echo "# Format: repo | npub | prod_status | archive_status | context | reason"
    echo "#"
    for line in "${READY_LINES[@]}"; do
        echo "$line"
    done
} > "$READY_FILE"

# Write needs-resync.txt
{
    echo "# Needs Re-sync - Action required"
    echo "# Generated: $TIMESTAMP"
    echo "# Format: repo | npub | prod_status | archive_status | context | action"
    echo "#"
    echo "# Context meanings:"
    echo "#   purgatory-expired = archive tried to sync but failed (30min timeout)"
    echo "#   none = archive never tried or announcement missing"
    echo "#"
    for line in "${RESYNC_LINES[@]}"; do
        echo "$line"
    done
} > "$RESYNC_FILE"

# Write manual-review.txt
{
    echo "# Manual Review Required - Investigation needed"
    echo "# Generated: $TIMESTAMP"
    echo "# Format: repo | npub | prod_status | archive_status | context | reason"
    echo "#"
    for line in "${REVIEW_LINES[@]}"; do
        echo "$line"
    done
} > "$REVIEW_FILE"

# ============================================================================
# Phase 5: Generate summary
# ============================================================================

log_info "Generating summary..."

TOTAL_READY="${#READY_LINES[@]}"
TOTAL_RESYNC="${#RESYNC_LINES[@]}"
TOTAL_REVIEW="${#REVIEW_LINES[@]}"
TOTAL=$((TOTAL_READY + TOTAL_RESYNC + TOTAL_REVIEW))

# Calculate percentages
if [[ $TOTAL -gt 0 ]]; then
    PCT_READY=$(awk "BEGIN {printf \"%.1f\", ($TOTAL_READY / $TOTAL) * 100}")
    PCT_RESYNC=$(awk "BEGIN {printf \"%.1f\", ($TOTAL_RESYNC / $TOTAL) * 100}")
    PCT_REVIEW=$(awk "BEGIN {printf \"%.1f\", ($TOTAL_REVIEW / $TOTAL) * 100}")
else
    PCT_READY="0.0"
    PCT_RESYNC="0.0"
    PCT_REVIEW="0.0"
fi

{
    echo "# Migration Classification Summary"
    echo "Generated: $TIMESTAMP"
    echo "Analysis Directory: $ANALYSIS_DIR"
    echo ""
    echo "## Overview"
    echo ""
    echo "| Category | Count | Percentage |"
    echo "|----------|-------|------------|"
    echo "| Ready for Migration | $TOTAL_READY | $PCT_READY% |"
    echo "| Needs Re-sync | $TOTAL_RESYNC | $PCT_RESYNC% |"
    echo "| Manual Review | $TOTAL_REVIEW | $PCT_REVIEW% |"
    echo "| **Total** | **$TOTAL** | **100%** |"
    echo ""
    echo "## Tier 1: Ready for Migration ($TOTAL_READY repos)"
    echo ""
    echo "These repositories are ready for migration or don't need migration:"
    echo ""
    echo "| Reason | Count |"
    echo "|--------|-------|"
    echo "| complete in both prod and archive | ${COUNTS[ready_complete_both]} |"
    echo "| deleted by user | ${COUNTS[ready_deleted]} |"
    echo "| empty in prod (user never pushed) | ${COUNTS[ready_empty_prod]} |"
    echo "| archive-only (not in prod) | ${COUNTS[ready_archive_only]} |"
    echo "| purgatory-only (not in prod) | ${COUNTS[ready_not_in_prod]} |"
    echo ""
    echo "## Tier 2: Needs Re-sync ($TOTAL_RESYNC repos)"
    echo ""
    echo "These repositories need re-sync to archive before migration:"
    echo ""
    echo "| Reason | Count | Action |"
    echo "|--------|-------|--------|"
    echo "| complete in prod, missing from archive | ${COUNTS[resync_missing_archive]} | trigger re-sync |"
    echo "| complete in prod, incomplete in archive | ${COUNTS[resync_incomplete_archive]} | trigger re-sync |"
    echo ""
    echo "### Purgatory Context"
    echo ""
    echo "Repos in needs-resync.txt include purgatory context:"
    echo "- **purgatory-expired**: Archive tried to sync but failed (30min timeout)"
    echo "- **none**: Archive never tried or announcement missing"
    echo ""
    echo "## Tier 3: Manual Review ($TOTAL_REVIEW repos)"
    echo ""
    echo "These repositories require human investigation:"
    echo ""
    echo "| Reason | Count |"
    echo "|--------|-------|"
    echo "| partial in prod (cat3) | ${COUNTS[review_partial_prod]} |"
    echo "| no-match in prod (cat4) | ${COUNTS[review_nomatch_prod]} |"
    echo "| complete in prod with parse failure | ${COUNTS[review_parse_failure]} |"
    echo ""
    echo "## Input Data Summary"
    echo ""
    echo "### Prod Categories"
    echo "- Category 1 (complete): $(wc -l < "$PROD_DIR/category1-complete-match.txt")"
    echo "- Category 2 (empty): $(wc -l < "$PROD_DIR/category2-empty-blank.txt")"
    echo "- Category 3 (partial): $(wc -l < "$PROD_DIR/category3-partial-match.txt")"
    echo "- Category 4 (no match): $(wc -l < "$PROD_DIR/category4-no-match.txt")"
    echo ""
    echo "### Archive Categories"
    echo "- Category 1 (complete): $(wc -l < "$ARCHIVE_DIR/category1-complete-match.txt")"
    echo "- Category 2 (empty): $(wc -l < "$ARCHIVE_DIR/category2-empty-blank.txt")"
    echo "- Category 3 (partial): $(wc -l < "$ARCHIVE_DIR/category3-partial-match.txt")"
    echo "- Category 4 (no match): $(wc -l < "$ARCHIVE_DIR/category4-no-match.txt")"
    echo ""
    echo "### Logs"
    echo "- Parse failures: $(grep -c -v '^#' "$LOGS_DIR/parse-failures.txt" 2>/dev/null || echo 0)"
    echo "- Purgatory expired: $(grep -c -v '^#' "$LOGS_DIR/purgatory-expired.txt" 2>/dev/null || echo 0)"
    echo ""
    echo "## Output Files"
    echo ""
    echo "- \`results/ready-for-migration.txt\` - $TOTAL_READY repos ready for migration"
    echo "- \`results/needs-resync.txt\` - $TOTAL_RESYNC repos needing re-sync"
    echo "- \`results/manual-review.txt\` - $TOTAL_REVIEW repos needing investigation"
    echo "- \`results/summary.txt\` - This summary file"
    echo ""
    echo "## Recommended Next Steps"
    echo ""
    echo "1. **Review needs-resync.txt** - Trigger re-sync for these repos"
    echo "2. **Review manual-review.txt** - Investigate unusual states"
    echo "3. **Verify ready-for-migration.txt** - Spot-check a few repos"
    echo "4. **Plan migration window** - Schedule cutover when action items resolved"
} > "$SUMMARY_FILE"

# ============================================================================
# Phase 6: Print summary to console
# ============================================================================

echo ""
log_success "Classification complete!"
echo ""
echo "=== Summary ==="
echo "Ready for Migration: $TOTAL_READY ($PCT_READY%)"
echo "  - Complete in both: ${COUNTS[ready_complete_both]}"
echo "  - Deleted by user: ${COUNTS[ready_deleted]}"
echo "  - Empty in prod: ${COUNTS[ready_empty_prod]}"
echo "  - Archive-only: ${COUNTS[ready_archive_only]}"
echo "  - Purgatory-only: ${COUNTS[ready_not_in_prod]}"
echo ""
echo "Needs Re-sync: $TOTAL_RESYNC ($PCT_RESYNC%)"
echo "  - Missing from archive: ${COUNTS[resync_missing_archive]}"
echo "  - Incomplete in archive: ${COUNTS[resync_incomplete_archive]}"
echo ""
echo "Manual Review: $TOTAL_REVIEW ($PCT_REVIEW%)"
echo "  - Partial in prod: ${COUNTS[review_partial_prod]}"
echo "  - No-match in prod: ${COUNTS[review_nomatch_prod]}"
echo "  - Parse failures: ${COUNTS[review_parse_failure]}"
echo ""
echo "Total: $TOTAL repos"
echo ""
echo "Output files:"
echo "  $READY_FILE"
echo "  $RESYNC_FILE"
echo "  $REVIEW_FILE"
echo "  $SUMMARY_FILE"
