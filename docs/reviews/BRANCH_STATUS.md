# Branch status for the 4.2.0-2157 validation cycle

This file prevents historical review branches from being mistaken for current flash candidates.

## Active references

| Purpose | Branch | Required identity |
| --- | --- | --- |
| Frozen real-device flash candidate | `review/flash-candidate-2157` | `556a9c92738e237ec9f353ba6f9db7223eade111` |
| Post-candidate cleanup and release hardening | `maintenance/post-candidate-2157` | moving maintenance branch; never substitute it for the frozen flash artifact without rebuilding/revalidating |
| Upstream baseline included by the candidate | `Hybrid-Mount/meta-hybrid_mount:dev` | `c46334edfa3c5beca7a1bc50e5788c9bc9b738eb` at the 2026-08-08 re-check |

## Historical review snapshots — do not flash

The following branches are retained only for audit history and regression comparison:

- `review/baseline-c4acd01-20260807`
- `review/final-candidate-1962`
- `review/flash-safety-20260807`
- `review/flash-safety-20260808`
- `review/round2-cleanup-backup`
- `review/triple-code-review`
- `review/triple-code-review-p0-fix`
- `review/triple-code-review-reconciled`
- `review/upstream-sync-only-20260807`

`review/final-candidate-1962` is specifically superseded by `review/flash-candidate-2157` and must not be presented as the current final candidate.

## Gate policy

1. Do not move or amend `review/flash-candidate-2157`.
2. Do not tag or publish a release from a review/maintenance branch.
3. A formal release tag must point to a commit already contained in `main` and must pass the release preflight.
4. Real-device acceptance of the exact 2157 artifact is required before merging/publishing the candidate line.
5. Any source change that affects the install package after the frozen candidate requires a new artifact identity and artifact-level validation.
