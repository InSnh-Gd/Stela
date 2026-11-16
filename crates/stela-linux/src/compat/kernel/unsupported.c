// SPDX-License-Identifier: GPL-2.0-or-later
/* Kernel services Stela does not provide yet. */
#include <linux/folio_batch.h>
#include <linux/fs.h>
#include <linux/io.h>
#include <linux/mm.h>
#include <linux/platform_device.h>
#include <linux/rcupdate.h>
#include <linux/seq_file.h>
#include <linux/swap.h>
#include <stela/linux.h>
#include <unsupported.h>

/* Pages behind shmem files and shmem-backed GEM objects, which swap and
 * nouveau's TTM objects never need: TTM allocates pages from its own pool and
 * Stela never reclaims them. */
STELA_UNSUPPORTED(struct folio *, shmem_read_folio_gfp, struct address_space *, pgoff_t, gfp_t)
STELA_UNSUPPORTED(struct page *, shmem_read_mapping_page_gfp, struct address_space *, pgoff_t,
		  gfp_t)
STELA_UNSUPPORTED(void, shmem_truncate_range, struct inode *, loff_t, uoff_t)
struct swap_iocb;
STELA_UNSUPPORTED(int, shmem_writeout, struct folio *, struct swap_iocb **, struct list_head *)
STELA_UNSUPPORTED(bool, folio_clear_dirty_for_io, struct folio *)
/* Transparent huge mappings of shmem-backed objects. */
STELA_UNSUPPORTED(unsigned long, mm_get_unmapped_area, struct file *, unsigned long,
		  unsigned long, unsigned long, unsigned long)
/* debugfs's seq_file operations, which no file opens: Stela has no debugfs. */
STELA_UNSUPPORTED(int, single_open, struct file *, int (*)(struct seq_file *, void *), void *)
STELA_UNSUPPORTED(int, single_release, struct inode *, struct file *)
STELA_UNSUPPORTED(ssize_t, seq_read, struct file *, char __user *, size_t, loff_t *)
STELA_UNSUPPORTED(loff_t, seq_lseek, struct file *, loff_t, int)
/* Platform devices, of which only HDMI audio codecs reach DRM. */
STELA_UNSUPPORTED(void, platform_device_unregister, struct platform_device *)
STELA_UNSUPPORTED(struct platform_device *, platform_device_alloc, const char *, int)
STELA_UNSUPPORTED(int, platform_device_add, struct platform_device *)
STELA_UNSUPPORTED(void, platform_device_put, struct platform_device *)

/* The GSC firmware interface, which xe only builds on the discrete parts that
 * carry one: it returns before the auxiliary device and the simulated
 * interrupt domain exist on every other platform, including this one. */
struct auxiliary_device;
STELA_UNSUPPORTED(int, auxiliary_device_init, struct auxiliary_device *)
STELA_UNSUPPORTED(int, __auxiliary_device_add, struct auxiliary_device *)
STELA_UNSUPPORTED(int, __irq_alloc_descs, int, unsigned int, unsigned int, int, struct module *,
		  const struct irq_affinity_desc *)
STELA_UNSUPPORTED(void, irq_free_descs, unsigned int, unsigned int)
STELA_UNSUPPORTED(void, irq_set_chip_and_handler_name, unsigned int, const struct irq_chip *,
		  irq_flow_handler_t, const char *)
STELA_UNSUPPORTED(int, irq_set_chip_data, unsigned int, void *)
STELA_UNSUPPORTED(void, handle_simple_irq, struct irq_desc *)
STELA_UNSUPPORTED(int, generic_handle_irq_safe, unsigned int)

/* The I2C-attached AMC of the display engine, which only a display build
 * creates: Stela's I2C core has no bus notifier loop, no software nodes and no
 * adapters for xe_i2c to find. */
struct i2c_client;
/* The bus every I2C device is on. Stela's I2C core attaches no bus to its
 * devices, so nothing walks this one's lists; naming it is all it takes for a
 * caller that registers a notifier on it. */
const struct bus_type i2c_bus_type = {.name = "i2c"};
STELA_UNSUPPORTED(struct i2c_adapter *, i2c_verify_adapter, struct device *)
STELA_UNSUPPORTED(int, i2c_transfer_buffer_flags, const struct i2c_client *, char *, int, u16)
STELA_UNSUPPORTED(int, bus_register_notifier, const struct bus_type *, struct notifier_block *)
STELA_UNSUPPORTED(int, bus_unregister_notifier, const struct bus_type *, struct notifier_block *)
struct fwnode_handle;
struct property_entry;
STELA_UNSUPPORTED(struct fwnode_handle *, fwnode_create_software_node,
		  const struct property_entry *, const struct fwnode_handle *)
STELA_UNSUPPORTED(void, fwnode_remove_software_node, struct fwnode_handle *)

/* Entry points Stela serves nothing through: no userspace DRM client opens an
 * out-of-band observation stream or a user-mode fence, no SVM address space is
 * made current, and a VM's memory is never snapshotted for a devcoredump. */
struct mm_struct;
STELA_UNSUPPORTED(void, mmput, struct mm_struct *)
STELA_UNSUPPORTED(void, kthread_use_mm, struct mm_struct *)
STELA_UNSUPPORTED(void, kthread_unuse_mm, struct mm_struct *)
struct vm_fault;
STELA_UNSUPPORTED(vm_fault_t, vmf_insert_pfn, struct vm_area_struct *, unsigned long,
		  unsigned long)
STELA_UNSUPPORTED(int, anon_inode_getfd, const char *, const struct file_operations *, void *,
		  int)

/* An interrupt that is in service. Stela's owner delivers a line in one
 * context and never reports a handler still in flight, so there is nothing to
 * wait for and no honest answer to give a driver that resets a device. */
STELA_UNSUPPORTED(void, synchronize_irq, unsigned int)

/* Another task's argument vector, which lives in an address space Stela does
 * not walk. Only lib/string_helpers.c's quotable command line asks for it, and
 * nothing in the image calls that. */
struct task_struct;
STELA_UNSUPPORTED(int, get_cmdline, struct task_struct *, char *, int)

/* Bus window assignment after a resize, which Stela refuses before a driver
 * can reach this. */
struct pci_bus;
STELA_UNSUPPORTED(void, pci_assign_unassigned_bus_resources, struct pci_bus *)
