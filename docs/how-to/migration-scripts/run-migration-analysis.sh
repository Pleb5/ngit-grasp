#!/usr/bin/env bash
#
# run-migration-analysis.sh - Orchestrate the complete GRASP relay to ngit-grasp migration analysis
#
# This script runs all 5 phases of the migration analysis pipeline in sequence,
# with proper error handling, progress reporting, and timing information.
#
# QUICK START:
#   # Basic usage (local analysis only - Phases 1, 3, 5)
#   ./run-migration-analysis.sh --prod-relay wss://relay.ngit.dev --archive-relay wss://archive.relay.ngit.dev
#
#   # Full analysis including git sync check (requires VPS access)
#   ./run-migration-analysis.sh \
#     --prod-relay wss://relay.ngit.dev \
#     --archive-relay wss://archive.relay.ngit.dev \
#     --prod-git /var/lib/grasp-relay/git \
#     --archive-git /var/lib/ngit-grasp/git
#
# USAGE:
#   ./run-migration-analysis.sh [options]
#
# REQUIRED OPTIONS:
#   --prod-relay <url>      Production relay WebSocket URL (e.g., wss://relay.ngit.dev)
#   --archive-relay <url>   Archive relay WebSocket URL (e.g., wss://archive.relay.ngit.dev)
#
# OPTIONAL OPTIONS:
#   --prod-git <path>       Git base directory for prod (enables Phase 2)
#   --archive-git <path>    Git base directory for archive (enables Phase 2)
#   --service <name>        Systemd service name for log extraction (enables Phase 4)
#   --output <dir>          Output directory (default: work/migration-analysis-YYYYMMDD-HHMM)
#   --since <date>          Start date for log extraction (default: 30 days ago)
#   --until <date>          End date for log extraction (default: now)
#
# PHASE CONTROL:
#   --skip-phase-1          Skip event fetching (use existing data)
#   --skip-phase-2          Skip git sync check (use existing data)
#   --skip-phase-3          Skip categorization (use existing data)
#   --skip-phase-4          Skip log extraction (use existing data)
#   --skip-phase-5          Skip final classification
#   --only-phase-N          Run only phase N (1-5)
#   --from-phase-N          Start from phase N (skip earlier phases)
#
# OTHER OPTIONS:
#   --dry-run               Show what would be executed without running
#   --continue-on-error     Continue to next phase even if current phase fails
#   --help                  Show this help message
#
# PHASES:
#   Phase 1: Fetch events from both relays (~30s each, local)
#   Phase 2: Check git sync status (~20 min each, requires VPS)
#   Phase 3: Categorize and compare results (fast, local)
#   Phase 4: Extract logs from systemd (requires VPS)
#   Phase 5: Final classification (fast, local)
#
# EXAMPLES:
#   # Dry run to see what would happen
#   ./run-migration-analysis.sh --prod-relay wss://relay.ngit.dev --archive-relay wss://archive.relay.ngit.dev --dry-run
#
#   # Run only Phase 1 (fetch events)
#   ./run-migration-analysis.sh --prod-relay wss://relay.ngit.dev --archive-relay wss://archive.relay.ngit.dev --only-phase-1
#
#   # Resume from Phase 3 using existing Phase 1-2 data
#   ./run-migration-analysis.sh --prod-relay wss://relay.ngit.dev --archive-relay wss://archive.relay.ngit.dev --from-phase-3 --output work/migration-analysis-20260122-1430
#
#   # Full analysis on VPS with all features
#   ./run-migration-analysis.sh \
#     --prod-relay wss://relay.ngit.dev \
#     --archive-relay wss://archive.relay.ngit.dev \
#     --prod-git /var/lib/grasp-relay/git \
#     --archive-git /var/lib/ngit-grasp/git \
#     --service ngit-grasp.service
#
# SEE ALSO:
#   docs/how-to/migrate-to-ngit-grasp.md - Full migration guide
#

set -euo pipefail

