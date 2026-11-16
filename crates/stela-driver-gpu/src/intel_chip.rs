//! Intel display-chip identity and memory as the xe kernel driver reports
//! them: the two facts [`crate::stack::Kernel`] takes from a vendor's driver
//! and Vulkan does not carry.
//!
//! Both answers are one `DRM_IOCTL_XE_DEVICE_QUERY` away, the way nouveau's
//! VRAM size and chipset are one `DRM_IOCTL_NOUVEAU_GETPARAM` away.

/// What xe's device file answers about the bound chip
/// (`include/uapi/drm/xe_drm.h`).
pub mod query {
    use crate::stack::Control;
    use alloc::{vec, vec::Vec};

    /// `struct drm_xe_device_query`: the query asked, the size of the reply
    /// the caller has room for and where that reply goes.
    #[repr(C)]
    struct DeviceQuery {
        extensions: u64,
        query: u32,
        size: u32,
        data: u64,
        pad: [u32; 2],
    }

    /// `DRM_IOCTL_XE_DEVICE_QUERY`: `DRM_IOWR(DRM_COMMAND_BASE, struct
    /// drm_xe_device_query)`, whose record is 32 bytes.
    const DEVICE_QUERY: u32 = 0xc020_6440;
    /// `DRM_XE_DEVICE_QUERY_CONFIG`.
    const QUERY_CONFIG: u32 = 2;
    /// `DRM_XE_DEVICE_QUERY_MEM_REGIONS`.
    const QUERY_MEM_REGIONS: u32 = 1;
    /// `struct drm_xe_query_config`: two 32-bit fields, then one 64-bit word
    /// per parameter.
    const CONFIG_INFO: usize = 8;
    /// `DRM_XE_QUERY_CONFIG_REV_AND_DEVICE_ID`: the parameter whose word
    /// holds the device ID in its lower 16 bits.
    const CONFIG_REV_AND_DEVICE_ID: usize = 0;
    /// How many parameters to make room for, past the five the header names.
    const CONFIG_PARAMS: usize = 8;
    /// `struct drm_xe_query_mem_regions`: its count, then its regions.
    const MEM_REGIONS_COUNT: usize = 0;
    const MEM_REGIONS_FIRST: usize = 8;
    /// How many regions to make room for: a device has one per memory class
    /// and instance, and a VF's are its own.
    const MEM_REGIONS: usize = 16;
    /// `struct drm_xe_mem_region`, and the two fields read from it.
    const MEM_REGION_BYTES: usize = 88;
    const MEM_REGION_CLASS: usize = 0;
    const MEM_REGION_TOTAL_SIZE: usize = 8;
    /// `DRM_XE_MEM_REGION_CLASS_VRAM` (`enum drm_xe_memory_class`).
    const MEM_REGION_CLASS_VRAM: u16 = 1;

    /// The little-endian `u64` at `offset`.
    fn word(reply: &[u8], offset: usize) -> u64 {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&reply[offset..offset + 8]);
        u64::from_le_bytes(bytes)
    }

    /// The little-endian `u32` at `offset`.
    fn half(reply: &[u8], offset: usize) -> u32 {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&reply[offset..offset + 4]);
        u32::from_le_bytes(bytes)
    }

    /// The little-endian `u16` at `offset`.
    fn quarter(reply: &[u8], offset: usize) -> u16 {
        let mut bytes = [0u8; 2];
        bytes.copy_from_slice(&reply[offset..offset + 2]);
        u16::from_le_bytes(bytes)
    }

    /// Asks `query` for a reply of `bytes` and answers what the driver wrote,
    /// or `None` when the call fails or asks for more room than that.
    fn ask(control: Control, query: u32, bytes: usize) -> Option<Vec<u8>> {
        let mut reply = vec![0u8; bytes];
        let mut record = DeviceQuery {
            extensions: 0,
            query,
            size: bytes as u32,
            data: reply.as_mut_ptr() as u64,
            pad: [0; 2],
        };
        if control(DEVICE_QUERY, (&raw mut record).cast()) != 0 {
            return None;
        }
        (record.size as usize <= bytes).then_some(reply)
    }

    /// `DRM_XE_QUERY_CONFIG_REV_AND_DEVICE_ID`: the PCI device ID xe bound,
    /// which is the chip identifier this stack reports.
    pub fn device_id(control: Control) -> Option<u32> {
        let reply = ask(control, QUERY_CONFIG, CONFIG_INFO + CONFIG_PARAMS * 8)?;
        let id = word(&reply, CONFIG_INFO + CONFIG_REV_AND_DEVICE_ID * 8) & 0xffff;
        (id != 0).then_some(id as u32)
    }

    /// The bytes of device memory the driver makes available: what its VRAM
    /// regions total, which on a VF is what the PF provisioned for it.
    pub fn memory_bytes(control: Control) -> Option<u64> {
        let bytes = MEM_REGIONS_FIRST + MEM_REGIONS * MEM_REGION_BYTES;
        let reply = ask(control, QUERY_MEM_REGIONS, bytes)?;
        let count = half(&reply, MEM_REGIONS_COUNT) as usize;
        if count == 0 || count > MEM_REGIONS {
            return None;
        }
        let mut total = 0u64;
        for index in 0..count {
            let region = MEM_REGIONS_FIRST + index * MEM_REGION_BYTES;
            if quarter(&reply, region + MEM_REGION_CLASS) == MEM_REGION_CLASS_VRAM {
                total += word(&reply, region + MEM_REGION_TOTAL_SIZE);
            }
        }
        (total != 0).then_some(total)
    }
}
