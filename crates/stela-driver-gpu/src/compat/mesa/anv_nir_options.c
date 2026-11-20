// SPDX-License-Identifier: GPL-2.0-or-later
/* The NIR options a device's shaders are built with, for the AIR compiler,
 * which reaches ANV by no route Vulkan offers. Its internal kernels are
 * compute shaders, as ANV's own are, so the options those are built with are
 * the ones it reads (anv_shader_get_nir_options, which Vulkan cannot call). */
#include "anv_private.h"

const struct nir_shader_compiler_options *stela_anv_nir_options(VkPhysicalDevice physical)
{
	ANV_FROM_HANDLE(anv_physical_device, pdev, physical);
	return &pdev->compiler->nir_options[MESA_SHADER_COMPUTE];
}
