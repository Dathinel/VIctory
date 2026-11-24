//! Host copies of title buffers (decision 0007, step 3 and "Copies of title
//! memory"). Title buffers are title memory, which the host GPU cannot
//! read, so a buffer a draw uses gets a persistent host buffer of its size
//! on first use (CPU-writable VRAM via Resizable BAR when supported with
//! headroom, or write-back system memory otherwise), and the ranges each
//! submission reads (vertex, index and constant data, buffer views) are
//! brought up to date from title memory before it runs: the 64 KiB pages
//! the title wrote since they were last copied ([`crate::watch`]: write
//! watching, or read-only guards on physical-page views), all of it where
//! neither works. A title GPU address inside the buffer is the same offset
//! in its copy. Ranges shaders write go back to title memory after the
//! submission ([`Shadow::gpu_writes`]).
//!
//! shortcut: only the ranges draws are known to read are brought up to
//! date (vertex and index buffer views, constant buffers in tables, root
//! descriptors, buffer views in tables); buffers read through directly
//! indexed heaps keep what their first use copied.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::Graphics::Direct3D12::{
    D3D12_RESOURCE_STATE_GENERIC_READ, D3D12_RESOURCE_STATES, ID3D12Resource,
};

use crate::host::{HostList, host};
use crate::watch::PAGE;

/// Total VRAM bytes currently allocated to ReBAR (`D3D12_HEAP_TYPE_GPU_UPLOAD`)
/// shadow buffers across live [`Shadow`] instances.
static REBAR_SHADOW_BYTES: AtomicU64 = AtomicU64::new(0);

/// Atomically reserves `size` bytes from `budget`, returning `false` if doing
/// so would exceed `budget`.
fn try_reserve_rebar(used: &AtomicU64, size: u64, budget: u64) -> bool {
    if budget == 0 || size == 0 || size > budget {
        return false;
    }
    used.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| {
        let next = cur.checked_add(size)?;
        (next <= budget).then_some(next)
    })
    .is_ok()
}

/// Extra state for a [`Shadow`] backed by CPU-writable VRAM
/// (`D3D12_HEAP_TYPE_GPU_UPLOAD`, Resizable BAR).
///
/// `Shadow::data` is a write-combining pointer into VRAM and MUST NEVER be read
/// on the CPU. Read-only buffers keep `mirror = None` (zero duplicate RAM). On
/// the first UAV write, `mirror` is lazily populated via a one-time GPU DMA and
/// kept synced on subsequent `Shadow::copy` / `copy_back_rebar` calls.
struct RebarShadow {
    bytes: u64,
    mirror: Mutex<Option<Vec<u8>>>,
}

/// A title buffer's host copy.
pub(crate) struct Shadow {
    pub(crate) buffer: ID3D12Resource,
    data: *mut u8,
    gpu: u64,
    /// Where the title buffer starts and how long it is.
    start: u64,
    len: u64,
    /// Per page of title memory from `start`'s on: the write-watch asking
    /// after which it was last copied (0: never).
    copied: Mutex<Vec<u64>>,
    /// Whether the memory is write-watched: unknown, yes, no.
    watched: AtomicU8,
    /// Ranges shaders of a submission not yet copied back may be writing:
    /// title memory there is older than the copy.
    pending: Mutex<Vec<(usize, usize)>>,
    /// The copy's bytes before the submission ran, from an offset on,
    /// covering the ranges its shaders write: what the GPU wrote differs.
    before: Mutex<Before>,
    /// Present when `buffer` lives in ReBAR `D3D12_HEAP_TYPE_GPU_UPLOAD` VRAM.
    rebar: Option<RebarShadow>,
}

impl Drop for Shadow {
    fn drop(&mut self) {
        if let Some(rebar) = &self.rebar {
            REBAR_SHADOW_BYTES.fetch_sub(rebar.bytes, Ordering::Relaxed);
        }
    }
}

#[derive(Default)]
struct Before {
    snapshot: Option<(usize, Vec<u8>)>,
    unchanged: UnchangedPages,
}

/// Pages already compared in full against this submission's snapshot.
#[derive(Default)]
struct UnchangedPages {
    start: usize,
    end: usize,
    pages: Vec<bool>,
}

impl UnchangedPages {
    fn begin(&mut self, start: usize, end: usize) {
        if self.pages.is_empty() {
            self.start = start;
            self.end = end;
            self.pages.resize(end.div_ceil(PAGE) - start / PAGE, false);
        }
    }

    fn differs(&mut self, address: usize, current: &[u8], before: &[u8]) -> bool {
        let page = address / PAGE;
        let unchanged = &mut self.pages[page - self.start / PAGE];
        if *unchanged {
            return false;
        }
        if current != before {
            return true;
        }
        // A later range can touch bytes a partial comparison missed. At
        // the edges, only bytes covered by the snapshot can be compared.
        *unchanged = address == (page * PAGE).max(self.start)
            && address + current.len() == ((page + 1) * PAGE).min(self.end);
        false
    }

