#!/usr/bin/env bash
#
# 22-compare-git-data.sh - Compare actual git data between prod and archive relays
#
# PHASE 3c of the GRASP relay to ngit-grasp migration analysis pipeline.
# Compares actual git commits between prod and archive to determine which is ahead.
#
# KEY INSIGHT:
#   Archive (ngit-grasp) enforces GRASP - git data ALWAYS matches a state event.
#   If archive has different/newer data than prod, it means:
#   - A state event authorized those commits at some point
#   - Archive is actually MORE up-to-date than prod
#   - Migration should use archive data (it's already correct)
#
# USAGE:
#   ./22-compare-git-data.sh <prod-git-base> <archive-git-base> <repo-list> <output-dir>
#
# EXAMPLES:
#   ./22-compare-git-data.sh /var/lib/grasp-relay/git /var/lib/ngit-grasp/git \
#       output/comparison/complete-prod-incomplete-archive.txt output/comparison
#
# INPUT:
#   prod-git-base     Base directory for prod git repos (e.g., /var/lib/grasp-relay/git)
#   archive-git-base  Base directory for archive git repos (e.g., /var/lib/ngit-grasp/git)
#   repo-list         File with repos to compare (format: "repo | npub | ...")
#
# OUTPUT:
#   <output-dir>/git-ancestry.tsv - Tab-separated values:
#     repo<TAB>npub<TAB>relationship<TAB>details
#
#   Relationship values:
#     archive-ahead    - Archive has all prod commits plus more (GOOD - use archive)
#     in-sync          - Both have identical commits
#     prod-ahead       - Prod has commits archive is missing (needs re-sync)
#     diverged         - Both have unique commits (manual review)
#     archive-only     - Only archive has git data
#     prod-only        - Only prod has git data
#     both-empty       - Neither has git data
#
# PREREQUISITES:
#   - git (for ref comparison)
#   - Read access to both git directories (may need sudo)
#
# RUNTIME: Depends on number of repos to compare
#
# SEE ALSO:
#   docs/how-to/migrate-to-ngit-grasp.md - Full migration guide
#   21-compare-relays.sh - Phase 3b script that identifies repos to compare
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
    echo -ne "\r${BLUE}[PROGRESS]${NC} $*" >&2
}

usage() {
    echo "Usage: $0 <prod-git-base> <archive-git-base> <repo-list> <output-dir>"
    echo ""
    echo "Arguments:"
    echo "  prod-git-base     Base directory for prod git repos"
    echo "  archive-git-base  Base directory for archive git repos"
    echo "  repo-list         File with repos to compare (format: 'repo | npub | ...')"
    echo "  output-dir        Directory to store output files"
    echo ""
    echo "Examples:"
    echo "  $0 /var/lib/grasp-relay/git /var/lib/ngit-grasp/git \\"
    echo "      output/comparison/complete-prod-incomplete-archive.txt output/comparison"
    echo ""
    echo "Output:"
    echo "  git-ancestry.tsv - TSV with: repo, npub, relationship, details"
    exit 1
}

# Get all branch refs from a git directory
# Args: $1=git_dir
# Returns: sorted list of "ref_name commit_hash" lines
get_git_refs() {
    local git_dir="$1"
    
    if [[ ! -d "$git_dir" ]]; then
        return
    fi
    
    git --git-dir="$git_dir" show-ref --heads 2>/dev/null | sort || true
}

# Check if commit A is ancestor of commit B
# Args: $1=git_dir, $2=commit_a, $3=commit_b
# Returns: 0 if A is ancestor of B, 1 otherwise
is_ancestor() {
    local git_dir="$1"
    local commit_a="$2"
    local commit_b="$3"
    
    git --git-dir="$git_dir" merge-base --is-ancestor "$commit_a" "$commit_b" 2>/dev/null
}

