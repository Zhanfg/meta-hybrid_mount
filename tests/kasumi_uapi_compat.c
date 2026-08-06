#include <stddef.h>
#include <stdint.h>

typedef uint32_t __u32;
typedef int32_t __s32;
typedef uint64_t __aligned_u64;

#include "../src/sys/kasumi_uapi.h"

#define ASSERT_SIZE(type, expected) \
    _Static_assert(sizeof(type) == (expected), "unexpected size: " #type)
#define ASSERT_OFFSET(type, field, expected) \
    _Static_assert(offsetof(type, field) == (expected), "unexpected offset: " #type "." #field)
#define ASSERT_IOCTL(name, expected) \
    _Static_assert((unsigned long)(name) == (unsigned long)(expected), "unexpected ioctl: " #name)

_Static_assert(sizeof(void *) == 8, "Kasumi ABI check requires a 64-bit LP64 host");
_Static_assert(sizeof(unsigned long) == 8, "Kasumi ABI check requires 64-bit unsigned long");
_Static_assert(KSM_PROTOCOL_VERSION == 17, "review protocol changes before updating the ABI lock");

ASSERT_SIZE(struct kasumi_syscall_arg, 24);
ASSERT_OFFSET(struct kasumi_syscall_arg, src, 0);
ASSERT_OFFSET(struct kasumi_syscall_arg, target, 8);
ASSERT_OFFSET(struct kasumi_syscall_arg, type, 16);

ASSERT_SIZE(struct kasumi_syscall_list_arg, 16);
ASSERT_OFFSET(struct kasumi_syscall_list_arg, buf, 0);
ASSERT_OFFSET(struct kasumi_syscall_list_arg, size, 8);

ASSERT_SIZE(struct kasumi_uid_list_arg, 16);
ASSERT_OFFSET(struct kasumi_uid_list_arg, count, 0);
ASSERT_OFFSET(struct kasumi_uid_list_arg, reserved, 4);
ASSERT_OFFSET(struct kasumi_uid_list_arg, uids, 8);

ASSERT_SIZE(struct kasumi_spoof_kstat, 368);
ASSERT_OFFSET(struct kasumi_spoof_kstat, target_ino, 0);
ASSERT_OFFSET(struct kasumi_spoof_kstat, target_pathname, 8);
ASSERT_OFFSET(struct kasumi_spoof_kstat, spoofed_ino, 264);
ASSERT_OFFSET(struct kasumi_spoof_kstat, spoofed_dev, 272);
ASSERT_OFFSET(struct kasumi_spoof_kstat, spoofed_nlink, 280);
ASSERT_OFFSET(struct kasumi_spoof_kstat, spoofed_size, 288);
ASSERT_OFFSET(struct kasumi_spoof_kstat, spoofed_blocks, 352);
ASSERT_OFFSET(struct kasumi_spoof_kstat, is_static, 360);
ASSERT_OFFSET(struct kasumi_spoof_kstat, err, 364);

ASSERT_SIZE(struct kasumi_spoof_uname, 396);
ASSERT_OFFSET(struct kasumi_spoof_uname, sysname, 0);
ASSERT_OFFSET(struct kasumi_spoof_uname, nodename, 65);
ASSERT_OFFSET(struct kasumi_spoof_uname, release, 130);
ASSERT_OFFSET(struct kasumi_spoof_uname, version, 195);
ASSERT_OFFSET(struct kasumi_spoof_uname, machine, 260);
ASSERT_OFFSET(struct kasumi_spoof_uname, domainname, 325);
/* Six 65-byte arrays occupy 390 bytes; the trailing int is 4-byte aligned. */
ASSERT_OFFSET(struct kasumi_spoof_uname, err, 392);
ASSERT_SIZE(struct kasumi_spoof_cmdline, 4100);
ASSERT_OFFSET(struct kasumi_spoof_cmdline, err, 4096);

ASSERT_SIZE(struct kasumi_maps_rule, 296);
ASSERT_OFFSET(struct kasumi_maps_rule, target_ino, 0);
ASSERT_OFFSET(struct kasumi_maps_rule, spoofed_pathname, 32);
ASSERT_OFFSET(struct kasumi_maps_rule, err, 288);

ASSERT_SIZE(struct kasumi_mount_hide_arg, 264);
ASSERT_OFFSET(struct kasumi_mount_hide_arg, path_pattern, 4);
ASSERT_OFFSET(struct kasumi_mount_hide_arg, err, 260);

ASSERT_SIZE(struct kasumi_maps_spoof_arg, 304);
ASSERT_OFFSET(struct kasumi_maps_spoof_arg, reserved, 4);
ASSERT_OFFSET(struct kasumi_maps_spoof_arg, err, 300);

ASSERT_SIZE(struct kasumi_statfs_spoof_arg, 280);
ASSERT_OFFSET(struct kasumi_statfs_spoof_arg, path, 4);
ASSERT_OFFSET(struct kasumi_statfs_spoof_arg, spoof_f_type, 264);
ASSERT_OFFSET(struct kasumi_statfs_spoof_arg, err, 272);

ASSERT_SIZE(struct kasumi_policy_config_arg, 36);
ASSERT_SIZE(struct kasumi_policy_state_arg, 56);
ASSERT_SIZE(struct kasumi_policy_uid_list_arg, 40);
ASSERT_OFFSET(struct kasumi_policy_uid_list_arg, uids, 24);
ASSERT_OFFSET(struct kasumi_policy_uid_list_arg, err, 32);

/* Protocol-16 shared requests. These values must remain wire-compatible. */
ASSERT_IOCTL(KSM_IOC_ADD_RULE, 0x40185301UL);
ASSERT_IOCTL(KSM_IOC_DEL_RULE, 0x40185302UL);
ASSERT_IOCTL(KSM_IOC_HIDE_RULE, 0x40185303UL);
ASSERT_IOCTL(KSM_IOC_CLEAR_ALL, 0x00005305UL);
ASSERT_IOCTL(KSM_IOC_GET_VERSION, 0x80045306UL);
ASSERT_IOCTL(KSM_IOC_LIST_RULES, 0xc0105307UL);
ASSERT_IOCTL(KSM_IOC_SET_DEBUG, 0x40045308UL);
ASSERT_IOCTL(KSM_IOC_REORDER_MNT_ID, 0x00005309UL);
ASSERT_IOCTL(KSM_IOC_SET_STEALTH, 0x4004530aUL);
ASSERT_IOCTL(KSM_IOC_HIDE_OVERLAY_XATTRS, 0x4018530bUL);
ASSERT_IOCTL(KSM_IOC_ADD_MERGE_RULE, 0x4018530cUL);
ASSERT_IOCTL(KSM_IOC_SET_MIRROR_PATH, 0x4018530eUL);
ASSERT_IOCTL(KSM_IOC_ADD_SPOOF_KSTAT, 0x4170530fUL);
ASSERT_IOCTL(KSM_IOC_UPDATE_SPOOF_KSTAT, 0x41705310UL);
ASSERT_IOCTL(KSM_IOC_SET_UNAME, 0x418c5311UL);
ASSERT_IOCTL(KSM_IOC_SET_CMDLINE, 0x50045312UL);
ASSERT_IOCTL(KSM_IOC_GET_FEATURES, 0x80045313UL);
ASSERT_IOCTL(KSM_IOC_SET_ENABLED, 0x40045314UL);
ASSERT_IOCTL(KSM_IOC_SET_HIDE_UIDS, 0x40105315UL);
ASSERT_IOCTL(KSM_IOC_GET_HOOKS, 0xc0105316UL);
ASSERT_IOCTL(KSM_IOC_ADD_MAPS_RULE, 0x41285317UL);
ASSERT_IOCTL(KSM_IOC_CLEAR_MAPS_RULES, 0x00005318UL);
ASSERT_IOCTL(KSM_IOC_SET_MOUNT_HIDE, 0x41085319UL);
ASSERT_IOCTL(KSM_IOC_SET_MAPS_SPOOF, 0x4130531aUL);
ASSERT_IOCTL(KSM_IOC_SET_STATFS_SPOOF, 0x4118531bUL);
ASSERT_IOCTL(KSM_IOC_SET_UNAME_GLOBAL, 0x418c531cUL);
ASSERT_IOCTL(KSM_IOC_SELINUX_FIX, 0x4004531dUL);

/* Protocol-17 policy extension. */
ASSERT_IOCTL(KSM_IOC_SET_POLICY, 0xc024531eUL);
ASSERT_IOCTL(KSM_IOC_SET_POLICY_UIDS, 0xc028531fUL);
ASSERT_IOCTL(KSM_IOC_CLEAR_POLICY_UIDS, 0xc0285320UL);
ASSERT_IOCTL(KSM_IOC_GET_POLICY, 0xc0385321UL);
ASSERT_IOCTL(KSM_IOC_GET_POLICY_UIDS, 0xc0285322UL);

int main(void) {
    return 0;
}