    fn clear(&mut self) {
        self.pages.clear();
    }
}

/// Snapshot memory kept for reuse (already touched, so no page faults).
static POOL: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

/// An empty buffer with room for `n` bytes, from the pool if it has one.
fn pooled(n: usize) -> Vec<u8> {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    let fit = pool
        .iter()
        .enumerate()
        .filter(|(_, v)| v.capacity() >= n)
        .min_by_key(|(_, v)| v.capacity())
        .map(|(i, _)| i);
    let mut v = match fit {
        Some(i) => pool.swap_remove(i),
        None => Vec::with_capacity(n),
    };
    v.clear();
    v
}

/// Returns a snapshot buffer to the pool (at most 8 kept, the largest).
fn give_back(v: Vec<u8>) {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    pool.push(v);
    if pool.len() > 8 {
        pool.sort_by_key(|v| std::cmp::Reverse(v.capacity()));
        pool.truncate(8);
    }
}

const NOT_WATCHED: u8 = 2;

// SAFETY: D3D12 resources are free-threaded; `data` is a persistent mapping
// written only on the submitting thread.
unsafe impl Send for Shadow {}
// SAFETY: as above.
unsafe impl Sync for Shadow {}

impl Shadow {
    /// The host GPU address of a title address inside the buffer.
    pub(crate) fn gpu(&self, address: u64) -> u64 {
        self.gpu + (address - self.start)
    }

    /// Bytes from a title address to the buffer's end.
    pub(crate) fn left(&self, address: u64) -> u64 {
        (self.start + self.len).saturating_sub(address)
    }

    /// Offset of a title address from the buffer's start.
    pub(crate) fn offset(&self, address: u64) -> u64 {
        address - self.start
    }

    /// Brings `len` bytes at `address` up to date when `list` is submitted,
    /// and keeps the copy alive until the list is reset.
    pub(crate) fn refresh(self: &Arc<Self>, list: &HostList, address: u64, len: u64) {
        self.refresh_as(list, address, len, D3D12_RESOURCE_STATE_GENERIC_READ);
    }

    /// Refresh a range and prepare its host buffer for the specified GPU use.
    pub(crate) fn refresh_as(
        self: &Arc<Self>,
        list: &HostList,
        address: u64,
        len: u64,
        access: D3D12_RESOURCE_STATES,
    ) {
        list.use_shadow(&self.buffer, access);
        let len = len.min(self.start + self.len - address);
        list.defer_refresh(self, address as usize, len as usize);
    }

    fn new(
        buffer: ID3D12Resource,
        data: *mut u8,
        gpu: u64,
        start: u64,
        len: u64,
        rebar_bytes: Option<u64>,
    ) -> Shadow {
        let pages = (start + len).div_ceil(PAGE as u64) - start / PAGE as u64;
        Shadow {
            buffer,
            data,
            gpu,
            start,
            len,
            copied: Mutex::new(vec![0; pages as usize]),
            watched: AtomicU8::new(0),
            pending: Mutex::default(),
            before: Mutex::default(),
            rebar: rebar_bytes.map(|bytes| RebarShadow {
                bytes,
                mirror: Mutex::new(None),
            }),
        }
    }

    /// Whether this copy lives in ReBAR (`D3D12_HEAP_TYPE_GPU_UPLOAD`) VRAM.
    pub(crate) fn is_rebar(&self) -> bool {
        self.rebar.is_some()
    }

    /// `[address, address + len)` cut to the buffer, as offsets in the copy.
    pub(crate) fn inside(&self, address: usize, len: usize) -> (usize, usize) {
        let (start, end) = (self.start as usize, (self.start + self.len) as usize);
        let from = address.clamp(start, end);
        (from - start, (address + len).clamp(from, end) - start)
    }

    /// Shaders of `list` may write `[address, address + len)` of the copy
    /// (unordered access): after the list's submission has run, the range
    /// is copied back into title memory, where the title reads it; until
    /// then no refresh copies title memory over it. Call after the
    /// [`Shadow::refresh`] that gives the shaders the title's data.
    ///
    /// shortcut: what shaders wrote is found by keeping the range's bytes
    /// before the submission (a copy per declared range and submission);
    /// a GPU-side dirty mask would save that if big ranges make it slow.
    pub(crate) fn gpu_writes(self: &Arc<Self>, list: &HostList, address: u64, len: u64) {
        let len = len.min(self.left(address));
        list.defer_gpu_write(self, address as usize, len as usize);
    }

