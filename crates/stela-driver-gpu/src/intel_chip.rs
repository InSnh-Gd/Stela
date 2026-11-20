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
    /// What `struct drm_xe_query_config` and `struct
    /// drm_xe_query_mem_regions` hold before their arrays: their count, then
    /// four bytes of alignment.
    const COUNT: usize = 0;
    /// The offset of `drm_xe_query_config`'s `info` array and of
    /// `drm_xe_query_mem_regions`'s `mem_regions` array.
    const CONFIG_INFO: usize = 8;
    /// `DRM_XE_QUERY_CONFIG_REV_AND_DEVICE_ID`: the parameter whose word
    /// holds the device ID in its lower 16 bits.
    const CONFIG_REV_AND_DEVICE_ID: usize = 0;
    const MEM_REGIONS_FIRST: usize = 8;
    /// `struct drm_xe_mem_region`, and the two fields read from it.
    const MEM_REGION_BYTES: usize = 88;
    const MEM_REGION_CLASS: usize = 0;
    const MEM_REGION_TOTAL_SIZE: usize = 8;
    /// `DRM_XE_MEM_REGION_CLASS_VRAM` (`enum drm_xe_memory_class`).
    const MEM_REGION_CLASS_VRAM: u16 = 1;
    /// The most bytes a reply may ask for. The driver answers a size of zero
    /// with what it needs, and refuses any other size (`xe_query.c`), so the
    /// reply is allocated from the driver's own answer: this bounds a reply
    /// that answer cannot be trusted for. Memory regions are the largest
    /// this reads, one per memory class and instance.
    const MOST_BYTES: usize = 8 << 10;

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

    /// Asks `query` for its reply and answers what the driver wrote, or `None`
    /// when a call fails or asks for more room than [`MOST_BYTES`].
    fn ask(control: Control, query: u32) -> Option<Vec<u8>> {
        // The size the caller has room for is not a hint the driver trims to:
        // it answers zero with the size it needs and refuses every other size
        // that is not exactly that one, so the size is asked for first.
        let mut record = DeviceQuery {
            extensions: 0,
            query,
            size: 0,
            data: 0,
            pad: [0; 2],
        };
        if control(DEVICE_QUERY, (&raw mut record).cast()) != 0 {
            return None;
        }
        let bytes = record.size as usize;
        if bytes == 0 || bytes > MOST_BYTES {
            return None;
        }
        let mut reply = vec![0u8; bytes];
        record.data = reply.as_mut_ptr() as u64;
        if control(DEVICE_QUERY, (&raw mut record).cast()) != 0 {
            return None;
        }
        Some(reply)
    }

    /// `DRM_XE_QUERY_CONFIG_REV_AND_DEVICE_ID`: the PCI device ID xe bound,
    /// which is the chip identifier this stack reports.
    pub fn device_id(control: Control) -> Option<u32> {
        let reply = ask(control, QUERY_CONFIG)?;
        if half(&reply, COUNT) as usize <= CONFIG_REV_AND_DEVICE_ID {
            return None;
        }
        let id = word(&reply, CONFIG_INFO + CONFIG_REV_AND_DEVICE_ID * 8) & 0xffff;
        (id != 0).then_some(id as u32)
    }

    /// The bytes of device memory the driver makes available: what its VRAM
    /// regions total, which on a VF is what the PF provisioned for it.
    pub fn memory_bytes(control: Control) -> Option<u64> {
        let reply = ask(control, QUERY_MEM_REGIONS)?;
        let count = half(&reply, COUNT) as usize;
        if count == 0 || count > (reply.len() - MEM_REGIONS_FIRST) / MEM_REGION_BYTES {
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
