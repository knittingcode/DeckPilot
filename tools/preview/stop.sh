#!/usr/bin/env bash
# Stops only the preview's session (its data in .dev/data is kept).
. "$(dirname "$0")/lib.sh"
dev down