    /// At submission, after the range's refresh: a submitted list's shaders
    /// write this range. Its bytes are kept to tell later what the GPU wrote:
    /// one snapshot per copy and submission, grown to cover every range the
    /// submission declares (the world declares overlapping views of its
    /// 174 MB heap dozens of times), in reused memory (fresh snapshots cost
    /// ~2.3 million page faults per world frame).
    pub(crate) fn writing(
        &self,
        address: usize,
        len: usize,
        rebar_init: impl FnOnce(&ID3D12Resource, u64) -> Option<Vec<u8>>,
    ) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((address, address + len));
        if let Some(rebar) = &self.rebar {
            let mut mirror = rebar.mirror.lock().unwrap_or_else(|e| e.into_inner());
            if mirror.is_none() {
                *mirror = rebar_init(&self.buffer, self.len);
            }
            return;
        }
        let (from, to) = self.inside(address, len);
        // SAFETY: inside the mapping (write-back system memory; ReBAR buffers
        // returned early above so write-combining memory is never read).
        let host =
            |a: usize, b: usize| unsafe { std::slice::from_raw_parts(self.data.add(a), b - a) };
        let mut before = self.before.lock().unwrap_or_else(|e| e.into_inner());
        let before = &mut before.snapshot;
        let (start, end) = match &*before {
            Some((at, bytes)) => ((*at).min(from), (at + bytes.len()).max(to)),
            None => (from, to),
        };
        if let Some((at, bytes)) = &*before
            && (*at, at + bytes.len()) == (start, end)
        {
            return;
        }
        // Parts newly covered from the copy, the rest as it was taken.
        let mut grown = pooled(end - start);
        match before.take() {
            Some((at, bytes)) => {
                grown.extend_from_slice(host(start, at));
                grown.extend_from_slice(&bytes);
                grown.extend_from_slice(host(at + bytes.len(), end));
                give_back(bytes);
            }
            None => grown.extend_from_slice(host(start, end)),
        }
        *before = Some((start, grown));
    }

    /// Clears pending UAV-write ranges for a ReBAR shadow if readback failed.
    pub(crate) fn clear_pending_rebar(&self) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// ReBAR counterpart to [`Shadow::copy_back`]: merges post-submission GPU
    /// bytes (`gpu_slices`, already DMAed into cached `READBACK` RAM) for
    /// sorted disjoint buffer-offset `spans` against the cached RAM mirror,
    /// without ever reading write-combining VRAM on the CPU.
    pub(crate) fn copy_back_rebar(&self, spans: &[(usize, usize)], gpu_slices: &[&[u8]]) -> usize {
        let Some(rebar) = &self.rebar else {
            return 0;
        };
        let mut mirror_guard = rebar.mirror.lock().unwrap_or_else(|e| e.into_inner());
        let mut written = 0;
        let start = self.start as usize;
        if let Some(mirror) = mirror_guard.as_mut() {
            let mut title = None;
            for (&(from, to), &gpu_span) in spans.iter().zip(gpu_slices) {
                if from >= to || to > mirror.len() || gpu_span.len() != to - from {
                    continue;
                }
                let mut synced = Vec::new();
                let mut at = from;
                while at < to {
                    let n = (PAGE - (start + at) % PAGE).min(to - at);
                    let gpu = &gpu_span[at - from..at - from + n];
                    let old = &mut mirror[at..at + n];
                    if gpu != old {
                        let title_buf = &mut title.get_or_insert_with(|| vec![0u8; PAGE])[..n];
                        if crate::host::read_into((start + at) as *const u8, title_buf) {
                            let mut changed = false;
                            let mut host_differs = false;
                            for ((t, &g), o) in title_buf.iter_mut().zip(gpu).zip(old.iter_mut()) {
                                if g != *o && *t != g {
                                    *t = g;
                                    changed = true;
                                }
                                if *t != g {
                                    host_differs = true;
                                }
                                *o = *t;
                            }
                            if changed {
                                crate::watch::writing(start + at, n);
                                crate::host::write_mapped(
                                    (start + at) as *mut u8,
                                    title_buf.as_ptr(),
                                    n,
                                );
                                written += n;
                            }
                            if host_differs {
                                // Only push back to WC VRAM when a concurrent CPU
                                // write altered bytes in the same page.
                                // SAFETY: `at + n <= self.len`, write-only WC store.
                                unsafe {
                                    std::ptr::copy_nonoverlapping(
                                        title_buf.as_ptr(),
                                        self.data.add(at),
                                        n,
                                    );
                                }
                            }
                            synced.push((start + at) / PAGE);
                        } else {
                            // Unmapped in title memory: VRAM holds `gpu`, keep mirror in sync.
                            old.copy_from_slice(gpu);
                        }
                    }
                    at += n;
                }
                let (first, last) = ((start + from).div_ceil(PAGE), (start + to) / PAGE);
                if first < last
                    && !synced.is_empty()
                    && self.watched.load(Ordering::Relaxed) != NOT_WATCHED
                    && let Some((now, _)) = crate::watch::writes(first * PAGE, last * PAGE, true)
                {
                    let base = start / PAGE;
                    let mut copied = self.copied.lock().unwrap_or_else(|e| e.into_inner());
                    for page in synced.into_iter().filter(|p| (first..last).contains(p)) {
                        copied[page - base] = now;
                    }
                }
            }
        }
        drop(mirror_guard);
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        written
    }

    /// After the submission ran: copies the bytes its shaders wrote back
    /// into title memory (unmapped pages left alone), and only those: the
    /// title's threads write the same buffers while the GPU runs (its
    /// command memory shares the big GPU heap with what shaders write), and
    /// putting back the copy's older bytes there broke the title's own
    /// records (a fence list read as -1, title+0x4e9b54a). Returns the bytes
    /// written.
    pub(crate) fn copy_back(&self, address: usize, len: usize) -> usize {
        let mut before = self.before.lock().unwrap_or_else(|e| e.into_inner());
        let Before {
            snapshot,
            unchanged,
        } = &mut *before;
        let mut written = 0;
        let (from, to) = self.inside(address, len);
        let start = self.start as usize;
        if let Some((taken, before)) = &*snapshot
            && *taken <= from
            && to <= taken + before.len()
            && from < to
        {
            unchanged.begin(start + taken, start + taken + before.len());
            let before = &before[from - taken..to - taken];
            // By 64 KiB piece: the copy compared with its bytes before the
            // submission, title memory read only where the GPU wrote.
            // An unchanged page is never modified by copy-back, so later
            // overlapping ranges can skip it for the rest of the submission.
            let mut title = None;
            let mut synced = Vec::new();
            let mut at = from;
            while at < to {
                let n = (PAGE - (start + at) % PAGE).min(to - at);
                // SAFETY: inside the mapping; only the submitting thread
                // writes it.
                let host = unsafe { std::slice::from_raw_parts_mut(self.data.add(at), n) };
                let old = &before[at - from..at - from + n];
                if unchanged.differs(start + at, host, old)
                    && let title = &mut title.get_or_insert_with(|| vec![0u8; PAGE])[..n]
                    && crate::host::read_into((start + at) as *const u8, title)
                {
                    // Title memory with the GPU's bytes put in; the copy
                    // takes the title's bytes in turn.
                    let mut changed = false;
                    for ((t, h), o) in title.iter_mut().zip(host.iter_mut()).zip(old) {
                        if *h != *o && *t != *h {
                            *t = *h;
                            changed = true;
                        }
                        *h = *t;
                    }
                    if changed {
                        crate::watch::writing(start + at, n);
                        crate::host::write_mapped((start + at) as *mut u8, title.as_ptr(), n);
                        written += n;
                    }
                    synced.push((start + at) / PAGE);
                }
                at += n;
            }
            // Pages made equal to title memory are current in the copy, so
            // our own write is not taken for the title's; others keep what
            // they had (a title write there shows at the next refresh).
            let (first, last) = ((start + from).div_ceil(PAGE), (start + to) / PAGE);
            if first < last
                && !synced.is_empty()
                && self.watched.load(Ordering::Relaxed) != NOT_WATCHED
                && let Some((now, _)) = crate::watch::writes(first * PAGE, last * PAGE, true)
            {
                let base = start / PAGE;
                let mut copied = self.copied.lock().unwrap_or_else(|e| e.into_inner());
                for page in synced.into_iter().filter(|p| (first..last).contains(p)) {
                    copied[page - base] = now;
                }
            }
        }
        // Every submitted step that declared the range is done with it (one
        // submission runs at a time), and the snapshot with the last range.
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|&r| r != (address, address + len));
        if pending.is_empty()
            && let Some((_, bytes)) = snapshot.take()
        {
            unchanged.clear();
            give_back(bytes);
        }
        written
    }

    /// At submission: copies what changed of `[address, address + len)`
    /// since it was last copied (all of it if the memory is not
    /// write-watched). Returns the bytes copied.
    pub(crate) fn update(&self, address: usize, len: usize) -> usize {
        let (start, end) = (self.start as usize, (self.start + self.len) as usize);
        let (from, to) = (address.max(start), (address + len).min(end));
        if from >= to {
            return 0;
        }
        let mut busy: Vec<(usize, usize)> = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .copied()
            .filter(|&(a, e)| a < to && e > from)
            .collect();
        if !busy.is_empty() {
            // Around ranges shaders are writing: the gaps, each as below
            // (in the world most submissions have such ranges; copying the
            // gaps whole cost ~1 s per frame).
            busy.sort_unstable();
            let (mut at, mut bytes) = (from, 0);
            for (a, e) in busy {
                if a > at {
                    bytes += self.update_free(at, a);
                }
                at = at.max(e);
            }
            if at < to {
                bytes += self.update_free(at, to);
            }
            return bytes;
        }
        self.update_free(from, to)
    }

    /// [`Shadow::update`] of `[from, to)` (inside the buffer) where no
    /// shader writes.
    fn update_free(&self, from: usize, to: usize) -> usize {
        if self.watched.load(Ordering::Relaxed) != NOT_WATCHED {
            if let Some((now, written)) = crate::watch::writes(from, to, true) {
                self.watched.store(1, Ordering::Relaxed);
                let copied = self.copy_stale(from, to, now, &written);
                #[cfg(feature = "diagnostics")]
                {
                    use crate::copystat::{C, add};
                    add(C::Checked, (to - from) as u64);
                    add(C::Unchanged, (to - from).saturating_sub(copied) as u64);
                }
                return copied;
            }
            // Only mark the shadow permanently unwatched if it has never been
            // observed as watched. A sub-range that lands entirely in an
            // unmapped hole inside an already-watched view pool must not
            // disable write-watching for the rest of the pool.
            let _ =
                self.watched
                    .compare_exchange(0, NOT_WATCHED, Ordering::Relaxed, Ordering::Relaxed);
        }
        self.copy(from, to);
        to - from
    }

    /// Copies the pages of `[from, to)` written after their last copy
    /// (`written`: last writes by page, in page order; `now`: this
    /// asking), whole pages within the buffer.
    fn copy_stale(&self, from: usize, to: usize, now: u64, written: &[(usize, u64)]) -> usize {
        let base = self.start as usize / PAGE;
        let mut copied = self.copied.lock().unwrap_or_else(|e| e.into_inner());
        let mut writes = written.iter().peekable();
        let (mut bytes, mut run) = (0, None);
        let last = (to - 1) / PAGE;
        for page in from / PAGE..=last {
            let write = match writes.peek() {
                Some(&&(p, w)) if p == page => {
                    writes.next();
                    w
                }
                _ => 0,
            };
            let at = &mut copied[page - base];
            if *at == 0 || write > *at {
                // A page only partly in the range is copied in part: it
                // stays stale for the rest.
                let (start, end) = (self.start as usize, (self.start + self.len) as usize);
                if (page * PAGE).max(start) >= from && ((page + 1) * PAGE).min(end) <= to {
                    *at = now;
                }
                run.get_or_insert(page);
            } else if let Some(first) = run.take() {
                bytes += self.copy((first * PAGE).max(from), (page * PAGE).min(to));
            }
        }
        // Cut to the range: next to it shaders may be writing the copy.
        if let Some(first) = run {
            bytes += self.copy((first * PAGE).max(from), ((last + 1) * PAGE).min(to));
        }
        bytes
    }

    /// Copies title memory `[from, to)`, cut to the buffer, into the copy.
    fn copy(&self, from: usize, to: usize) -> usize {
        let (start, end) = (self.start as usize, (self.start + self.len) as usize);
        let (from, to) = (from.max(start), to.min(end));
        let off = from - start;
        let len = to - from;
        if let Some(rebar) = &self.rebar {
            let mut mirror = rebar.mirror.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(m) = mirror.as_mut()
                && off + len <= m.len()
            {
                // Copy once from guarded title memory into cached RAM, then
                // push straight to write-combining VRAM so both see identical bytes.
                crate::host::copy_mapped(
                    m.as_mut_ptr().wrapping_add(off),
                    from as *const u8,
                    len,
                    false,
                );
                // SAFETY: `off + len <= self.len`, write-only WC store.
                unsafe {
                    std::ptr::copy_nonoverlapping(m.as_ptr().add(off), self.data.add(off), len);
                }
                return len;
            }
        }
        // SAFETY: inside the mapping (`from` and `to` are within the buffer).
        let at = unsafe { self.data.add(off) };
        crate::host::copy_mapped(at, from as *const u8, len, false);
        len
    }
}

