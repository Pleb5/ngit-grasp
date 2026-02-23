#!/usr/bin/env bash
#
# 21-compare-relays.sh - Compare prod vs archive category files to find gaps
#
# PHASE 3b of the GRASP relay to ngit-grasp migration analysis pipeline.
# Compares categorized output from prod and archive to identify:
# - Repos complete in prod but missing/incomplete in archive
# - Repos in archive but not in prod
# - Status differences between relays
#
# USAGE:
#   ./21-compare-relays.sh <prod-dir> <archive-dir> <output-dir>
#
# EXAMPLES:
#   ./21-compare-relays.sh output/prod output/archive output/comparison
#
# INPUT:
#   Both prod-dir and archive-dir must contain:
#   - category1-complete-match.txt
#   - category2-empty-blank.txt
#   - category3-partial-match.txt
#   - category4-no-match.txt
#
# OUTPUT:
#   <output-dir>/complete-in-both.txt           - Repos complete in both relays (no action)
#   <output-dir>/complete-prod-missing-archive.txt - Complete in prod, not in archive cat1
#   <output-dir>/complete-prod-incomplete-archive.txt - Complete in prod, incomplete in archive
#   <output-dir>/incomplete-in-both.txt         - Incomplete in both relays
#   <output-dir>/in-archive-not-prod.txt        - In archive but not in prod
#   <output-dir>/summary.txt                    - Human-readable summary
#
# OUTPUT FORMAT:
#   Each file contains lines in the format:
#   repo | npub | prod_status | archive_status
#
# PREREQUISITES:
#   - awk, sort, comm (standard Unix tools)
#
# RUNTIME: < 1 second (local processing only)
#
# SEE ALSO:
#   docs/how-to/migrate-to-ngit-grasp.md - Full migration guide
#   20-categorize.sh - Phase 3a script that produces input for this script
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

usage() {
    echo "Usage: $0 <prod-dir> <archive-dir> <output-dir>"
    echo ""
    echo "Arguments:"
    echo "  prod-dir     Directory containing prod category files"
    echo "  archive-dir  Directory containing archive category files"
    echo "  output-dir   Directory to store comparison results"
    echo ""
    echo "Examples:"
    echo "  $0 output/prod output/archive output/comparison"
    echo ""
    echo "Required input files in each directory:"
    echo "  category1-complete-match.txt"
    echo "  category2-empty-blank.txt"
    echo "  category3-partial-match.txt"
    echo "  category4-no-match.txt"
    exit 1
}

# Extract repo|npub key from category line
# Input: "repo | npub | state_refs=N | ..."
# Output: "repo|npub"
extract_key() {
    awk -F' \\| ' '{print $1 "|" $2}'
}

# Build lookup table from category files
# Args: $1=directory, $2=output_file
build_lookup() {
    local dir="$1"
    local output="$2"
    
    # Process all 4 category files
    for cat in 1 2 3 4; do
        local file="$dir/category${cat}-*.txt"
        # shellcheck disable=SC2086
        if ls $file 1>/dev/null 2>&1; then
            # shellcheck disable=SC2086
            cat $file | while IFS= read -r line; do
                key=$(echo "$line" | extract_key)
                echo "${key}|cat${cat}|${line}"
            done
        fi
    done | sort -t'|' -k1,2 > "$output"
}

