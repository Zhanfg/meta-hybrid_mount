# Kasumi protocol compatibility

Hybrid Mount supports the two Kasumi ABIs required by the maintained fork:

- **Protocol 16**: the last known-good integrated-kernel ABI used with the Hybrid Mount `4.2.0-1838` baseline.
- **Protocol 17**: the current fork-pinned Kasumi ABI, including the policy API introduced at ioctls 30–34.

## Compatibility rules

1. Protocols below 16 are rejected.
2. Protocol 16 remains available for existing integrated-kernel deployments.
3. Protocol 17 is the preferred current ABI.
4. Protocols newer than the compiled userspace ABI are rejected until reviewed.
5. Mount, hide, spoof, uname and SELinux-fix ioctls shared by protocols 16 and 17 retain their existing wire layout.
6. UID policy dispatch is selected at runtime:
   - Protocol 16 uses legacy `KSM_IOC_SET_HIDE_UIDS` (ioctl 21).
   - Protocol 17 uses the policy owner/list API (ioctls 30–32).

## Failure behavior

An unsupported Kasumi protocol must not prevent unrelated modules from mounting. Startup degrades Kasumi rules to Magic Mount for the current boot while leaving the persistent configuration unchanged. This downgrade must convert only paths whose effective mode is Kasumi; existing Overlay, Magic and Ignore path decisions remain unchanged.

An operable integrated or externally managed Kasumi runtime must not be replaced or unloaded by the bundled LKM manager. The control-plane API exposes whether the detected LKM is managed by this module so WebUI actions cannot treat an integrated or externally managed runtime as unloadable.

Kasumi rollback returns the runtime to a disabled and unspoofed baseline. It attempts all cleanup operations even after an earlier failure: mount and maps rules, runtime enablement, debug, stealth, UID policy, mount hiding, maps/statfs spoofing, SELinux correction, cmdline spoofing and scoped/global uname spoofing. Cleanup failures are aggregated rather than hiding later failures.

Post-mount telemetry, temporary cleanup, xattr hiding and umount-list registration are best-effort operations; failures in those steps are logged but do not convert already active mounts into a false startup failure. Bind mounts that fail during readonly conversion are detached before the error is returned.

## Fork packaging policy

The maintained fork keeps the canonical `hybrid_mount` module ID so an existing installation and its configuration can be upgraded in place. Fork artifacts use a distinct display name and a fork-specific monotonic `versionCode` namespace. Development packages omit `updateJson`; they must not silently return to the upstream release channel or replace the fork with an unrelated upstream artifact.

The repository-level Full/Lite/Nano update manifests are also marked disabled until a reviewed fork release and stable artifact URL exist.

## Three-round review scope

The maintained branch is reviewed in three separate passes:

1. **Local implementation review**: protocol classification, protocol-specific UID dispatch, legacy module-name detection, integrated-runtime ownership and LKM load/unload behavior.
2. **Architecture and failure-path review**: Kasumi mirror preparation, effective path-rule conversion during local Magic Mount downgrade, transactional Kasumi activation, complete runtime rollback, bind-mount rollback, feature-gated builds and WebUI ownership state.
3. **Whole-chain and artifact review**: protocol-16-compatible structure/ioctl ABI lock, shell entrypoint syntax, post-mount failure semantics, Full/Lite/Nano package contents, ARM64 ELF identity and seven pinned Kasumi LKM targets.

The permanent ABI test locks the shared protocol 16 wire layout and ioctl values 1–29. Policy ioctls 30–34 are separately locked as the protocol 17 extension.

## Validation requirements

Any later Kasumi UAPI update must verify:

- protocol classification tests;
- protocol-specific UID dispatch;
- legacy/current module-name and management-ownership detection;
- unchanged layouts for shared structs and ioctls;
- effective Kasumi-to-Magic path conversion without changing Overlay or Ignore rules;
- complete runtime cleanup after configuration or execution failure;
- Full/Lite/Nano builds and package-content checks;
- Android 15 / kernel 6.6 LKM packaging;
- upgrade compatibility with an existing 1838-style configuration, including the `enable_hidexattr` alias;
- fork display name, monotonic `versionCode`, and absence of an upstream `updateJson` in development artifacts;
- all-feature, Lite and Nano Clippy checks with warnings denied;
- all-feature, Lite and Nano Rust tests plus WebUI lint/tests.

All automated checks must run against the same final branch head. Artifacts from an earlier head are not release candidates.

## Device release gate

Passing code review and CI produces a **device-test candidate**, not a flash-ready release. Promotion requires evidence from the target Android 15 / kernel 6.6 device for:

- protocol 16 detection without loading the bundled protocol 17 LKM;
- cold boot and one additional reboot;
- OverlayFS, Magic Mount and Kasumi module activation;
- a deliberately failing module proving failure isolation and effective Magic fallback;
- WebUI status and LKM ownership consistency;
- runtime cleanup after a forced Kasumi configuration failure;
- module disable/uninstall recovery and return to the known-good 1838 baseline.

Until those checks are recorded, the fork must not publish an update manifest or stable release package.
