# REHYBIRD / Hybrid Mount 4.2.0-2157 Flash Candidate Review

## Candidate identity

- Validated branch: `review/flash-candidate-2157`
- Validated commit: `556a9c92738e237ec9f353ba6f9db7223eade111`
- Source review branch: `review/flash-safety-20260808`
- Upstream baseline included: `Hybrid-Mount/meta-hybrid_mount` `dev` at `c46334edfa3c5beca7a1bc50e5788c9bc9b738eb`
- Candidate relationship to that upstream baseline at review time: ahead, not behind.
- Package version: `4.2.0-2157`
- Package versionCode: `1000002157`

The candidate branch is intentionally frozen. Post-candidate publishing and repository-cleanup work must be performed on a different branch so the reviewed commit and its artifacts remain reproducible.

## Blocking issue found and fixed during this review

The startup configuration loader parsed and sanitized `config.toml` but did not enforce the same protected-target validation used by configuration writes. A stale, manually edited, or otherwise invalid persisted configuration could therefore reach boot-time mount orchestration without the persistent protected-path gate.

The candidate adds protected-target validation to the boot/daemon configuration loader and a regression test that rejects a custom bind target under a radio-critical firmware path.

The same review also corrected the Rust formatting failures that had caused the previous safety branch's lint job to stop before the substantive safety tests ran.

## Review round 1 — installation, startup, rollback, storage

Result: **PASS**

Reviewed areas included:

- archive extraction and package structure safety;
- installer transaction and rollback behavior;
- private `/data/adb` state ownership and permissions;
- startup configuration loading and protected target validation;
- boot failure fail-safe and next-boot module disable behavior;
- uninstall backing-store safety;
- mount transaction rollback and finalization ordering;
- direct block-device write rejection.

The exact candidate passed the repository's shell safety tests, installer transaction tests, package integrity tests, boot fail-safe tests, uninstall safety tests and unit tests.

## Review round 2 — Kasumi, LKM/KMI, path safety, critical Android payloads

Result: **PASS**

Reviewed areas included:

- current-boot LKM ownership receipt and guarded unload;
- KMI override matching against kernel Android generation and major/minor release;
- fallback to Magic Mount when Kasumi cannot be safely activated;
- Kasumi userspace/kernel UAPI lockstep;
- direct Kasumi target/source path ownership, writable-bit, symlink and node-type checks;
- firmware, modem, radio, Bluetooth, qcril, init, VINTF, SELinux, fstab and related critical path exclusions;
- custom bind target canonicalization and read-only remount;
- module critical-payload inventory scanning and explicit Ignore semantics.

All seven arm64 Kasumi LKM targets built and verified successfully in the exact candidate workflow, including `android15-6.6`.

The standalone `android15-6.6` LKM artifact and the copy embedded in the Full package were compared byte-for-byte and matched.

`android15-6.6_arm64_kasumi_lkm.ko` SHA-256:

`72fa18426fcc9465aa050e202c108641a732ffa7da656bce291563d018fa4f05`

## Review round 3 — lint/test matrix, WebUI, build artifacts, package reproducibility

Result: **PASS**

Exact-candidate lint/test workflow:

- WebUI lint: PASS
- WebUI tests: PASS
- Rust formatting: PASS
- shell syntax: PASS
- archive safety: PASS
- installer transaction: PASS
- package integrity: PASS
- boot failure fail-safe: PASS
- uninstall safety: PASS
- direct block-device write rejection: PASS
- Kasumi ABI 16/17 check: PASS
- Clippy Full: PASS
- Clippy Lite: PASS
- Clippy Nano: PASS
- Rust unit tests: PASS
- Lite feature check: PASS
- Nano feature check: PASS

Exact-candidate build workflow:

- Kasumi UAPI export: PASS
- all seven LKM builds: PASS
- Full build / package validation / artifact upload: PASS
- Lite build / package validation / artifact upload: PASS
- Nano build / package validation / artifact upload: PASS
- package notification job: PASS

Workflow run: `31196660354`

## Validated artifacts

GitHub Actions artifact digest for the Full artifact wrapper:

`a868c05212d27884f2c364e6ad89277b5f0ee3f240bd4a898fa82222ed36e5b0`

The downloaded wrapper matched that digest exactly.

Inner install ZIP hashes:

- Full `Hybrid-Mount-4.2.0-2157.zip`: `c6eae8d7480519ca16f98c0b0cc751f40d8e76e8259140d6c2b1572c7930ad68`
- Lite `Hybrid-Mount-Lite-4.2.0-2157.zip`: `dabb84512ebb118086896a17903bfd07494c37bb1dc9be1a3b71859789e1ed6b`
- Nano `Hybrid-Mount-Nano-4.2.0-2157.zip`: `1deee6e8f2026595e0d3344b9afe024b4ac271bfbeb2ccc6818d8ca223b368cb`

Independent artifact inspection confirmed:

- ZIP integrity;
- no absolute or `..` archive paths;
- no archive symlink/special-node entries;
- expected `module.prop` identity;
- aarch64 Android ELF core binary;
- exact seven-LKM Full package set;
- package-integrity helper passes for Full, Lite and Nano;
- all three flavors use the same monotonic fork versionCode `1000002157`.

## Flash eligibility decision

**Code/build/package review status: eligible for a controlled real-device flash test.**

This statement means the reviewed source and exact generated package have passed the three required review rounds and the available automated/static artifact gates. It is not a claim that software-only review can guarantee hardware behavior on every kernel or ROM.

The remaining acceptance gate is device-level verification after flashing the exact frozen Full artifact (or another selected flavor): boot completion, root-manager module state, mount behavior, radio/Bluetooth sanity, and rollback/recovery behavior if startup fails.

Do not substitute a later rebuild or a different branch head for this candidate without repeating the artifact-level validation.

## Post-candidate maintenance separated from the flash candidate

Publishing/repository cleanup is being kept on `maintenance/post-candidate-2157` and does not alter the frozen flash candidate. This includes correcting renamed-fork links and hardening the formal Release workflow so release metadata derives its versionCode from the actual built packages and all release packages pass the same package-integrity gates before publication.
