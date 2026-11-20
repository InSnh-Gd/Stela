//! One vendor's GPU driver stack as data: the Linux kernel driver that binds
//! the PCI function, the Mesa Vulkan driver over it, and the few facts and
//! driver-internal calls Vulkan does not carry. Everything above and below
//! (the Linux environment, the DRM ioctl path, the Metal device, the AIR
//! compiler, the gate) is shared and reads a vendor only through this.
//!
//! [`GpuStack::select`] chooses by PCI vendor. Whether a device of that
//! vendor is driven is then the kernel driver's own ID table's answer, when
//! Linux matches the function against the registered driver.
use crate::{Adapter, Vendor};
use core::ffi::c_void;

/// A device-control call on the bound GPU's open DRM file: the request code
/// and its argument, answering Linux's result (zero or a negative errno).
pub type Control<'a> = &'a mut dyn FnMut(u32, *mut c_void) -> i32;

/// The Linux kernel driver of a stack.
pub struct Kernel {
    /// The name its `pci_driver` registers under.
    pub driver: &'static str,
    /// The address bits of the widest DMA mask the driver sets on the devices
    /// Stela drives; a mapping past it is refused, not truncated.
    pub dma_bits: u8,
    /// The driver's own identifier of the bound chip, by its query ioctl.
    pub chip: fn(Control) -> Option<u32>,
    /// The bytes of device memory the driver makes available.
    pub memory_bytes: fn(Control) -> Option<u64>,
}

/// One vendor's stack.
pub struct GpuStack {
    pub vendor: Vendor,
    pub kernel: Kernel,
    /// Admission of the stack's Vulkan device as a Metal device.
    #[cfg(feature = "metal")]
    pub metal: crate::metal::open::Admission,
    /// The Vulkan driver linked into the image. This crate's own test
    /// binary links no driver archive, so it holds none.
    #[cfg(all(feature = "metal-linked", not(test)))]
    pub vulkan: Option<crate::metal::native::Driver>,
}

impl Kernel {
    /// The widest DMA address the driver's mask allows.
    pub const fn dma_limit(&self) -> u64 {
        u64::MAX >> (64 - self.dma_bits)
    }
}

/// Every stack this build holds. NVIDIA's stands for every image, as the
/// engine's own refusal lines do; Intel's joins it with the xe port, since
/// no other build can bind one.
#[cfg(feature = "xe-port")]
pub static STACKS: [&GpuStack; 2] = [&NVIDIA, &INTEL];
#[cfg(not(feature = "xe-port"))]
pub static STACKS: [&GpuStack; 1] = [&NVIDIA];

/// nouveau with GSP firmware, NVK and NAK.
pub static NVIDIA: GpuStack = GpuStack {
    vendor: Vendor::Nvidia,
    kernel: Kernel {
        driver: "nouveau",
        // The Turing-and-later MMUs' `dma_bits` (`nvkm/subdev/mmu/tu102.c`
        // 47, `gh100.c` 52), which `nvkm_device_pci_new` sets as the mask.
        dma_bits: 47,
        chip: crate::nouveau_chip::query::chipset,
        memory_bytes: crate::nouveau_chip::query::memory_bytes,
    },
    #[cfg(feature = "metal")]
    metal: crate::metal::nvidia::ADMISSION,
    #[cfg(all(feature = "metal-linked", not(test)))]
    vulkan: Some(crate::metal::nvidia::DRIVER),
};

/// Intel's integrated GPUs: the xe kernel driver over its GuC and HuC
/// firmware, for the Xe3P parts Stela drives (`build/xe.rs`), with ANV for
/// the Metal device macOS asks for.
#[cfg(feature = "xe-port")]
pub static INTEL: GpuStack = GpuStack {
    vendor: Vendor::Intel,
    kernel: Kernel {
        driver: "xe",
        // Panther Lake's `ptl_desc.dma_mask_size`, which xe sets on the
        // function (`drivers/gpu/drm/xe/xe_pci.c`): a mapping past it is
        // refused, not truncated.
        dma_bits: 46,
        chip: crate::intel_chip::query::device_id,
        memory_bytes: crate::intel_chip::query::memory_bytes,
    },
    #[cfg(feature = "metal")]
    metal: crate::metal::intel::ADMISSION,
    #[cfg(all(feature = "metal-linked", not(test)))]
    vulkan: Some(crate::metal::intel::DRIVER),
};