struct Entry {
    end: u64,
    shadow: Option<Arc<Shadow>>,
}

/// Title buffers by (start address, resource), with their copy once made.
static BUFFERS: Mutex<BTreeMap<(u64, usize), Entry>> = Mutex::new(BTreeMap::new());

fn buffers() -> std::sync::MutexGuard<'static, BTreeMap<(u64, usize), Entry>> {
    BUFFERS.lock().unwrap_or_else(|e| e.into_inner())
}

/// A title buffer exists at `start` (`len` bytes).
pub(crate) fn register(resource: usize, start: u64, len: u64) {
    buffers().insert(
        (start, resource),
        Entry {
            end: start + len,
            shadow: None,
        },
    );
}

/// The title released it; lists that still hold its copy keep it alive,
/// and any active epoch keeps it until that work completes.
pub(crate) fn unregister(resource: usize, start: u64) {
    let shadow = {
        let mut map = buffers();
        map.remove(&(start, resource)).and_then(|e| e.shadow)
    };
    if let Some(shadow) = shadow {
        crate::host::retire(shadow);
    }
}

/// Allocates a shadow backing buffer: prefers ReBAR (`D3D12_HEAP_TYPE_GPU_UPLOAD`)
/// VRAM within the adapter's budget, falling back cleanly to system-RAM
/// [`Host::shared_buffer`] when ReBAR is unavailable, disabled, or full.
fn alloc_shadow_buffer(size: u64) -> Option<(ID3D12Resource, *mut u8, Option<u64>)> {
    let h = host()?;
    let budget = h.rebar_shadow_budget();
    if try_reserve_rebar(&REBAR_SHADOW_BYTES, size, budget) {
        if let Some((buffer, data)) = h.try_rebar_shadow_buffer(size) {
            return Some((buffer, data, Some(size)));
        }
        REBAR_SHADOW_BYTES.fetch_sub(size, Ordering::Relaxed);
    }
    let (buffer, data) = h.shared_buffer(size)?;
    Some((buffer, data, None))
}

