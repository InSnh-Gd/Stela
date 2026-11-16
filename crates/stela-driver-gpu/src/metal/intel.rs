//! Intel's integrated GPUs under Stela's Metal device.
//!
//! The xe port drives the GPU and the guest's own drivers reach it through
//! Stela's Linux, but the Metal device macOS asks for is a Vulkan driver
//! linked into the image: NVK's route for NVIDIA's GPUs, and the Intel
//! Vulkan driver's (ANV, built for the xe port) for these before it can be
//! the same. That driver is not linked yet, so an Intel GPU is refused here
//! by name, and a boot that had claimed one stops with the reason instead of
//! starting a guest whose WindowServer has no device to draw through.
use super::{
    capabilities::Platform,
    open::{self, Admission, Attachment},
    vulkan::physical::Identity,
};

/// The Intel stack's admission: the GPU is refused until the image links the
/// Vulkan driver over the xe port.
pub const ADMISSION: Admission = Admission { admit };

/// Why an Intel GPU is not admitted as a Metal device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The image links no Intel Vulkan driver over the xe port.
    NoVulkanDriver,
}

fn admit(
    _identity: &Identity,
    _chip: u32,
    _attachment: Attachment,
) -> Result<Platform, open::Refusal> {
    Err(open::Refusal::Intel(Refusal::NoVulkanDriver))
}