# Main
main() {
    if [[ $# -ne 3 ]]; then
        usage
    fi

    local prod_dir="$1"
    local archive_dir="$2"
    local output_dir="$3"

    # Validate input directories
    for dir in "$prod_dir" "$archive_dir"; do
        if [[ ! -d "$dir" ]]; then
            log_error "Directory not found: $dir"
            exit 1
        fi
        if [[ ! -f "$dir/category1-complete-match.txt" ]]; then
            log_error "Missing category1-complete-match.txt in $dir"
            exit 1
        fi
    done

    log_info "Comparing relay categories"
    log_info "Prod: $prod_dir"
    log_info "Archive: $archive_dir"
    log_info "Output: $output_dir"

    # Create output directory
    mkdir -p "$output_dir"

    # Create temp files for processing
    local tmp_dir
    tmp_dir=$(mktemp -d)
    # shellcheck disable=SC2064
    trap "rm -rf '$tmp_dir'" EXIT

    log_info "Building lookup tables..."

    # Build lookup tables: key|category|full_line
    build_lookup "$prod_dir" "$tmp_dir/prod_lookup.txt"
    build_lookup "$archive_dir" "$tmp_dir/archive_lookup.txt"

    # Extract just keys for comparison
    cut -d'|' -f1,2 "$tmp_dir/prod_lookup.txt" | sort -u > "$tmp_dir/prod_keys.txt"
    cut -d'|' -f1,2 "$tmp_dir/archive_lookup.txt" | sort -u > "$tmp_dir/archive_keys.txt"

    log_info "Comparing categories..."

    # Initialize output files
    > "$output_dir/complete-in-both.txt"
    > "$output_dir/complete-prod-missing-archive.txt"
    > "$output_dir/complete-prod-incomplete-archive.txt"
    > "$output_dir/incomplete-in-both.txt"
    > "$output_dir/in-archive-not-prod.txt"

    # Process prod category 1 (complete) entries
    while IFS='|' read -r repo npub cat full_line; do
        key="${repo}|${npub}"
        
        # Look up in archive
        archive_entry=$(grep "^${key}|" "$tmp_dir/archive_lookup.txt" 2>/dev/null | head -1 || echo "")
        
        if [[ -z "$archive_entry" ]]; then
            # Not in archive at all
            echo "$repo | $npub | prod=complete | archive=missing" >> "$output_dir/complete-prod-missing-archive.txt"
        else
            archive_cat=$(echo "$archive_entry" | cut -d'|' -f3)
            if [[ "$archive_cat" == "cat1" ]]; then
                # Complete in both
                echo "$repo | $npub | prod=complete | archive=complete" >> "$output_dir/complete-in-both.txt"
            else
                # Complete in prod, incomplete in archive
                echo "$repo | $npub | prod=complete | archive=$archive_cat" >> "$output_dir/complete-prod-incomplete-archive.txt"
            fi
        fi
    done < <(grep '|cat1|' "$tmp_dir/prod_lookup.txt" | sed 's/|cat1|/|cat1|/')

    # Process prod categories 2-4 (incomplete) entries
    for cat in cat2 cat3 cat4; do
        while IFS='|' read -r repo npub _ full_line; do
            key="${repo}|${npub}"
            
            # Look up in archive
            archive_entry=$(grep "^${key}|" "$tmp_dir/archive_lookup.txt" 2>/dev/null | head -1 || echo "")
            
            if [[ -z "$archive_entry" ]]; then
                # Incomplete in prod, missing in archive
                echo "$repo | $npub | prod=$cat | archive=missing" >> "$output_dir/incomplete-in-both.txt"
            else
                archive_cat=$(echo "$archive_entry" | cut -d'|' -f3)
                if [[ "$archive_cat" != "cat1" ]]; then
                    # Incomplete in both
                    echo "$repo | $npub | prod=$cat | archive=$archive_cat" >> "$output_dir/incomplete-in-both.txt"
                fi
                # If archive is complete but prod is not, that's unusual but not an error
            fi
        done < <(grep "|${cat}|" "$tmp_dir/prod_lookup.txt")
    done

    # Find entries in archive but not in prod
    comm -23 "$tmp_dir/archive_keys.txt" "$tmp_dir/prod_keys.txt" | while IFS='|' read -r repo npub; do
        key="${repo}|${npub}"
        archive_entry=$(grep "^${key}|" "$tmp_dir/archive_lookup.txt" 2>/dev/null | head -1 || echo "")
        archive_cat=$(echo "$archive_entry" | cut -d'|' -f3)
        echo "$repo | $npub | prod=missing | archive=$archive_cat" >> "$output_dir/in-archive-not-prod.txt"
    done

    # Count results
    local count_both count_missing count_incomplete count_both_incomplete count_archive_only
    count_both=$(wc -l < "$output_dir/complete-in-both.txt" | tr -d ' ')
    count_missing=$(wc -l < "$output_dir/complete-prod-missing-archive.txt" | tr -d ' ')
    count_incomplete=$(wc -l < "$output_dir/complete-prod-incomplete-archive.txt" | tr -d ' ')
    count_both_incomplete=$(wc -l < "$output_dir/incomplete-in-both.txt" | tr -d ' ')
    count_archive_only=$(wc -l < "$output_dir/in-archive-not-prod.txt" | tr -d ' ')

    # Generate summary
    cat > "$output_dir/summary.txt" << EOF
# Relay Comparison Summary
Generated: $(date -Iseconds)

## Input
- Prod: $prod_dir
- Archive: $archive_dir

## Results

### No Action Required
- Complete in both relays: $count_both

### Action/Decision Required
- Complete in prod, MISSING from archive: $count_missing
- Complete in prod, INCOMPLETE in archive: $count_incomplete
- Incomplete in BOTH relays: $count_both_incomplete

### For Reference
- In archive but not in prod: $count_archive_only

## Files
- complete-in-both.txt: Repos successfully migrated (no action)
- complete-prod-missing-archive.txt: Need investigation - why not in archive?
- complete-prod-incomplete-archive.txt: Archive sync may still be in progress
- incomplete-in-both.txt: Git data incomplete on both relays
- in-archive-not-prod.txt: May be deleted from prod or new to archive

## Next Steps
1. Review complete-prod-missing-archive.txt - these repos need attention
2. Check if archive sync is still running for incomplete entries
3. Cross-reference with deletion events (kind 5) from Phase 1
4. Use Phase 4 logs to understand parse failures and purgatory expiry
EOF

    # Display summary
    echo ""
    log_info "=== Comparison Summary ==="
    log_success "Complete in both: $count_both (no action needed)"
    log_error "Complete in prod, MISSING from archive: $count_missing"
    log_warn "Complete in prod, incomplete in archive: $count_incomplete"
    log_warn "Incomplete in both: $count_both_incomplete"
    log_info "In archive only: $count_archive_only"
    echo ""
    log_info "Output files:"
    echo "  $output_dir/complete-in-both.txt"
    echo "  $output_dir/complete-prod-missing-archive.txt"
    echo "  $output_dir/complete-prod-incomplete-archive.txt"
    echo "  $output_dir/incomplete-in-both.txt"
    echo "  $output_dir/in-archive-not-prod.txt"
    echo "  $output_dir/summary.txt"
}

main "$@"
