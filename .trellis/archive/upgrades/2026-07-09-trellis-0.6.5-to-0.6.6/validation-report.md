# Validation Report

## Audit Matrix

- source-version-check: tier=required, classification=run, ok=True, note=source versions agree
- package-manager-detection: tier=required, classification=run, ok=True, note=selected package manager detected
- trellis-inventory: tier=required, classification=run, ok=True, note=requires initialization decision if missing
- structured-config-parse: tier=required, classification=run, ok=True, note=0 parse failures; 0 parser unavailable
- static-hook-detection: tier=required, classification=run, ok=False, note=1300 runnable-looking hooks; 22 static-only hooks
- cli-basic: tier=required, classification=run, ok=None, note=run with --run-basic-checks to execute trellis --version and --help
- safe-write-path: tier=safe-write, classification=confirm, ok=None, note=run only when Trellis dry-run is available; otherwise static-only and skipped
- remote-or-expensive-hooks: tier=optional, classification=static-only, ok=None, note=list only; do not execute by default

## Post-Upgrade Checks

Fill this section after the upgrade and post-upgrade audit.