/// The copy of the innermost title buffer holding `[address, address +
/// len)`, made on first use.
pub(crate) fn find(address: u64, len: u64) -> Option<Arc<Shadow>> {
    let end = address.checked_add(len.max(1))?;
    let mut map = buffers();
    let key = map
        .range(..=(address, usize::MAX))
        .rev()
        .take(64)
        .find(|(_, e)| e.end >= end)
        .map(|(k, _)| *k);
    let Some(key) = key else {
        drop(map);
        return loose(address, end);
    };
    let entry = map.get_mut(&key)?;
    if entry.shadow.is_none() {
        let len = entry.end - key.0;
        // Rounded up so that views ending in the buffer's last 256 bytes fit.
        // Shaders write some buffers (unordered access), so not upload memory.
        let alloc_size = len.next_multiple_of(1 << 16);
        let (buffer, data, rebar_bytes) = alloc_shadow_buffer(alloc_size)?;
        crate::host::name(&buffer, || format!("title buffer {:#x}+{len:#x}", key.0));
        // SAFETY: a live buffer.
        let gpu = unsafe { buffer.GetGPUVirtualAddress() };
        if len >= 16 << 20 {
            tracing::info!(
                start = format_args!("{:#x}", key.0),
                len,
                rebar = rebar_bytes.is_some(),
                "large buffer copied to the host"
            );
        } else {
            tracing::debug!(
                start = format_args!("{:#x}", key.0),
                len,
                rebar = rebar_bytes.is_some(),
                "buffer copied to the host"
            );
        }
        entry.shadow = Some(Arc::new(Shadow::new(
            buffer,
            data,
            gpu,
            key.0,
            len,
            rebar_bytes,
        )));
    }
    entry.shadow.clone()
}

