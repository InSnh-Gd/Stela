// SPDX-License-Identifier: GPL-2.0-or-later
/* An mm's MMU-notifier subscriptions, following Linux
 * cc9de5d4b5ab6459f5774946aa3195ad51236fa8 mm/mmu_notifier.c. Stela offers no
 * page-table invalidation to register for, so no subscription can exist and
 * releasing an mm has nothing to tear down. The interface itself is Linux's:
 * <linux/mmu_notifier.h> supplies its interval helpers to the drivers that
 * reference them. */
#include <linux/mmu_notifier.h>
#include <linux/mm_types.h>

void __mmu_notifier_subscriptions_destroy(struct mm_struct *mm)
{
	(void)mm;
}
