// SPDX-License-Identifier: GPL-2.0-or-later
/* Mesa's futex: the word its locks, queues and allocators wait on. Under
 * `HAVE_LINUX_FUTEX_H` Mesa reaches the kernel's own futex with `syscall`,
 * which this image answers with ENOSYS; its host, though, waits on a word of
 * memory and wakes whoever waits on one directly, so the two halves are
 * `stela_libc_wait` and `stela_libc_wake`. What a caller may rely on is
 * Linux's: a wait ends when the word no longer holds the value, or when the
 * deadline passes, which is absolute and in `stela_libc_now_ns`'s
 * nanoseconds. */
#include <errno.h>
#include <stdint.h>
#include <time.h>
#include <stela/host.h>

int futex_wait(uint32_t *addr, int32_t value, const struct timespec *timeout)
{
	uint64_t deadline = UINT64_MAX;

	if (timeout) {
		deadline = (uint64_t)timeout->tv_sec * 1000000000ULL +
			   (uint64_t)timeout->tv_nsec;
	}

	stela_libc_wait(addr, (unsigned int)value, deadline);

	/* Linux reports a wait that ran out of time and found the word unchanged;
	 * a caller waiting on a fence reads that as the fence not moving. */
	if (timeout && *(volatile uint32_t *)addr == (uint32_t)value &&
	    stela_libc_now_ns() >= deadline) {
		errno = ETIMEDOUT;
		return -1;
	}
	return 0;
}

int futex_wake(uint32_t *addr, int32_t count)
{
	/* The host releases every waiter; Linux's count only spares some of them,
	 * and no caller here reads the count it gets back. */
	(void)count;
	stela_libc_wake(addr);
	return 0;
}
