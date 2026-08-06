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

An unsupported Kasumi protocol must not prevent unrelated modules from mounting. Startup degrades Kasumi mount rules to Magic Mount for the current boot while leaving the persistent configuration unchanged.

## Fork packaging policy

The maintained fork keeps the canonical `hybrid_mount` module ID so an existing installation and its configuration can be upgraded in place. Fork artifacts use a distinct display name and a fork-specific monotonic `versionCode` namespace. Development packages omit `updateJson`; they must not silently return to the upstream release channel or replace the fork with an unrelated upstream artifact.

The repository-level Full/Lite/Nano update manifests are also marked disabled until a reviewed fork release and stable artifact URL exist.

## Validation requirements

Any later Kasumi UAPI update must verify:

- protocol classification tests;
- protocol-specific UID dispatch;
- unchanged layouts for shared structs and ioctls;
- Full/Lite/Nano builds;
- Android 15 / kernel 6.6 LKM packaging;
- upgrade compatibility with an existing 1838-style configuration, including the `enable_hidexattr` alias;
- fork display name, monotonic `versionCode`, and absence of an upstream `updateJson` in development artifacts.

The maintained candidate is accepted only when these checks run against the same final branch head; artifacts from an earlier head are not treated as release candidates.
