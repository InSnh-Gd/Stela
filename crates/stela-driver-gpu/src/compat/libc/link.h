/* SPDX-License-Identifier: GPL-2.0-or-later */
/* The dynamic linker's list of loaded objects. The image is linked whole, so
 * nothing is loaded and the walk this header names is empty: Mesa's own
 * build-id lookup, the one caller in the image, is `compat/mesa/build_id.c`. */
#ifndef STELA_LIBC_LINK_H
#define STELA_LIBC_LINK_H
#include <stddef.h>
#include <sys/cdefs.h>
__BEGIN_DECLS
/* glibc's field names, for the x86-64 objects Stela runs as. */
struct dl_phdr_info {
	unsigned long dlpi_addr;
	const char *dlpi_name;
	const void *dlpi_phdr;
	unsigned short dlpi_phnum;
	/* glibc's remaining fields: the segment count and the TLS and namespace
	 * description a caller that asks a loaded object about them reads. */
	unsigned long dlpi_adds;
	unsigned long dlpi_subs;
	size_t dlpi_tls_modid;
	void *dlpi_tls_data;
};
int dl_iterate_phdr(int (*callback)(struct dl_phdr_info *info, size_t size, void *data),
		    void *data);
__END_DECLS
#endif
