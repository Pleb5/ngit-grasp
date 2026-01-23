#!/usr/bin/env bash
#
# validate-service.sh - Validate service name for structured logging
#
# This helper script validates that a service name is appropriate for
# Phase 4 log extraction. Structured logging ([PARSE_FAIL], [PURGATORY_EXPIRED])
# only exists in ngit-grasp services, NOT in ngit-relay services.
#
# USAGE:
#   Source this script and call the validation function:
#
#   source validate-service.sh
#   validate_service_for_structured_logging "$SERVICE_NAME" || exit 1
#
# BACKGROUND:
#   Phase 4 of the migration analysis extracts structured log entries from
#   journald. These log entries only exist in ngit-grasp services. If you
#   accidentally specify an ngit-relay service, Phase 4 will find no logs
#   and produce empty results.
#
#   This validation prevents that common mistake by:
#   1. Checking if the service name contains "ngit-relay" (error)
#   2. Warning if the service name doesn't contain "ngit-grasp"
#   3. Optionally checking if structured logs actually exist
#
# SEE ALSO:
#   docs/how-to/migrate-to-ngit-grasp.md - Full migration guide
#   30-extract-parse-failures.sh - Uses this validation
#   31-extract-purgatory-expiry.sh - Uses this validation
#

# Colors for output (disabled if not a terminal)
if [[ -t 1 ]]; then
    _VS_RED='\033[0;31m'
    _VS_YELLOW='\033[0;33m'
    _VS_NC='\033[0m'
else
    _VS_RED=''
    _VS_YELLOW=''
    _VS_NC=''
fi

# Validates that the service name is appropriate for structured logging
#
# Arguments:
#   $1 - service_name: The systemd service name to validate
#   $2 - check_logs: Whether to check if logs actually exist (default: "true")
#   $3 - interactive: Whether to prompt for confirmation (default: "true")
#
# Returns:
#   0 - Service is valid for structured logging
#   1 - Service is invalid or user declined to continue
#
# Example:
#   validate_service_for_structured_logging "ngit-grasp.service" || exit 1
#   validate_service_for_structured_logging "ngit-grasp.service" "false"  # Skip log check
#   validate_service_for_structured_logging "ngit-grasp.service" "true" "false"  # Non-interactive
#
validate_service_for_structured_logging() {
    local service_name="$1"
    local check_logs="${2:-true}"
    local interactive="${3:-true}"
    
    # Check if service name looks like ngit-relay (ERROR - wrong service type)
    if [[ "$service_name" == *"ngit-relay"* ]]; then
        echo -e "${_VS_RED}ERROR: Service name appears to be ngit-relay: $service_name${_VS_NC}" >&2
        echo "" >&2
        echo "Structured logging ([PARSE_FAIL], [PURGATORY_EXPIRED]) only exists in" >&2
        echo "ngit-grasp services, NOT in ngit-relay services." >&2
        echo "" >&2
        echo "Please use the ngit-grasp archive service instead." >&2
        echo "" >&2
        echo "To find the correct service name:" >&2
        echo "  systemctl list-units 'ngit-grasp*' --all" >&2
        echo "" >&2
        echo "Common ngit-grasp service names:" >&2
        echo "  - ngit-grasp.service" >&2
        echo "  - ngit-grasp-relay-ngit-dev.service (NixOS multi-instance)" >&2
        echo "  - ngit-grasp-archive.service" >&2
        return 1
    fi
    
    # Check if service name looks like ngit-grasp (WARNING if not)
    if [[ "$service_name" != *"ngit-grasp"* && "$service_name" != *"grasp"* ]]; then
        echo -e "${_VS_YELLOW}WARNING: Service name doesn't contain 'ngit-grasp': $service_name${_VS_NC}" >&2
        echo "" >&2
        echo "Structured logging ([PARSE_FAIL], [PURGATORY_EXPIRED]) only exists in" >&2
        echo "ngit-grasp services." >&2
        echo "" >&2
        
        if [[ "$interactive" == "true" ]]; then
            read -p "Continue anyway? (y/N) " -n 1 -r
            echo
            if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                return 1
            fi
        else
            echo "Non-interactive mode: proceeding despite warning" >&2
        fi
    fi
    
    # Optionally check if structured logs actually exist
    if [[ "$check_logs" == "true" ]]; then
        # Check if journalctl is available
        if ! command -v journalctl &> /dev/null; then
            echo -e "${_VS_YELLOW}WARNING: journalctl not available, cannot verify logs exist${_VS_NC}" >&2
            return 0
        fi
        
        # Check for structured log entries
        local has_parse_fail has_purgatory
        has_parse_fail=$(journalctl -u "$service_name" --since "7 days ago" 2>/dev/null | grep -c '\[PARSE_FAIL\]' || echo "0")
        has_purgatory=$(journalctl -u "$service_name" --since "7 days ago" 2>/dev/null | grep -c '\[PURGATORY_EXPIRED\]' || echo "0")
        
        # Strip any non-numeric characters (grep -c can have trailing whitespace)
        has_parse_fail="${has_parse_fail//[^0-9]/}"
        has_purgatory="${has_purgatory//[^0-9]/}"
        has_parse_fail="${has_parse_fail:-0}"
        has_purgatory="${has_purgatory:-0}"
        
        if [[ "$has_parse_fail" -eq 0 && "$has_purgatory" -eq 0 ]]; then
            echo -e "${_VS_YELLOW}WARNING: No structured logs found in $service_name (last 7 days)${_VS_NC}" >&2
            echo "" >&2
            echo "This may indicate:" >&2
            echo "  1. Wrong service (should be ngit-grasp archive service, not ngit-relay)" >&2
            echo "  2. Structured logging not yet deployed to this ngit-grasp instance" >&2
            echo "  3. No parse failures or purgatory expiry events in the time window" >&2
            echo "" >&2
            echo "To verify you have the right service:" >&2
            echo "  systemctl list-units 'ngit-grasp*' --all" >&2
            echo "  journalctl -u <service> | grep -E '\\[PARSE_FAIL\\]|\\[PURGATORY_EXPIRED\\]' | head -5" >&2
            echo "" >&2
            
            if [[ "$interactive" == "true" ]]; then
                read -p "Continue anyway? (y/N) " -n 1 -r
                echo
                if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                    return 1
                fi
            else
                echo "Non-interactive mode: proceeding despite warning" >&2
            fi
        fi
    fi
    
    return 0
}

# Export the function so it can be used after sourcing
export -f validate_service_for_structured_logging