# Compare git data between prod and archive for a single repo
# Args: $1=prod_git_dir, $2=archive_git_dir
# Returns: relationship string
compare_repo_git() {
    local prod_git="$1"
    local archive_git="$2"
    
    local prod_exists=false
    local archive_exists=false
    
    [[ -d "$prod_git" ]] && prod_exists=true
    [[ -d "$archive_git" ]] && archive_exists=true
    
    # Handle cases where one or both don't exist
    if [[ "$prod_exists" == "false" && "$archive_exists" == "false" ]]; then
        echo "both-empty"
        return
    fi
    
    if [[ "$prod_exists" == "false" ]]; then
        echo "archive-only"
        return
    fi
    
    if [[ "$archive_exists" == "false" ]]; then
        echo "prod-only"
        return
    fi
    
    # Both exist - get refs
    local prod_refs archive_refs
    prod_refs=$(get_git_refs "$prod_git")
    archive_refs=$(get_git_refs "$archive_git")
    
    # Handle empty refs
    if [[ -z "$prod_refs" && -z "$archive_refs" ]]; then
        echo "both-empty"
        return
    fi
    
    if [[ -z "$prod_refs" ]]; then
        echo "archive-only"
        return
    fi
    
    if [[ -z "$archive_refs" ]]; then
        echo "prod-only"
        return
    fi
    
    # Compare refs - check if they're identical
    if [[ "$prod_refs" == "$archive_refs" ]]; then
        echo "in-sync"
        return
    fi
    
    # Refs differ - need to check ancestry
    # Strategy: For each branch, check if one is ancestor of the other
    # If all archive branches are ahead of or equal to prod branches, archive is ahead
    # If all prod branches are ahead of or equal to archive branches, prod is ahead
    # Otherwise, they've diverged
    
    local archive_ahead=true
    local prod_ahead=true
    local has_common_branch=false
    
    # Create temporary file to use archive as reference repo for ancestry checks
    # We need a repo that has both sets of commits to check ancestry
    # Use archive since it's the target and should have the superset
    
    # Check each prod branch against archive
    while read -r prod_hash prod_ref; do
        [[ -z "$prod_hash" ]] && continue
        
        # Get the same branch from archive
        local archive_hash
        archive_hash=$(echo "$archive_refs" | grep " $prod_ref$" | awk '{print $1}' || echo "")
        
        if [[ -z "$archive_hash" ]]; then
            # Branch exists in prod but not archive - prod has something archive doesn't
            # But this could be a deleted branch, so don't immediately say prod is ahead
            continue
        fi
        
        has_common_branch=true
        
        if [[ "$prod_hash" == "$archive_hash" ]]; then
            # Same commit - neither ahead for this branch
            continue
        fi
        
        # Different commits - check ancestry
        # First, try to check if prod is ancestor of archive (archive ahead)
        if is_ancestor "$archive_git" "$prod_hash" "$archive_hash" 2>/dev/null; then
            # Prod commit is ancestor of archive commit - archive is ahead for this branch
            prod_ahead=false
        elif is_ancestor "$archive_git" "$archive_hash" "$prod_hash" 2>/dev/null; then
            # Archive commit is ancestor of prod commit - prod is ahead for this branch
            archive_ahead=false
        else
            # Neither is ancestor - diverged
            archive_ahead=false
            prod_ahead=false
        fi
    done <<< "$prod_refs"
    
    # Also check for branches only in archive (archive has extra branches)
    while read -r archive_hash archive_ref; do
        [[ -z "$archive_hash" ]] && continue
        
        local prod_hash
        prod_hash=$(echo "$prod_refs" | grep " $archive_ref$" | awk '{print $1}' || echo "")
        
        if [[ -z "$prod_hash" ]]; then
            # Branch exists in archive but not prod - archive has something prod doesn't
            # This means archive is ahead (has extra branches)
            prod_ahead=false
        fi
    done <<< "$archive_refs"
    
    # Determine final relationship
    if [[ "$has_common_branch" == "false" ]]; then
        # No common branches - completely different
        echo "diverged"
        return
    fi
    
    if [[ "$archive_ahead" == "true" && "$prod_ahead" == "false" ]]; then
        echo "archive-ahead"
    elif [[ "$prod_ahead" == "true" && "$archive_ahead" == "false" ]]; then
        echo "prod-ahead"
    elif [[ "$archive_ahead" == "true" && "$prod_ahead" == "true" ]]; then
        # Both true means all common branches are identical
        # But one might have extra branches
        echo "in-sync"
    else
        echo "diverged"
    fi
}

