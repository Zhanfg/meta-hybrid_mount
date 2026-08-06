from pathlib import Path

path = Path("src/sys/kasumi.rs")
source = path.read_text(encoding="utf-8")

uid_anchor = '''pub type KasumiUidListArg = uapi::kasumi_uid_list_arg;

impl uapi::kasumi_uid_list_arg {
    pub fn from_slice(uids: &[u32]) -> Self {
        Self {
            count: uids.len() as u32,
            reserved: 0,
            uids: if uids.is_empty() {
                0
            } else {
                uids.as_ptr() as usize as u64
            },
        }
    }
}
'''
uid_replacement = uid_anchor + '''
pub const KSM_POLICY_API_VERSION: u32 = uapi::KSM_POLICY_API_VERSION as u32;
pub const KSM_POLICY_OWNER_AUTO: u32 = uapi::KSM_POLICY_OWNER_AUTO as u32;
pub const KSM_POLICY_OWNER_MANUAL: u32 = uapi::KSM_POLICY_OWNER_MANUAL as u32;
pub const KSM_POLICY_FLAG_USE_ALLOW_UIDS: u32 =
    uapi::KSM_POLICY_FLAG_USE_ALLOW_UIDS as u32;
pub const KSM_POLICY_UID_LIST_ALLOW: u32 = uapi::KSM_POLICY_UID_LIST_ALLOW as u32;
pub const KSM_POLICY_UID_LIST_ALL: u32 = uapi::KSM_POLICY_UID_LIST_ALL as u32;

pub type KasumiPolicyConfigArg = uapi::kasumi_policy_config_arg;
pub type KasumiPolicyUidListArg = uapi::kasumi_policy_uid_list_arg;

impl_zeroed_default!(uapi::kasumi_policy_config_arg);
impl_zeroed_default!(uapi::kasumi_policy_uid_list_arg);

impl uapi::kasumi_policy_config_arg {
    fn new(owner: u32, flags: u32) -> Self {
        Self {
            version: KSM_POLICY_API_VERSION,
            size: std::mem::size_of::<Self>() as u32,
            owner,
            flags,
            ..Self::default()
        }
    }
}

impl uapi::kasumi_policy_uid_list_arg {
    fn from_slice(list: u32, uids: &[u32]) -> Self {
        Self {
            version: KSM_POLICY_API_VERSION,
            size: std::mem::size_of::<Self>() as u32,
            list,
            count: uids.len() as u32,
            uids: if uids.is_empty() {
                0
            } else {
                uids.as_ptr() as usize as u64
            },
            ..Self::default()
        }
    }

    fn for_list(list: u32) -> Self {
        Self {
            version: KSM_POLICY_API_VERSION,
            size: std::mem::size_of::<Self>() as u32,
            list,
            ..Self::default()
        }
    }
}
'''

ioctl_anchor = '''pub const KSM_IOC_SELINUX_FIX: KasumiIoctlRequest =
    ioctl::opcode::write::<c_int>(KSM_IOC_MAGIC, 29);
'''
ioctl_replacement = ioctl_anchor + '''pub const KSM_IOC_SET_POLICY: KasumiIoctlRequest =
    ioctl::opcode::read_write::<KasumiPolicyConfigArg>(KSM_IOC_MAGIC, 30);
pub const KSM_IOC_SET_POLICY_UIDS: KasumiIoctlRequest =
    ioctl::opcode::read_write::<KasumiPolicyUidListArg>(KSM_IOC_MAGIC, 31);
pub const KSM_IOC_CLEAR_POLICY_UIDS: KasumiIoctlRequest =
    ioctl::opcode::read_write::<KasumiPolicyUidListArg>(KSM_IOC_MAGIC, 32);
'''

legacy_anchor = '''pub fn set_hide_uids(uids: &[u32]) -> Result<()> {
    let mut arg = KasumiUidListArg::from_slice(uids);
    ioctl_with_arg("set_hide_uids", KSM_IOC_SET_HIDE_UIDS, &mut arg)
}
'''
legacy_replacement = '''fn set_policy(owner: u32, flags: u32) -> Result<()> {
    let mut arg = KasumiPolicyConfigArg::new(owner, flags);
    ioctl_with_arg("set_policy", KSM_IOC_SET_POLICY, &mut arg)?;
    ensure_kernel_err("Kasumi set_policy", arg.err)
}

fn set_policy_uids(list: u32, uids: &[u32]) -> Result<()> {
    let mut arg = KasumiPolicyUidListArg::from_slice(list, uids);
    ioctl_with_arg("set_policy_uids", KSM_IOC_SET_POLICY_UIDS, &mut arg)?;
    ensure_kernel_err("Kasumi set_policy_uids", arg.err)
}

fn clear_policy_uids(list: u32) -> Result<()> {
    let mut arg = KasumiPolicyUidListArg::for_list(list);
    ioctl_with_arg("clear_policy_uids", KSM_IOC_CLEAR_POLICY_UIDS, &mut arg)?;
    ensure_kernel_err("Kasumi clear_policy_uids", arg.err)
}

/// Configure the legacy `hide_uids` setting through protocol 17's manual
/// policy API. Protocol 17 still defines ioctl 21 for source compatibility,
/// but no longer dispatches it. An explicit Hybrid Mount UID list means
/// exactly those UIDs receive Kasumi's managed view, so map it to MANUAL +
/// ALLOW instead of silently receiving `EINVAL` from the obsolete request.
pub fn set_hide_uids(uids: &[u32]) -> Result<()> {
    clear_policy_uids(KSM_POLICY_UID_LIST_ALL)?;

    if uids.is_empty() {
        return set_policy(KSM_POLICY_OWNER_AUTO, 0);
    }

    set_policy_uids(KSM_POLICY_UID_LIST_ALLOW, uids)?;
    set_policy(KSM_POLICY_OWNER_MANUAL, KSM_POLICY_FLAG_USE_ALLOW_UIDS)
}
'''

for name, old, new in (
    ("policy types", uid_anchor, uid_replacement),
    ("policy ioctls", ioctl_anchor, ioctl_replacement),
    ("legacy hide UID migration", legacy_anchor, legacy_replacement),
):
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{name}: expected exactly one anchor, found {count}")
    source = source.replace(old, new, 1)

path.write_text(source, encoding="utf-8")
