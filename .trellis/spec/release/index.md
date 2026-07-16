# Release Specifications

Project-specific contracts for preparing and publishing Agent-Switch releases.

## Pre-Development Checklist

- Read [GitHub Actions Release](github-actions-release.md) before changing versions, tags, signing, updater artifacts, or `.github/workflows/release.yml`.
- Confirm the latest GitHub Release and choose the next semantic version.
- Separate development validation from release packaging: tests remain risk-based, while distributable MSI packaging is cloud-owned.

## Quality Check

- All four project version declarations match.
- `main` and the release tag point to the intended commit.
- The GitHub Release workflow succeeds.
- The Release contains the MSI, MSI signature, and `latest.json`.
- No local MSI build is required solely to publish a release.
