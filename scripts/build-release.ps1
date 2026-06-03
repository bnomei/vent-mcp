<#
Builds the Windows release binary for the requested Rust target.

This is the Windows companion to the shell build script. It keeps CI invocation
simple by requiring only the target triple and delegating the actual build to
Cargo with release optimization enabled.
#>
param(
  [Parameter(Mandatory = $true)][string]$Target
)

$ErrorActionPreference = 'Stop'

cargo build --release --target $Target
