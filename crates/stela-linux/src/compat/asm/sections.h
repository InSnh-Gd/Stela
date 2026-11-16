/* SPDX-License-Identifier: GPL-2.0 */
/* An architecture's section boundaries come from its linker script, which
 * Stela's image has none of, so the generic boundaries stay declared and
 * unusable here. The one question Linux asks about them that Stela can answer
 * is whether a pointer is into read-only data: the image owner classifies
 * that itself, so that predicate goes to it. */
#ifndef _ASM_STELA_SECTIONS_H
#define _ASM_STELA_SECTIONS_H

#include <asm-generic/sections.h>
#include <stela/linux.h>

#define is_kernel_rodata(address) stela_linux_is_rodata((const void *)(address))

#endif