# Get script directory for finding other scripts
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Colors for output (disabled if not a terminal)
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    CYAN='\033[0;36m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    CYAN=''
    BOLD=''
    NC=''
fi

# Logging functions
log_header() {
    echo ""
    echo -e "${BOLD}${CYAN}════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${CYAN}  $*${NC}"
    echo -e "${BOLD}${CYAN}════════════════════════════════════════════════════════════════${NC}"
    echo ""
}

log_phase() {
    echo ""
    echo -e "${BOLD}${BLUE}┌──────────────────────────────────────────────────────────────┐${NC}"
    echo -e "${BOLD}${BLUE}│  $*${NC}"
    echo -e "${BOLD}${BLUE}└──────────────────────────────────────────────────────────────┘${NC}"
}

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

log_step() {
    echo -e "${CYAN}  →${NC} $*" >&2
}

# Default values
PROD_RELAY=""
ARCHIVE_RELAY=""
PROD_GIT=""
ARCHIVE_GIT=""
SERVICE_NAME=""
OUTPUT_DIR=""
DRY_RUN=false
CONTINUE_ON_ERROR=false
LOG_SINCE=""
LOG_UNTIL=""

# Phase control
SKIP_PHASE_1=false
SKIP_PHASE_2=false
SKIP_PHASE_3=false
SKIP_PHASE_4=false
SKIP_PHASE_5=false
ONLY_PHASE=""
FROM_PHASE=""

# Timing
declare -A PHASE_TIMES

usage() {
    head -73 "$0" | tail -n +3 | sed 's/^# //' | sed 's/^#//'
    exit 0
}

# Parse command line arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --prod-relay)
                PROD_RELAY="$2"
                shift 2
                ;;
            --archive-relay)
                ARCHIVE_RELAY="$2"
                shift 2
                ;;
            --prod-git)
                PROD_GIT="$2"
                shift 2
                ;;
            --archive-git)
                ARCHIVE_GIT="$2"
                shift 2
                ;;
            --service)
                SERVICE_NAME="$2"
                shift 2
                ;;
            --output)
                OUTPUT_DIR="$2"
                shift 2
                ;;
            --skip-phase-1)
                SKIP_PHASE_1=true
                shift
                ;;
            --skip-phase-2)
                SKIP_PHASE_2=true
                shift
                ;;
            --skip-phase-3)
                SKIP_PHASE_3=true
                shift
                ;;
            --skip-phase-4)
                SKIP_PHASE_4=true
                shift
                ;;
            --skip-phase-5)
                SKIP_PHASE_5=true
                shift
                ;;
            --only-phase-1|--only-phase-2|--only-phase-3|--only-phase-4|--only-phase-5)
                ONLY_PHASE="${1#--only-phase-}"
                shift
                ;;
            --from-phase-1|--from-phase-2|--from-phase-3|--from-phase-4|--from-phase-5)
                FROM_PHASE="${1#--from-phase-}"
                shift
                ;;
            --dry-run)
                DRY_RUN=true
                shift
                ;;
            --continue-on-error)
                CONTINUE_ON_ERROR=true
                shift
                ;;
            --since)
                LOG_SINCE="$2"
                shift 2
                ;;
            --until)
                LOG_UNTIL="$2"
                shift 2
                ;;
            --help|-h)
                usage
                ;;
            *)
                log_error "Unknown option: $1"
                echo "Use --help for usage information."
                exit 1
                ;;
        esac
    done
}