/// Brings Linux up and registers every kernel driver built into the image,
/// stopping at the first stage that fails. Linux then matches each presented
/// function against the registered drivers' own ID tables, so one image
/// serves every vendor's devices and registration is not per stack.
///
/// # Safety
/// On the canonical Linux owner's original managed task after Initial and
/// SRCU startup have completed, once per environment. Initcalls may sleep.
#[cfg(any(feature = "nouveau-port", feature = "xe-port"))]
pub unsafe fn start_kernel() -> Result<(), stela_linux::owner::Errno> {
    // SAFETY: the caller's contract, once per image.
    #[cfg(feature = "nouveau-port")]
    unsafe {
        crate::nouveau::start()?
    };
    #[cfg(feature = "xe-port")]
    // SAFETY: as above.
    unsafe {
        crate::xe::start()?
    };
    Ok(())
}

impl GpuStack {
    /// The stack for `adapter`'s vendor, or why Stela has none.
    pub fn select(adapter: &Adapter) -> Result<&'static GpuStack, &'static str> {
        STACKS
            .iter()
            .copied()
            .find(|stack| stack.vendor == adapter.vendor)
            .ok_or_else(|| adapter.unavailable_reason())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::pci::Address;

    fn adapter(vendor: u16) -> Adapter {
        Adapter {
            address: Address {
                segment: 0,
                bus: 1,
                device: 0,
                function: 0,
            },
            vendor: Vendor::from_pci(vendor),
            device: 0x1234,
            class: [3, 0, 0],
        }
    }

    #[test]
    fn a_stack_is_selected_by_pci_vendor_alone() {
        let nvidia = GpuStack::select(&adapter(0x10de)).unwrap();
        assert!(core::ptr::eq(nvidia, &NVIDIA));
        assert_eq!(nvidia.kernel.driver, "nouveau");
        // Intel's GPU is answered by the xe port's stack, which only an image
        // carrying that port holds; the rest have no stack in any build.
        let intel = GpuStack::select(&adapter(0x8086));
        if cfg!(feature = "xe-port") {
            assert_eq!(intel.unwrap().kernel.driver, "xe");
        } else {
            assert_eq!(
                intel.err(),
                Some(adapter(0x8086).unavailable_reason())
            );
        }
        for vendor in [0x1002, 0x1234] {
            let adapter = adapter(vendor);
            assert_eq!(
                GpuStack::select(&adapter).err(),
                Some(adapter.unavailable_reason())
            );
        }
    }

    #[test]
    fn each_vendor_has_one_stack_and_a_usable_dma_mask() {
        for (index, stack) in STACKS.iter().enumerate() {
            assert!(
                STACKS[..index]
                    .iter()
                    .all(|other| other.vendor != stack.vendor)
            );
            assert!((32..=64).contains(&stack.kernel.dma_bits));
            assert_eq!(
                stack.kernel.dma_limit().count_ones(),
                stack.kernel.dma_bits.into()
            );
            assert!(!stack.kernel.driver.is_empty());
        }
    }

    #[test]
    fn nouveau_queries_ask_its_getparam_and_keep_refusals() {
        // `struct drm_nouveau_getparam`: the parameter, then its value.
        let mut asked = alloc::vec::Vec::new();
        let mut answer = |request: u32, argument: *mut c_void| {
            // SAFETY: the query passes its own two-word record.
            let words = unsafe { &mut *argument.cast::<[u64; 2]>() };
            asked.push((request, words[0]));
            words[1] = if words[0] == 11 { 0x1b3 } else { 16 << 30 };
            0
        };
        assert_eq!((NVIDIA.kernel.chip)(&mut answer), Some(0x1b3));
        assert_eq!((NVIDIA.kernel.memory_bytes)(&mut answer), Some(16 << 30));
        assert_eq!(asked, [(0xc010_6440, 11), (0xc010_6440, 8)]);
        let mut refuse = |_: u32, _: *mut c_void| -22;
        assert_eq!((NVIDIA.kernel.chip)(&mut refuse), None);
        assert_eq!((NVIDIA.kernel.memory_bytes)(&mut refuse), None);
        let mut empty = |_: u32, _: *mut c_void| 0;
        assert_eq!((NVIDIA.kernel.memory_bytes)(&mut empty), None);
    }

    /// xe answers its own device query: `DRM_XE_DEVICE_QUERY_CONFIG` for the
    /// device ID in the first `info` word's lower 16 bits
    /// (`DRM_XE_QUERY_CONFIG_REV_AND_DEVICE_ID`), and
    /// `DRM_XE_DEVICE_QUERY_MEM_REGIONS` for what its VRAM regions total.
    #[cfg(feature = "xe-port")]
    #[test]
    fn xe_queries_ask_its_device_query_and_keep_refusals() {
        let mut asked = alloc::vec::Vec::new();
        let mut answer = |request: u32, argument: *mut c_void| {
            // SAFETY: both queries pass their own `struct drm_xe_device_query`.
            let record = unsafe { &mut *argument.cast::<[u64; 4]>() };
            // The query is the record's first 32-bit field, its size the
            // second.
            let query = record[1] as u32;
            let bytes = (record[1] >> 32) as usize;
            asked.push((request, query));
            // What the driver answers: the bytes its reply needs, and how to
            // fill that many when they are asked for.
            let (needed, fill): (usize, fn(&mut [u8])) = match query {
                // `DRM_XE_DEVICE_QUERY_CONFIG`: five parameters, whose first
                // holds the device ID and revision.
                2 => (
                    8 + 5 * 8,
                    |reply| {
                        reply[..4].copy_from_slice(&5u32.to_le_bytes());
                        reply[8..16].copy_from_slice(&0x0000_0000_0000_b080u64.to_le_bytes());
                    },
                ),
                // `DRM_XE_DEVICE_QUERY_MEM_REGIONS`: one VRAM region of
                // 3072 MiB, whose `total_size` lies past its two halves.
                1 => (
                    8 + 88,
                    |reply| {
                        reply[..4].copy_from_slice(&1u32.to_le_bytes());
                        reply[8..10].copy_from_slice(&1u16.to_le_bytes());
                        reply[16..24].copy_from_slice(&(3072u64 << 20).to_le_bytes());
                    },
                ),
                other => panic!("unexpected query {other}"),
            };
            // The driver refuses a size of its own reply that is not exactly
            // what it needs, and answers a size of zero with that one.
            if bytes == 0 {
                record[1] |= (needed as u64) << 32;
                return 0;
            }
            assert_eq!(bytes, needed, "a reply is asked for at its own size");
            // SAFETY: the caller allocated `bytes` for this reply.
            let reply = unsafe { core::slice::from_raw_parts_mut(record[2] as *mut u8, bytes) };
            fill(reply);
            0
        };
        assert_eq!((INTEL.kernel.chip)(&mut answer), Some(0xb080));
        assert_eq!((INTEL.kernel.memory_bytes)(&mut answer), Some(3072 << 20));
        assert_eq!(
            asked,
            [
                (0xc020_6440, 2),
                (0xc020_6440, 2),
                (0xc020_6440, 1),
                (0xc020_6440, 1)
            ]
        );
        let mut refuse = |_: u32, _: *mut c_void| -22;
        assert_eq!((INTEL.kernel.chip)(&mut refuse), None);
        assert_eq!((INTEL.kernel.memory_bytes)(&mut refuse), None);
    }
}
