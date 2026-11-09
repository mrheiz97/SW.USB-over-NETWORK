#!/usr/bin/env sh
set -eu
tracked="$(git ls-files -- ':(icase,glob)**/AGENTS.md')"
if [ -n "$tracked" ]; then
  echo "ERROR: private AGENTS.md file is tracked" >&2
  exit 1
fi