/// Titles also use memory outside any buffer object as GPU memory (Grand
/// Theft Auto VI's interface vertices, from its own allocator). Such memory
/// gets copies of the aligned windows it touches.
/// shortcut: copies of windows are never freed; key them to the title's
/// allocations if memory use grows.
const WINDOW: u64 = 4 << 20;

/// Copies of title memory outside buffers, by (start, length).
static LOOSE: Mutex<BTreeMap<(u64, u64), Arc<Shadow>>> = Mutex::new(BTreeMap::new());

fn loose(address: u64, end: u64) -> Option<Arc<Shadow>> {
    let start = address & !(WINDOW - 1);
    let len = end.next_multiple_of(WINDOW) - start;
    let mut map = LOOSE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(shadow) = map.get(&(start, len)) {
        return Some(shadow.clone());
    }
    let (buffer, data, rebar_bytes) = alloc_shadow_buffer(len)?;
    crate::host::name(&buffer, || format!("title memory {start:#x}+{len:#x}"));
    // SAFETY: a live buffer.
    let gpu = unsafe { buffer.GetGPUVirtualAddress() };
    tracing::debug!(
        start = format_args!("{start:#x}"),
        len,
        rebar = rebar_bytes.is_some(),
        "loose memory copied to the host"
    );
    let shadow = Arc::new(Shadow::new(buffer, data, gpu, start, len, rebar_bytes));
    map.insert((start, len), shadow.clone());
    Some(shadow)
}

