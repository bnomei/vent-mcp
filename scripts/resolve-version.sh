#!/usr/bin/env bash
#
# Resolves the release version for CI and local packaging commands.
#
# Tagged GitHub releases use the tag name without a leading `v`; untagged runs
# read the Cargo package version. When GITHUB_OUTPUT is present, the script emits
# the value in GitHub Actions output format so later release steps share exactly
# the same version string.
set -euo pipefail

if [[ "${GITHUB_REF_NAME:-}" == v* ]]; then
  version="${GITHUB_REF_NAME#v}"
else
  package_id=$(cargo pkgid)
  version="${package_id##*#}"
  version="${version##*@}"
fi

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "version=${version}" >> "$GITHUB_OUTPUT"
else
  echo "$version"
fi
