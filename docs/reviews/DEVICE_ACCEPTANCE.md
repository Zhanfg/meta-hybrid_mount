# Controlled real-device acceptance gate

The frozen 4.2.0-2157 candidate has passed source, CI and artifact review. Real hardware remains a separate gate because software-only review cannot prove Bluetooth, radio/baseband, vendor services or boot behavior on a specific device.

## Exact candidate

- Branch: `review/flash-candidate-2157`
- Commit: `556a9c92738e237ec9f353ba6f9db7223eade111`
- Full ZIP SHA-256: `c6eae8d7480519ca16f98c0b0cc751f40d8e76e8259140d6c2b1572c7930ad68`
- Expected Full package version: `4.2.0-2157`
- Expected versionCode: `1000002157`

Do not substitute a later rebuild, maintenance-branch ZIP, or historical review artifact.

## Before flashing

Run `scripts/device-preflash-snapshot.sh` as root and retain the generated `REHYBIRD_PreFlash_*.tar.gz` archive. It is read-only and intentionally avoids full telephony/subscriber dumps.

Confirm the install ZIP SHA-256 matches the value above before installing it.

## After reboot

Run `scripts/device-postflash-verify.sh` as root. The script checks only observable state and produces a redacted `REHYBIRD_PostFlash_*.tar.gz` report.

A software-side acceptance requires all of the following:

- Android reaches `sys.boot_completed=1`.
- `hybrid_mount` remains installed and has no `disable` marker.
- Expected Hybrid Mount/Overlay/Kasumi mount state is observable for the modules being managed.
- Bluetooth service is present and does not show an immediate crash/fatal loop in the bounded report.
- Telephony/radio services are present and the generic baseband property remains available.
- No new Hybrid Mount/Kasumi kernel panic/oops or repeated fatal startup error is visible in the bounded relevant logs.

## Manual hardware checks still required

The scripts cannot prove RF/audio hardware operation. Manually verify:

- Bluetooth can be enabled and disabled normally.
- A known Bluetooth audio device can connect and play audio.
- SIM/network registration returns normally.
- Mobile data can establish a connection.
- A normal voice call can be placed/received if the current SIM/plan supports it.
- The modules that motivated Hybrid Mount actually expose their expected mounted files/features.

Do not merge/tag/release solely because the automated post-flash script says `OBSERVABLE_GATES_PASS`; the manual Bluetooth/radio/mount checks are part of acceptance.

## Failure boundary

If boot, Bluetooth, radio/baseband, or mount behavior regresses, stop the acceptance run and preserve the pre/post reports. Do not test additional experimental builds on top of the failed state. Recovery should use the root manager/device recovery procedure appropriate to the installed environment, after which the reports can be compared to isolate the regression.