/// For device-removed reports: the copy of title memory whose host range
/// starts closest below host GPU address `va`, and where `va` is in it.
pub(crate) fn near(va: u64) -> String {
    let mut all: Vec<(u64, u64, u64)> = buffers()
        .values()
        .filter_map(|e| e.shadow.as_deref().map(|s| (s.gpu, s.start, s.len)))
        .collect();
    all.extend(
        LOOSE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|s| (s.gpu, s.start, s.len)),
    );
    let below = all.iter().filter(|c| c.0 <= va).max();
    let above = all.iter().filter(|c| c.0 > va).min();
    let mut text = match below {
        Some(&(gpu, start, len)) => format!(
            "; the nearest title memory copy below it: {start:#x}+{len:#x} at host {gpu:#x}, offset {:#x}",
            va - gpu
        ),
        None => "; no title memory copy below it".into(),
    };
    if let Some(&(gpu, start, len)) = above {
        text.push_str(&format!(
            "; the nearest above: {start:#x}+{len:#x} at host {gpu:#x} ({} copies in all)",
            all.len()
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

    #[test]
    fn partial_match_leaves_unchecked_bytes_visible() {
        let mut unchanged = UnchangedPages::default();
        let before = vec![0; PAGE];
        let mut host = before.clone();
        host[PAGE - 1] = 1;
        unchanged.begin(PAGE, 2 * PAGE);

        assert!(!unchanged.differs(PAGE, &host[..4], &before[..4]));
        assert!(!unchanged.pages[0]);
        assert!(unchanged.differs(PAGE, &host, &before));
        assert!(!unchanged.pages[0]);
    }

    #[test]
    fn snapshot_edges_are_cacheable_and_survive_later_ranges() {
        let mut unchanged = UnchangedPages::default();
        unchanged.begin(PAGE - 3, PAGE + 4);
        assert!(!unchanged.differs(PAGE - 3, &[1; 3], &[1; 3]));
        assert!(!unchanged.differs(PAGE, &[2; 4], &[2; 4]));
        assert_eq!(unchanged.pages, [true, true]);

        unchanged.begin(PAGE - 3, PAGE + 4);
        assert!(!unchanged.differs(PAGE - 2, &[1; 2], &[1; 2]));
        assert!(!unchanged.differs(PAGE + 1, &[2; 2], &[2; 2]));
        assert_eq!(unchanged.pages, [true, true]);
    }

    #[test]
    fn next_submission_checks_the_same_page_again() {
        let mut unchanged = UnchangedPages::default();
        unchanged.begin(0, 4);
        assert!(!unchanged.differs(0, &[0; 4], &[0; 4]));
        unchanged.clear();
        unchanged.begin(0, 4);
        assert!(unchanged.differs(0, &[0, 0, 0, 1], &[0; 4]));
    }

    // Both versions walk exactly the same declared ranges, in their
    // original order. Only the page comparison differs.
    fn scan(
        host: &[u8],
        before: &[u8],
        ranges: &[Range<usize>],
        mut differs: impl FnMut(usize, &[u8], &[u8]) -> bool,
    ) -> usize {
        let mut changed = 0;
        for range in ranges {
            let mut at = range.start;
            while at < range.end {
                let n = (PAGE - at % PAGE).min(range.end - at);
                changed += usize::from(differs(at, &host[at..at + n], &before[at..at + n]));
                at += n;
            }
        }
        changed
    }

    #[test]
    fn overlapping_ranges_match_uncached_comparisons() {
        let before = vec![0; 4 * PAGE];
        let mut host = before.clone();
        for at in [2, PAGE - 1, PAGE + 5, 3 * PAGE + 7] {
            host[at] = 1;
        }
        let ranges = [
            0..4,
            2..2 * PAGE + 3,
            PAGE - 2..4 * PAGE,
            2 * PAGE..3 * PAGE,
        ];
        let uncached = scan(&host, &before, &ranges, |_, host, before| host != before);
        let mut unchanged = UnchangedPages::default();
        unchanged.begin(0, before.len());
        let cached = scan(&host, &before, &ranges, |at, host, before| {
            unchanged.differs(at, host, before)
        });
        assert_eq!(uncached, 6);
        assert_eq!(cached, uncached);
        assert_eq!(unchanged.pages, [false, false, true, false]);
    }

    // cargo test --release -p victory-gpu benchmark_overlapping_copy_back -- --ignored --nocapture
    #[test]
    #[ignore = "synthetic CPU release benchmark"]
    fn benchmark_overlapping_copy_back() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};

        const SIZE: usize = 128 << 20;
        let before = vec![0xA5; SIZE];
        let mut host = before.clone();
        for page in (0..SIZE / PAGE).step_by(64) {
            host[page * PAGE + PAGE / 2] ^= 0xFF;
        }
        let ranges: Vec<_> = (0..24).map(|i| i * PAGE + i..SIZE - i * 13).collect();
        let mut baseline = [Duration::ZERO; 5];
        let mut cached = [Duration::ZERO; 5];
        let mut unchanged = UnchangedPages::default();
        for (baseline, cached) in baseline.iter_mut().zip(&mut cached) {
            let started = Instant::now();
            let expected = scan(
                black_box(&host),
                black_box(&before),
                black_box(&ranges),
                |_, host, before| host != before,
            );
            *baseline = started.elapsed();

            unchanged.clear();
            let started = Instant::now();
            unchanged.begin(0, before.len());
            let actual = scan(
                black_box(&host),
                black_box(&before),
                black_box(&ranges),
                |at, host, before| unchanged.differs(at, host, before),
            );
            *cached = started.elapsed();
            assert_eq!(actual, expected);
        }
        baseline.sort_unstable();
        cached.sort_unstable();
        println!(
            "median of five runs, 24 overlapping ranges across 128 MiB: {:?} -> {:?} ({:.1}x)",
            baseline[2],
            cached[2],
            baseline[2].as_secs_f64() / cached[2].as_secs_f64(),
        );
    }

    #[cfg(windows)]
    #[test]
    fn unregister_drops_shadow_immediately_when_idle() {
        let _retire = crate::host::isolated_retire_scope();
        if host().is_none() {
            return;
        }
        let data = vec![0u8; 4096];
        let start = data.as_ptr() as u64;
        let resource = 0xdead_beef_0001_usize;
        register(resource, start, data.len() as u64);
        let weak = {
            let shadow = find(start, 64).expect("shadow");
            Arc::downgrade(&shadow)
        };
        assert_eq!(weak.strong_count(), 1);
        unregister(resource, start);
        assert_eq!(weak.strong_count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn unregister_retires_shadow_until_active_epoch_finishes() {
        let _retire = crate::host::isolated_retire_scope();
        let Some(host) = host() else {
            return;
        };
        let list = host.new_list().expect("host list");
        list.mark_work();

        let data = vec![0u8; 4096];
        let start = data.as_ptr() as u64;
        let resource = 0xdead_beef_0002_usize;
        register(resource, start, data.len() as u64);
        let weak = {
            let shadow = find(start, 64).expect("shadow");
            Arc::downgrade(&shadow)
        };
        assert_eq!(weak.strong_count(), 1);

        unregister(resource, start);
        assert_eq!(
            weak.strong_count(),
            1,
            "unregistered shadow must wait for active epochs"
        );

        list.close();
        list.reset();
        assert_eq!(weak.strong_count(), 0);
    }

    #[test]
    fn rebar_budget_reservation_enforces_cap_and_releases() {
        let used = AtomicU64::new(0);
        assert!(!try_reserve_rebar(&used, 64, 0));
        assert!(!try_reserve_rebar(&used, 0, 1024));
        assert!(!try_reserve_rebar(&used, 2048, 1024));
        assert!(try_reserve_rebar(&used, 600, 1024));
        assert_eq!(used.load(Ordering::Relaxed), 600);
        assert!(!try_reserve_rebar(&used, 500, 1024));
        assert!(try_reserve_rebar(&used, 424, 1024));
        assert_eq!(used.load(Ordering::Relaxed), 1024);
    }

    #[cfg(windows)]
    #[test]
    fn rebar_shadow_gpu_write_round_trips_and_preserves_concurrent_cpu_writes() {
        use windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_STATE_COPY_DEST;

        let _retire = crate::host::isolated_retire_scope();
        let Some(h) = host() else {
            return;
        };
        if h.rebar_shadow_budget() == 0 {
            // Requires a discrete GPU with D3D12 GPU upload heap (ReBAR) support.
            return;
        }
        let mut backing = vec![0x11u8; 4096];
        let start = backing.as_ptr() as u64;
        let resource = 0xdead_beef_0003_usize;
        register(resource, start, backing.len() as u64);

        let shadow = find(start, backing.len() as u64).expect("shadow");
        assert!(
            shadow.is_rebar(),
            "expected ReBAR-backed shadow on ReBAR adapter"
        );

        let list = h.new_list().expect("host list");
        // Seed initial title bytes, declare overlapping GPU writes, and overwrite
        // bytes 16..32 on the GPU via CopyBufferRegion while the CPU concurrently
        // writes byte 64 in the same page.
        shadow.refresh_as(
            &list,
            start,
            backing.len() as u64,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );
        shadow.gpu_writes(&list, start, 128);
        shadow.gpu_writes(&list, start + 16, 128);

        let (src_buf, src_ptr) = h.upload_buffer(16).expect("upload");
        // SAFETY: `src_ptr` is mapped for 16 bytes.
        unsafe {
            std::ptr::write_bytes(src_ptr, 0xAA, 16);
        }
        // SAFETY: live buffers and valid 16-byte copy inside both.
        unsafe {
            list.list
                .CopyBufferRegion(&shadow.buffer, 16, &src_buf, 0, 16);
        }
        // Simulate a concurrent CPU write to byte 64 while the GPU runs.
        let concurrent_addr = backing.as_mut_ptr() as usize;
        list.defer_host(move || {
            // SAFETY: `backing` stays alive for the duration of this test.
            unsafe {
                *(concurrent_addr as *mut u8).add(64) = 0x77;
            }
        });
        list.close();
        h.execute(&[&list]);

        assert!(backing[..16].iter().all(|&b| b == 0x11));
        assert!(backing[16..32].iter().all(|&b| b == 0xAA));
        assert_eq!(backing[64], 0x77);
        assert!(backing[65..128].iter().all(|&b| b == 0x11));

        list.reset();
        drop(shadow);
        unregister(resource, start);
    }
}