# Validate required arguments
validate_args() {
    local errors=0
    
    if [[ -z "$PROD_RELAY" ]]; then
        log_error "Missing required option: --prod-relay"
        errors=1
    fi
    
    if [[ -z "$ARCHIVE_RELAY" ]]; then
        log_error "Missing required option: --archive-relay"
        errors=1
    fi
    
    # Validate relay URLs
    if [[ -n "$PROD_RELAY" && ! "$PROD_RELAY" =~ ^wss?:// ]]; then
        log_error "Invalid prod relay URL: $PROD_RELAY (must start with ws:// or wss://)"
        errors=1
    fi
    
    if [[ -n "$ARCHIVE_RELAY" && ! "$ARCHIVE_RELAY" =~ ^wss?:// ]]; then
        log_error "Invalid archive relay URL: $ARCHIVE_RELAY (must start with ws:// or wss://)"
        errors=1
    fi
    
    # Validate git paths if provided
    if [[ -n "$PROD_GIT" && ! -d "$PROD_GIT" ]]; then
        log_warn "Prod git directory not found: $PROD_GIT"
        log_warn "Phase 2 will fail unless running on VPS with access to this path."
    fi
    
    if [[ -n "$ARCHIVE_GIT" && ! -d "$ARCHIVE_GIT" ]]; then
        log_warn "Archive git directory not found: $ARCHIVE_GIT"
        log_warn "Phase 2 will fail unless running on VPS with access to this path."
    fi
    
    if [[ $errors -eq 1 ]]; then
        echo ""
        echo "Use --help for usage information."
        exit 1
    fi
}

# Check prerequisites
check_prerequisites() {
    local missing=0
    
    log_info "Checking prerequisites..."
    
    # Required tools
    for tool in git nak jq awk sort; do
        if command -v "$tool" &> /dev/null; then
            log_step "$tool: found"
        else
            log_error "$tool: NOT FOUND"
            missing=1
        fi
    done
    
    # Optional tools
    if command -v journalctl &> /dev/null; then
        log_step "journalctl: found (Phase 4 available)"
    else
        log_step "journalctl: not found (Phase 4 will be skipped)"
        SKIP_PHASE_4=true
    fi
    
    if [[ $missing -eq 1 ]]; then
        log_error "Missing required tools. Install them and try again."
        exit 1
    fi
    
    # Check scripts exist
    for script in 01-fetch-events.sh 10-check-git-sync.sh 20-categorize.sh 21-compare-relays.sh 22-compare-git-data.sh 30-extract-parse-failures.sh 31-extract-purgatory-expiry.sh 40-classify-actions.sh; do
        if [[ ! -x "$SCRIPT_DIR/$script" ]]; then
            log_error "Script not found or not executable: $SCRIPT_DIR/$script"
            missing=1
        fi
    done
    
    if [[ $missing -eq 1 ]]; then
        exit 1
    fi
    
    log_success "All prerequisites satisfied"
}

# Determine which phases to run
determine_phases() {
    # Handle --only-phase-N
    if [[ -n "$ONLY_PHASE" ]]; then
        for i in 1 2 3 4 5; do
            if [[ "$i" != "$ONLY_PHASE" ]]; then
                eval "SKIP_PHASE_$i=true"
            fi
        done
    fi
    
    # Handle --from-phase-N
    if [[ -n "$FROM_PHASE" ]]; then
        for i in 1 2 3 4 5; do
            if [[ "$i" -lt "$FROM_PHASE" ]]; then
                eval "SKIP_PHASE_$i=true"
            fi
        done
    fi
    
    # Auto-skip Phase 2 if git paths not provided
    if [[ -z "$PROD_GIT" && -z "$ARCHIVE_GIT" ]]; then
        if [[ "$SKIP_PHASE_2" != "true" ]]; then
            log_warn "No git paths provided. Phase 2 (git sync check) will be skipped."
            log_warn "Use --prod-git and --archive-git to enable Phase 2."
            SKIP_PHASE_2=true
        fi
    fi
    
    # Auto-skip Phase 4 if service not provided
    if [[ -z "$SERVICE_NAME" ]]; then
        if [[ "$SKIP_PHASE_4" != "true" ]]; then
            log_warn "No service name provided. Phase 4 (log extraction) will be skipped."
            log_warn "Use --service to enable Phase 4."
            SKIP_PHASE_4=true
        fi
    fi
}

# Setup output directory
setup_output_dir() {
    if [[ -z "$OUTPUT_DIR" ]]; then
        OUTPUT_DIR="work/migration-analysis-$(date +%Y%m%d-%H%M)"
    fi
    
    log_info "Output directory: $OUTPUT_DIR"
    
    if [[ "$DRY_RUN" == "true" ]]; then
        log_info "[DRY RUN] Would create directory structure"
        return
    fi
    
    mkdir -p "$OUTPUT_DIR"/{prod/raw,archive/raw,comparison,logs,results}
    
    # Save configuration
    cat > "$OUTPUT_DIR/config.txt" << EOF
# Migration Analysis Configuration
# Generated: $(date -Iseconds)

PROD_RELAY=$PROD_RELAY
ARCHIVE_RELAY=$ARCHIVE_RELAY
PROD_GIT=$PROD_GIT
ARCHIVE_GIT=$ARCHIVE_GIT
SERVICE_NAME=$SERVICE_NAME
OUTPUT_DIR=$OUTPUT_DIR
EOF
    
    log_success "Created output directory structure"
}

# Run a phase with timing and error handling
run_phase() {
    local phase_num="$1"
    local phase_name="$2"
    shift 2
    local cmd=("$@")
    
    local skip_var="SKIP_PHASE_$phase_num"
    if [[ "${!skip_var}" == "true" ]]; then
        log_phase "Phase $phase_num: $phase_name [SKIPPED]"
        return 0
    fi
    
    log_phase "Phase $phase_num: $phase_name"
    
    if [[ "$DRY_RUN" == "true" ]]; then
        log_info "[DRY RUN] Would execute:"
        for c in "${cmd[@]}"; do
            echo "  $c"
        done
        return 0
    fi
    
    local start_time
    start_time=$(date +%s)
    
    local exit_code=0
    
    # Execute the command(s)
    for c in "${cmd[@]}"; do
        log_step "Running: $c"
        if ! eval "$c"; then
            exit_code=1
            if [[ "$CONTINUE_ON_ERROR" == "true" ]]; then
                log_warn "Command failed, continuing due to --continue-on-error"
            else
                log_error "Command failed"
                break
            fi
        fi
    done
    
    local end_time
    end_time=$(date +%s)
    local duration=$((end_time - start_time))
    PHASE_TIMES[$phase_num]=$duration
    
    if [[ $exit_code -eq 0 ]]; then
        log_success "Phase $phase_num completed in ${duration}s"
    else
        log_error "Phase $phase_num failed after ${duration}s"
        if [[ "$CONTINUE_ON_ERROR" != "true" ]]; then
            return 1
        fi
    fi
    
    return $exit_code
}

# Phase 1: Fetch events
run_phase_1() {
    local cmds=()
    
    # Fetch from prod relay
    cmds+=("'$SCRIPT_DIR/01-fetch-events.sh' '$PROD_RELAY' '$OUTPUT_DIR/prod'")
    
    # Fetch from archive relay
    cmds+=("'$SCRIPT_DIR/01-fetch-events.sh' '$ARCHIVE_RELAY' '$OUTPUT_DIR/archive'")
    
    run_phase 1 "Fetch Events (~30s each)" "${cmds[@]}"
}

# Phase 2: Git sync check
run_phase_2() {
    local cmds=()
    
    if [[ -n "$PROD_GIT" ]]; then
        cmds+=("'$SCRIPT_DIR/10-check-git-sync.sh' '$OUTPUT_DIR/prod/raw/state-events.json' '$PROD_GIT' '$OUTPUT_DIR/prod' --categorize")
    else
        log_warn "Skipping prod git sync check (no --prod-git provided)"
    fi
    
    if [[ -n "$ARCHIVE_GIT" ]]; then
        cmds+=("'$SCRIPT_DIR/10-check-git-sync.sh' '$OUTPUT_DIR/archive/raw/state-events.json' '$ARCHIVE_GIT' '$OUTPUT_DIR/archive' --categorize")
    else
        log_warn "Skipping archive git sync check (no --archive-git provided)"
    fi
    
    if [[ ${#cmds[@]} -eq 0 ]]; then
        log_warn "No git paths provided, skipping Phase 2"
        return 0
    fi
    
    run_phase 2 "Git Sync Check (~20 min each)" "${cmds[@]}"
}

# Phase 3: Categorize and compare
run_phase_3() {
    local cmds=()
    
    # Check if we have git-sync-status.tsv files (from Phase 2)
    # If not, we can't run categorization
    local has_prod_sync=false
    local has_archive_sync=false
    
    if [[ -f "$OUTPUT_DIR/prod/git-sync-status.tsv" ]]; then
        has_prod_sync=true
    fi
    
    if [[ -f "$OUTPUT_DIR/archive/git-sync-status.tsv" ]]; then
        has_archive_sync=true
    fi
    
    # Run categorization if we have sync data but no category files
    if [[ "$has_prod_sync" == "true" && ! -f "$OUTPUT_DIR/prod/category1-complete-match.txt" ]]; then
        cmds+=("'$SCRIPT_DIR/20-categorize.sh' '$OUTPUT_DIR/prod/git-sync-status.tsv' '$OUTPUT_DIR/prod'")
    fi
    
    if [[ "$has_archive_sync" == "true" && ! -f "$OUTPUT_DIR/archive/category1-complete-match.txt" ]]; then
        cmds+=("'$SCRIPT_DIR/20-categorize.sh' '$OUTPUT_DIR/archive/git-sync-status.tsv' '$OUTPUT_DIR/archive'")
    fi
    
    # Run comparison if we have category files
    if [[ -f "$OUTPUT_DIR/prod/category1-complete-match.txt" && -f "$OUTPUT_DIR/archive/category1-complete-match.txt" ]]; then
        cmds+=("'$SCRIPT_DIR/21-compare-relays.sh' '$OUTPUT_DIR/prod' '$OUTPUT_DIR/archive' '$OUTPUT_DIR/comparison'")
    else
        log_warn "Missing category files for comparison."
        log_warn "Phase 2 must complete successfully before Phase 3 can compare relays."
        
        # Create placeholder comparison files if they don't exist
        if [[ "$DRY_RUN" != "true" ]]; then
            mkdir -p "$OUTPUT_DIR/comparison"
            for f in complete-in-both.txt complete-prod-missing-archive.txt complete-prod-incomplete-archive.txt incomplete-in-both.txt in-archive-not-prod.txt; do
                if [[ ! -f "$OUTPUT_DIR/comparison/$f" ]]; then
                    echo "# Placeholder - Phase 2 data not available" > "$OUTPUT_DIR/comparison/$f"
                fi
            done
            echo "# Comparison not available - Phase 2 data missing" > "$OUTPUT_DIR/comparison/summary.txt"
        fi
    fi
    
    if [[ ${#cmds[@]} -eq 0 ]]; then
        log_warn "No categorization or comparison needed (already done or missing input)"
        return 0
    fi
    
    run_phase 3 "Categorize & Compare (fast)" "${cmds[@]}"
    
    # Phase 3c: Compare git data between relays (requires git paths)
    # This determines if archive is ahead of prod for repos with mismatched state
    if [[ -n "$PROD_GIT" && -n "$ARCHIVE_GIT" ]]; then
        # Build list of repos to compare: those where prod=complete but archive is not
        local repos_to_compare="$OUTPUT_DIR/comparison/complete-prod-incomplete-archive.txt"
        if [[ -f "$repos_to_compare" ]] && [[ ! -f "$OUTPUT_DIR/comparison/git-ancestry.tsv" ]]; then
            log_info "Running git ancestry comparison (Phase 3c)..."
            run_phase 3 "Git Ancestry Comparison" "'$SCRIPT_DIR/22-compare-git-data.sh' '$PROD_GIT' '$ARCHIVE_GIT' '$repos_to_compare' '$OUTPUT_DIR/comparison'"
        fi
    else
        log_warn "Git paths not provided - skipping git ancestry comparison"
        log_warn "Without git comparison, repos where archive is ahead will be incorrectly flagged as needing re-sync"
    fi
}

# Phase 4: Extract logs
run_phase_4() {
    if [[ -z "$SERVICE_NAME" ]]; then
        log_warn "No service name provided, skipping Phase 4"
        return 0
    fi
    
    # Validate service name before running Phase 4
    # Structured logging only exists in ngit-grasp, not ngit-relay
    if [[ "$SERVICE_NAME" == *"ngit-relay"* ]]; then
        log_error "SERVICE_NAME appears to be ngit-relay: $SERVICE_NAME"
        log_error ""
        log_error "Phase 4 requires an ngit-grasp service with structured logging."
        log_error "Structured logging ([PARSE_FAIL], [PURGATORY_EXPIRED]) only exists"
        log_error "in ngit-grasp services, NOT in ngit-relay services."
        log_error ""
        log_error "Please update --service to use the ngit-grasp archive service."
        log_error ""
        log_error "To find the correct service name:"
        log_error "  systemctl list-units 'ngit-grasp*' --all"
        log_error ""
        log_error "Common ngit-grasp service names:"
        log_error "  - ngit-grasp.service"
        log_error "  - ngit-grasp-relay-ngit-dev.service (NixOS multi-instance)"
        log_error "  - ngit-grasp-archive.service"
        return 1
    fi
    
    # Warn if service name doesn't look like ngit-grasp
    if [[ "$SERVICE_NAME" != *"ngit-grasp"* && "$SERVICE_NAME" != *"grasp"* ]]; then
        log_warn "SERVICE_NAME doesn't contain 'ngit-grasp': $SERVICE_NAME"
        log_warn "Structured logging only exists in ngit-grasp services."
        log_warn "If this is not an ngit-grasp service, Phase 4 will find no logs."
    fi
    
    local cmds=()
    
    # Build log extraction options
    local log_opts=""
    if [[ -n "$LOG_SINCE" ]]; then
        log_opts="$log_opts --since '$LOG_SINCE'"
    fi
    if [[ -n "$LOG_UNTIL" ]]; then
        log_opts="$log_opts --until '$LOG_UNTIL'"
    fi
    
    cmds+=("'$SCRIPT_DIR/30-extract-parse-failures.sh' '$SERVICE_NAME' '$OUTPUT_DIR/logs' $log_opts")
    cmds+=("'$SCRIPT_DIR/31-extract-purgatory-expiry.sh' '$SERVICE_NAME' '$OUTPUT_DIR/logs' $log_opts")
    
    run_phase 4 "Extract Logs (VPS required)" "${cmds[@]}"
}

# Phase 5: Final classification
run_phase_5() {
    # Check if we have the minimum required files
    local can_run=true
    
    if [[ ! -d "$OUTPUT_DIR/prod" ]]; then
        log_warn "Missing prod directory"
        can_run=false
    fi
    
    if [[ ! -d "$OUTPUT_DIR/archive" ]]; then
        log_warn "Missing archive directory"
        can_run=false
    fi
    
    if [[ ! -d "$OUTPUT_DIR/comparison" ]]; then
        log_warn "Missing comparison directory"
        can_run=false
    fi
    
    # Create logs directory with empty files if missing
    if [[ "$DRY_RUN" != "true" ]]; then
        mkdir -p "$OUTPUT_DIR/logs"
        for f in parse-failures.txt purgatory-expired.txt; do
            if [[ ! -f "$OUTPUT_DIR/logs/$f" ]]; then
                echo "# No data - Phase 4 not run" > "$OUTPUT_DIR/logs/$f"
            fi
        done
    fi
    
    if [[ "$can_run" == "false" ]]; then
        log_error "Cannot run Phase 5 - missing required input directories"
        return 1
    fi
    
    run_phase 5 "Final Classification (fast)" "'$SCRIPT_DIR/40-classify-actions.sh' '$OUTPUT_DIR'"
}

# Display summary
display_summary() {
    log_header "Migration Analysis Complete"
    
    echo "Output Directory: $OUTPUT_DIR"
    echo ""
    
    # Phase timing summary
    echo "Phase Timing:"
    local total_time=0
    for phase in 1 2 3 4 5; do
        local skip_var="SKIP_PHASE_$phase"
        if [[ "${!skip_var}" == "true" ]]; then
            echo "  Phase $phase: SKIPPED"
        elif [[ -n "${PHASE_TIMES[$phase]:-}" ]]; then
            local t="${PHASE_TIMES[$phase]}"
            echo "  Phase $phase: ${t}s"
            total_time=$((total_time + t))
        else
            echo "  Phase $phase: N/A"
        fi
    done
    echo "  ─────────────"
    echo "  Total: ${total_time}s"
    echo ""
    
    # Results summary
    if [[ -f "$OUTPUT_DIR/results/summary.txt" ]]; then
        echo "Results Summary:"
        echo ""
        # Extract key metrics from summary
        if grep -q "No Action Required" "$OUTPUT_DIR/results/summary.txt"; then
            grep -A1 "No Action Required" "$OUTPUT_DIR/results/summary.txt" | head -2
        fi
        if grep -q "Action Required" "$OUTPUT_DIR/results/summary.txt"; then
            grep -A1 "Action Required" "$OUTPUT_DIR/results/summary.txt" | head -2
        fi
        if grep -q "Manual Investigation" "$OUTPUT_DIR/results/summary.txt"; then
            grep -A1 "Manual Investigation" "$OUTPUT_DIR/results/summary.txt" | head -2
        fi
        echo ""
    fi
    
    # Output files
    echo "Output Files:"
    echo "  $OUTPUT_DIR/results/no-action-required.txt"
    echo "  $OUTPUT_DIR/results/action-required.txt"
    echo "  $OUTPUT_DIR/results/manual-investigation.txt"
    echo "  $OUTPUT_DIR/results/summary.txt"
    echo ""
    
    # Next steps
    echo "Next Steps:"
    echo "  1. Review results/summary.txt for overview"
    echo "  2. Address items in results/action-required.txt"
    echo "  3. Investigate items in results/manual-investigation.txt"
    echo "  4. Plan migration window when action items are resolved"
    echo ""
}

# Main
main() {
    parse_args "$@"
    
    log_header "GRASP Relay to ngit-grasp Migration Analysis"
    
    validate_args
    check_prerequisites
    determine_phases
    setup_output_dir
    
    # Show configuration
    log_info "Configuration:"
    log_step "Prod relay: $PROD_RELAY"
    log_step "Archive relay: $ARCHIVE_RELAY"
    [[ -n "$PROD_GIT" ]] && log_step "Prod git: $PROD_GIT"
    [[ -n "$ARCHIVE_GIT" ]] && log_step "Archive git: $ARCHIVE_GIT"
    [[ -n "$SERVICE_NAME" ]] && log_step "Service: $SERVICE_NAME"
    log_step "Output: $OUTPUT_DIR"
    echo ""
    
    # Show phase plan
    log_info "Phase Plan:"
    for phase in 1 2 3 4 5; do
        local skip_var="SKIP_PHASE_$phase"
        if [[ "${!skip_var}" == "true" ]]; then
            log_step "Phase $phase: SKIP"
        else
            log_step "Phase $phase: RUN"
        fi
    done
    echo ""
    
    if [[ "$DRY_RUN" == "true" ]]; then
        log_warn "DRY RUN MODE - No changes will be made"
        echo ""
    fi
    
    # Run phases
    local overall_exit=0
    
    run_phase_1 || overall_exit=1
    run_phase_2 || overall_exit=1
    run_phase_3 || overall_exit=1
    run_phase_4 || overall_exit=1
    run_phase_5 || overall_exit=1
    
    # Display summary
    if [[ "$DRY_RUN" != "true" ]]; then
        display_summary
    fi
    
    if [[ $overall_exit -ne 0 ]]; then
        log_warn "Some phases failed. Review output for details."
    fi
    
    exit $overall_exit
}

main "$@"
