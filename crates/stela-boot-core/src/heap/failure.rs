//! First failed ordinary allocation: no allocation, waiting or caller formatter.
use super::{CLASSES, CPUS, Heap, Usage};
use core::{
    alloc::Layout,
    fmt,
    sync::atomic::{AtomicUsize, Ordering},
};

/// Called once, after the first failed allocation's report reached the sink:
/// the boot names the blocks the heap held then. The heap has just refused
/// an allocation, so the hook keeps its own work small yet may allocate: the
/// one-shot guard already claimed this failure.
static FAILURE_HOOK: AtomicUsize = AtomicUsize::new(0);

/// Sets the hook [`FAILURE_HOOK`] calls.
pub fn set_failure_hook(hook: fn()) {
    FAILURE_HOOK.store(hook as usize, Ordering::Release);
}

fn failure_reported() {
    let hook = FAILURE_HOOK.load(Ordering::Acquire);
    if hook != 0 {
        // SAFETY: only `set_failure_hook` stores a nonzero value, a `fn()`.
        let hook: fn() = unsafe { core::mem::transmute(hook) };
        hook();
    }
}

struct Report {
    layout: Layout,
    usage: Option<Usage>,
    /// Live size-class bytes by class, and each tagged core's own bytes, as
    /// the relaxed per-core counters hold them.
    classes: [isize; CLASSES],
    cores: [isize; CPUS],
    tagged: usize,
}

fn capture(heap: &Heap, layout: Layout) -> Option<Report> {
    if heap
        .failure_reported
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return None;
    }
    let usage = heap.try_lock().and_then(|guard| {
        // SAFETY: the successful one-shot guard owns heap metadata and masks
        // local IRQ reentry. No state borrow or guard reaches UART formatting.
        let state = unsafe { &*heap.state.get() };
        let usage = state.ready.then(|| heap.usage_locked(state));
        drop(guard);
        usage
    });
    let snapshot = usage.map_or([0; 4], |usage| {
        [usage.live, usage.carved, usage.largest, usage.total]
    });
    heap.failed[0].store(layout.size(), Ordering::Relaxed);
    for (slot, value) in heap.failed[2..].iter().zip(snapshot) {
        slot.store(value, Ordering::Relaxed);
    }
    // The alignment, never zero, publishes the rest.
    heap.failed[1].store(layout.align(), Ordering::Release);
    let mut cores = [0; CPUS];
    let mut tagged = 0;
    for (slot, own) in cores.iter_mut().zip(heap.owned()) {
        *slot = own;
        tagged += 1;
    }
    Some(Report {
        layout,
        usage,
        classes: heap.class_live(),
        cores,
        tagged,
    })
}
impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "HEAP ALLOCATION FAILURE bytes={} align={}",
            self.layout.size(),
            self.layout.align()
        )?;
        match self.usage {
            Some(usage) => {
                writeln!(
                    f,
                    " snapshot=locked-metadata live_estimate={} cpu_counters=relaxed total={} carved={} peak={} largest={}",
                    usage.live, usage.total, usage.carved, usage.peak, usage.largest
                )?;
                // The live free-list allocations by power of two from 4 KiB,
                // where a size holds a megabyte or more, as HEAP lines give them.
                f.write_str("HEAP AT FAILURE large")?;
                for (bucket, &(count, bytes)) in usage.large.iter().enumerate() {
                    if bytes >= 1 << 20 {
                        write!(f, " {}K:{count}/{}M", 4 << bucket, bytes >> 20)?;
                    }
                }
            }
            None => f.write_str(" snapshot=unavailable\nHEAP AT FAILURE")?,
        }
        f.write_str(" classes")?;
        for (class, &bytes) in self.classes.iter().enumerate() {
            write!(f, " {}:{}M", 16 << class, bytes >> 20)?;
        }
        f.write_str(" cores")?;
        for &own in &self.cores[..self.tagged] {
            write!(f, " {}M", own >> 20)?;
        }
        writeln!(f)
    }
}

/// The first allocation that found no memory, and the heap it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Failure {
    pub bytes: usize,
    pub align: usize,
    /// None where another holder had the heap's metadata.
    pub heap: Option<FailedHeap>,
}

/// The heap's bytes when an allocation failed: a largest free block under
/// the request with much more free means fragmentation, both small means
/// the heap was full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailedHeap {
    pub live: usize,
    pub carved: usize,
    pub largest: usize,
    pub total: usize,
}

pub(super) fn first(heap: &Heap) -> Option<Failure> {
    let align = heap.failed[1].load(Ordering::Acquire);
    if align == 0 {
        return None;
    }
    let [live, carved, largest, total] =
        core::array::from_fn(|index| heap.failed[2 + index].load(Ordering::Relaxed));
    Some(Failure {
        bytes: heap.failed[0].load(Ordering::Relaxed),
        align,
        heap: (total != 0).then_some(FailedHeap {
            live,
            carved,
            largest,
            total,
        }),
    })
}

/// ` (the heap had no room for 17609526 bytes aligned to 1: heap=2619M/3072M
/// carved=2776M largest=16M)`, with BEAT's names, for the stop line the
/// screen and boot log keep.
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            " (the heap had no room for {} bytes aligned to {}",
            self.bytes, self.align
        )?;
        if let Some(heap) = self.heap {
            write!(
                f,
                ": heap={}M/{}M carved={}M largest={}M",
                heap.live >> 20,
                heap.total >> 20,
                heap.carved >> 20,
                heap.largest >> 20
            )?;
        }
        f.write_str(")")
    }
}

pub(super) fn report(heap: &Heap, layout: Layout) {
    if let Some(report) = capture(heap, layout) {
        // This formatter emits only fixed strings and integer fields; the
        // sink is bounded, lock-free output that runs no queue or log.
        (heap.failure_sink)(format_args!("\n{report}\n"));
        // Outside the lock `capture` took and released, and after the
        // report, so the hook's own lines follow the numbers they name.
        failure_reported();
    }
}

#[cfg(test)]
#[path = "failure/tests.rs"]
mod tests;