# Main
main() {
    if [[ $# -ne 4 ]]; then
        usage
    fi
    
    local prod_git_base="$1"
    local archive_git_base="$2"
    local repo_list="$3"
    local output_dir="$4"
    
    # Validate inputs
    if [[ ! -d "$prod_git_base" ]]; then
        log_error "Prod git base directory not found: $prod_git_base"
        exit 1
    fi
    
    if [[ ! -d "$archive_git_base" ]]; then
        log_error "Archive git base directory not found: $archive_git_base"
        exit 1
    fi
    
    if [[ ! -f "$repo_list" ]]; then
        log_error "Repo list file not found: $repo_list"
        exit 1
    fi
    
    log_info "=== Git Data Comparison ==="
    log_info "Prod git base: $prod_git_base"
    log_info "Archive git base: $archive_git_base"
    log_info "Repo list: $repo_list"
    log_info "Output: $output_dir"
    log_info "Started: $(date)"
    echo ""
    
    # Create output directory
    mkdir -p "$output_dir"
    
    # Output file
    local tsv_file="$output_dir/git-ancestry.tsv"
    
    # Initialize TSV with header
    echo -e "repo\tnpub\trelationship\tdetails" > "$tsv_file"
    
    # Count repos
    local total_repos
    total_repos=$(grep -c -v '^#' "$repo_list" 2>/dev/null || echo "0")
    log_info "Processing $total_repos repos..."
    echo ""
    
    # Counters
    local count=0
    local count_archive_ahead=0
    local count_in_sync=0
    local count_prod_ahead=0
    local count_diverged=0
    local count_archive_only=0
    local count_prod_only=0
    local count_both_empty=0
    
    # Process each repo
    while IFS='|' read -r repo npub rest || [[ -n "$repo" ]]; do
        # Skip comments and empty lines
        [[ "$repo" =~ ^# ]] && continue
        [[ -z "$repo" ]] && continue
        
        # Clean up whitespace
        repo="${repo// /}"
        npub="${npub// /}"
        
        [[ -z "$repo" || -z "$npub" ]] && continue
        
        count=$((count + 1))
        
        # Build git paths
        local prod_git="$prod_git_base/${npub}/${repo}.git"
        local archive_git="$archive_git_base/${npub}/${repo}.git"
        
        # Compare
        local relationship details=""
        relationship=$(compare_repo_git "$prod_git" "$archive_git")
        
        # Count by relationship
        case "$relationship" in
            archive-ahead) count_archive_ahead=$((count_archive_ahead + 1)) ;;
            in-sync) count_in_sync=$((count_in_sync + 1)) ;;
            prod-ahead) count_prod_ahead=$((count_prod_ahead + 1)) ;;
            diverged) count_diverged=$((count_diverged + 1)) ;;
            archive-only) count_archive_only=$((count_archive_only + 1)) ;;
            prod-only) count_prod_only=$((count_prod_only + 1)) ;;
            both-empty) count_both_empty=$((count_both_empty + 1)) ;;
        esac
        
        # Output TSV line
        printf '%s\t%s\t%s\t%s\n' "$repo" "$npub" "$relationship" "$details" >> "$tsv_file"
        
        # Progress indicator every 10 repos
        if [[ $((count % 10)) -eq 0 ]]; then
            log_progress "Processed $count/$total_repos repos..."
        fi
    done < "$repo_list"
    
    # Clear progress line
    echo "" >&2
    
    # Summary
    echo ""
    log_info "=== Comparison Summary ==="
    log_success "Archive ahead (use archive data): $count_archive_ahead"
    log_success "In sync: $count_in_sync"
    log_warn "Prod ahead (needs re-sync): $count_prod_ahead"
    log_error "Diverged (manual review): $count_diverged"
    log_info "Archive only: $count_archive_only"
    log_info "Prod only: $count_prod_only"
    log_info "Both empty: $count_both_empty"
    echo ""
    log_info "Total: $count repos"
    log_info "Output: $tsv_file"
}

main "$@"
