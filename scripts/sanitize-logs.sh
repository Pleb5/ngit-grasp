#!/bin/bash
# sanitize-logs.sh - Truncates verbose log lines for LLM analysis
#
# Usage:
#   cargo run -- [args] 2>&1 | ./scripts/sanitize-logs.sh
#   ./scripts/sanitize-logs.sh < logfile.txt
#   ./scripts/sanitize-logs.sh --head-chars 150 --tail-chars 30 < logfile.txt

set -euo pipefail

# Default settings
HEAD_CHARS=100
TAIL_CHARS=20
MAX_LINE_LENGTH=$((HEAD_CHARS + TAIL_CHARS + 20))  # buffer for the ellipsis marker

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --head-chars)
            HEAD_CHARS="$2"
            shift 2
            ;;
        --tail-chars)
            TAIL_CHARS="$2"
            shift 2
            ;;
        --max-line)
            MAX_LINE_LENGTH="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Sanitizes log output for LLM analysis by truncating long lines."
            echo "Reads from stdin, writes to stdout."
            echo ""
            echo "Options:"
            echo "  --head-chars N    Show first N chars of long lines (default: 100)"
            echo "  --tail-chars N    Show last N chars of long lines (default: 20)"
            echo "  --max-line N      Lines shorter than this are unchanged (default: head+tail+20)"
            echo "  -h, --help        Show this help"
            echo ""
            echo "Examples:"
            echo "  cargo run -- --sync-bootstrap-relay wss://git.shakespeare.diy 2>&1 | $0"
            echo "  timeout 30s cargo run -- [args] 2>&1 | $0 > sanitized.log"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

MAX_LINE_LENGTH=$((HEAD_CHARS + TAIL_CHARS + 20))

# Process each line
while IFS= read -r line; do
    len=${#line}
    
    if [[ $len -le $MAX_LINE_LENGTH ]]; then
        # Short line - pass through unchanged
        echo "$line"
    else
        # Long line - truncate with marker showing omitted char count
        head="${line:0:$HEAD_CHARS}"
        tail="${line: -$TAIL_CHARS}"
        omitted=$((len - HEAD_CHARS - TAIL_CHARS))
        echo "${head}...<${omitted} chars>...${tail}"
    fi
done
