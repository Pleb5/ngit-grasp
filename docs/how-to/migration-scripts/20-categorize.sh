#!/usr/bin/env bash
#
# 20-categorize.sh - Categorize git sync status into 4 categories
#
# PHASE 3a of the GRASP relay to ngit-grasp migration analysis pipeline.
# Takes git-sync-status.tsv from Phase 2 and categorizes into 4 files.
#
# USAGE:
#   ./20-categorize.sh <git-sync-status.tsv> <output-dir>
#
# EXAMPLES:
#   ./20-categorize.sh output/prod/git-sync-status.tsv output/prod
#   ./20-categorize.sh output/archive/git-sync-status.tsv output/archive
#
# INPUT FORMAT (git-sync-status.tsv):
#   Tab-separated values with columns:
#   repo<TAB>npub<TAB>state_refs<TAB>git_refs<TAB>matches<TAB>reason
#
#   Where reason is optional and can be: no_git_dir, empty_refs, no_state_refs
#
# OUTPUT:
#   <output-dir>/category1-complete-match.txt  - All refs match perfectly
#   <output-dir>/category2-empty-blank.txt     - No git data available
#   <output-dir>/category3-partial-match.txt   - Some refs match
#   <output-dir>/category4-no-match.txt        - Git exists but refs don't match
#
# OUTPUT FORMAT:
#   repo | npub | state_refs=N | git_refs=N | matches=N [| reason=X]
#
# CATEGORIES:
#   1. Complete Match: state_refs == git_refs == matches (all > 0)
#   2. Empty/Blank: git_refs == 0 OR reason in (no_git_dir, empty_refs, no_state_refs)
#   3. Partial Match: matches > 0 AND matches < state_refs
#   4. No Match: git_refs > 0 AND matches == 0
#
# PREREQUISITES:
#   - awk (standard Unix tool)
#
# RUNTIME: < 1 second (local processing only)
#
# SEE ALSO:
#   docs/how-to/migrate-to-ngit-grasp.md - Full migration guide
#   10-check-git-sync.sh - Phase 2 script that produces input for this script
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
    echo "Usage: $0 <git-sync-status.tsv> <output-dir>"
    echo ""
    echo "Arguments:"
    echo "  git-sync-status.tsv  TSV file from Phase 2 (10-check-git-sync.sh)"
    echo "  output-dir           Directory to store categorized output"
    echo ""
    echo "Examples:"
    echo "  $0 output/prod/git-sync-status.tsv output/prod"
    echo "  $0 output/archive/git-sync-status.tsv output/archive"
    echo ""
    echo "Input format (TSV):"
    echo "  repo<TAB>npub<TAB>state_refs<TAB>git_refs<TAB>matches<TAB>reason"
    echo ""
    echo "Output files:"
    echo "  category1-complete-match.txt  - All refs match"
    echo "  category2-empty-blank.txt     - No git data"
    echo "  category3-partial-match.txt   - Some refs match"
    echo "  category4-no-match.txt        - Git exists, refs don't match"
    exit 1
}

# Main
main() {
    if [[ $# -ne 2 ]]; then
        usage
    fi

    local input_file="$1"
    local output_dir="$2"

    # Validate input file
    if [[ ! -f "$input_file" ]]; then
        log_error "Input file not found: $input_file"
        exit 1
    fi

    log_info "Categorizing git sync status"
    log_info "Input: $input_file"
    log_info "Output: $output_dir"

    # Create output directory
    mkdir -p "$output_dir"

    # Output files
    local cat1="$output_dir/category1-complete-match.txt"
    local cat2="$output_dir/category2-empty-blank.txt"
    local cat3="$output_dir/category3-partial-match.txt"
    local cat4="$output_dir/category4-no-match.txt"

    # Clear previous results
    > "$cat1"
    > "$cat2"
    > "$cat3"
    > "$cat4"

    # Process input file with awk
    # Input: repo<TAB>npub<TAB>state_refs<TAB>git_refs<TAB>matches<TAB>reason
    awk -F'\t' -v cat1="$cat1" -v cat2="$cat2" -v cat3="$cat3" -v cat4="$cat4" '
    BEGIN {
        count1 = 0; count2 = 0; count3 = 0; count4 = 0
    }
    NR == 1 && /^repo/ { next }  # Skip header if present
    NF >= 5 {
        repo = $1
        npub = $2
        state_refs = int($3)
        git_refs = int($4)
        matches = int($5)
        reason = (NF >= 6) ? $6 : ""

        # Format output line
        if (reason != "") {
            line = repo " | " npub " | state_refs=" state_refs " | git_refs=" git_refs " | matches=" matches " | reason=" reason
        } else {
            line = repo " | " npub " | state_refs=" state_refs " | git_refs=" git_refs " | matches=" matches
        }

        # Categorize
        if (reason == "no_git_dir" || reason == "empty_refs" || reason == "no_state_refs" || git_refs == 0) {
            # Category 2: Empty/Blank
            print line >> cat2
            count2++
        } else if (state_refs > 0 && state_refs == git_refs && matches == state_refs) {
            # Category 1: Complete Match
            print line >> cat1
            count1++
        } else if (matches > 0 && matches < state_refs) {
            # Category 3: Partial Match
            print line >> cat3
            count3++
        } else if (git_refs > 0 && matches == 0) {
            # Category 4: No Match
            print line >> cat4
            count4++
        } else if (matches > 0) {
            # Edge case: matches > 0 but does not fit other categories
            # This can happen when git_refs > state_refs but all state refs match
            # Treat as partial match
            print line >> cat3
            count3++
        } else {
            # Fallback: treat as category 2 (empty/blank)
            print line >> cat2
            count2++
        }
    }
    END {
        total = count1 + count2 + count3 + count4
        print "COUNTS:" count1 ":" count2 ":" count3 ":" count4 ":" total
    }
    ' "$input_file" 2>&1 | while IFS= read -r line; do
        if [[ "$line" =~ ^COUNTS: ]]; then
            # Parse counts from awk output
            IFS=':' read -r _ c1 c2 c3 c4 total <<< "$line"
            
            echo ""
            log_info "=== Categorization Summary ==="
            log_info "Total entries: $total"
            log_success "Category 1 (Complete Match): $c1"
            log_warn "Category 2 (Empty/Blank): $c2"
            log_warn "Category 3 (Partial Match): $c3"
            log_error "Category 4 (No Match): $c4"
            echo ""
            log_info "Output files:"
            echo "  $cat1"
            echo "  $cat2"
            echo "  $cat3"
            echo "  $cat4"
        fi
    done
}

main "$@"
