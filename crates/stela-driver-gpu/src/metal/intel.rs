//! Intel's integrated GPUs under Stela's Metal device: the xe port's kernel
//! driver under Linux and ANV, Mesa's Intel Vulkan driver, over it.
//!
//! Hardware access, firmware, memory management, command submission and
//! shader code generation stay with xe, ANV and its Intel compiler; this
//! module only admits a device and supplies the facts Vulkan does not report,
//! as the Intel stack's part of [`open`](super::open).
use super::{
    abi::DeviceLocation,
    capabilities::Platform,
    open::{self, Admission, Attachment},
    vulkan::{DriverId, PhysicalDeviceType, physical::Identity},
};

pub const VENDOR_ID: u32 = 0x8086;

/// Why a Vulkan device is not admitted as an Intel Metal device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    NotIntel {
        vendor_id: u32,
    },
    /// Only ANV is integrated with Stela's xe port.
    NotAnv(DriverId),
    /// The parts the xe port drives are integrated, and their memory is the
    /// host's.
    NotIntegrated(PhysicalDeviceType),
    /// The kernel driver and the Vulkan driver name different functions,
    /// so the device the guest would draw through is not the one Linux bound.
    Mismatch {
        kernel_device_id: u32,
        vulkan_device_id: u32,
    },
}

/// Admits the Vulkan device `identity`, which the kernel driver reports as
/// the PCI device `device_id` (`DRM_XE_QUERY_CONFIG_REV_AND_DEVICE_ID`).
///
/// No table of Intel parts stands here, as one of nouveau's chipsets does for
/// NVIDIA: xe binds only the parts its own ID table knows and ANV reports a
/// physical device only for those it drives, so the two identifiers agreeing
/// is the whole of what is left to check.
pub fn admit(
    identity: &Identity,
    device_id: u32,
    attachment: Attachment,
) -> Result<Platform, Refusal> {
    if identity.vendor_id != VENDOR_ID {
        return Err(Refusal::NotIntel {
            vendor_id: identity.vendor_id,
        });
    }
    if identity.driver_id != DriverId::IntelOpenSourceMesa {
        return Err(Refusal::NotAnv(identity.driver_id));
    }
    if identity.device_type != PhysicalDeviceType::IntegratedGpu {
        return Err(Refusal::NotIntegrated(identity.device_type));
    }
    if identity.device_id != device_id {
        return Err(Refusal::Mismatch {
            kernel_device_id: device_id,
            vulkan_device_id: identity.device_id,
        });
    }
    Ok(platform(identity, attachment))
}

/// The row-pitch and bind-offset alignment ISL asks of a linear surface:
/// `PRI_STRIDE`, which the display engine reads, must be 64-byte aligned
/// (`isl.c`, "When using linear memory, this must be at least 64 byte
/// aligned"), and a linear surface's base address carries the same 64 bytes
/// where it is displayable, the strongest of the alignments ISL derives from
/// a linear surface's uses.
const LINEAR_ALIGNMENT: u64 = 64;
/// ANV's sparse tile is a 64 KiB page of the surface's address space
/// (`anv_sparse.c`, which binds a sparse surface in 64 KiB steps), which is
/// Vulkan's standard block shape as well.
const SPARSE_PAGE_BYTES: u64 = 64 << 10;

/// The facts Vulkan does not report for the admitted device.
fn platform(identity: &Identity, attachment: Attachment) -> Platform {
    Platform {
        // The Vulkan driver's own name for the chip, which is where Intel's
        // parts are named: xe reports the identifier alone, and no table of
        // Intel names stands beside nouveau's.
        architecture: identity.device_name.clone(),
        // The part is on the host's own board, not in a slot or an
        // enclosure, and its memory is the host's.
        location: DeviceLocation::BuiltIn,
        location_number: attachment.location_number,
        max_transfer_rate: 0,
        // An Intel integrated part drives the panel through its display
        // engine; what the guest sees of the display is the platform's.
        headless: false,
        removable: false,
        linear_row_alignment: LINEAR_ALIGNMENT,
        linear_offset_alignment: LINEAR_ALIGNMENT,
        sparse_page_bytes: SPARSE_PAGE_BYTES,
    }
}

/// The Intel stack's admission: the kernel driver's part and the Vulkan
/// device ANV's.
pub const ADMISSION: Admission = Admission { admit: admission };

fn admission(
    identity: &Identity,
    device_id: u32,
    attachment: Attachment,
) -> Result<Platform, open::Refusal> {
    admit(identity, device_id, attachment).map_err(open::Refusal::Intel)
}

/// ANV as linked into the image, with its driver-internal calls
/// (`compat/mesa/anv_nir_options.c`).
#[cfg(all(feature = "metal-linked", not(test)))]
pub const DRIVER: super::native::Driver = {
    use super::native::{Driver, vk};
    use core::ffi::c_char;
    use core::ffi::c_void;
    unsafe extern "system" {
        fn stela_anv_icd_get_instance_proc_addr(
            instance: vk::Instance,
            name: *const c_char,
        ) -> vk::PFN_vkVoidFunction;
    }
    unsafe extern "C" {
        fn stela_anv_nir_options(physical: vk::PhysicalDevice) -> *const c_void;
    }
    Driver {
        get_instance_proc_addr: stela_anv_icd_get_instance_proc_addr,
        nir_options: stela_anv_nir_options,
        // ANV has no submit thread of its own to enable: its queues submit
        // where Linux runs only through the port's own path.
        submit_thread: None,
        // A refusal ANV keeps the device through is not a shape it has.
        refusals: None,
    }
};
