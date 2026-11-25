//! Which title memory the title wrote since a host copy took it (decision
//! 0007, "Copies of title memory"). The runtime reserves title GPU memory
//! with write watching (`victory-hle` xmem), so the kernel keeps a dirty bit
//! per 4 KiB page; asking for a range's written pages also clears them. Host
//! copies of the same memory can overlap (nested buffers, textures in
//! buffers), so what one asks is kept here for the others: a journal of
//! each page's last write, numbered by the asking.
//!
//! Physical-page views (xmem's sections) cannot be write-watched. There a
//! copy leaves each 64 KiB block it took read-only, and our vectored handler
//! takes the first write to it: it notes the block as written and gives the
//! write access back. The runtime tells us when it maps or unmaps views
//! ([`title_memory_remapped`]), which resets their protection.
//!
//! shortcut: writes into physical pages through another view (xmem's
//! `write_physical`, DirectStorage's page destinations) are not seen at
//! the guarded addresses; they go to the texture pool, whose textures are
//! hashed, not guarded. A system call writing into a guarded block fails
//! (none is known to).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

/// The unit tracked and copied: 64 KiB, the console's page (the kernel's
/// 4 KiB pages fold into it). Finer units made the checks of the world's
/// big buffers cost more than the copies they saved.
pub(crate) const PAGE: usize = 64 << 10;
/// The kernel's page, the unit of its answers.
const KERNEL_PAGE: usize = 4096;

struct Journal {
    /// Numbers the askings; 0 is "never".
    asked: u64,
    /// Page number → the asking that last found it written.
    written: BTreeMap<usize, u64>,
    /// Room for the kernel's answer.
    found: Vec<usize>,
}

impl Journal {
    fn snapshot(&self, first: usize, last: usize) -> (u64, Vec<(usize, u64)>) {
        (
            self.asked,
            self.written
                .range(first..last)
                .map(|(&p, &w)| (p, w))
                .collect(),
        )
    }
}

static JOURNAL: Mutex<Journal> = Mutex::new(Journal {
    asked: 0,
    written: BTreeMap::new(),
    found: Vec::new(),
});

/// The pages of `[start, end)` (whole pages) as last written, for pages
/// written since tracking began: `(page number, asking)` in page order, and
/// this asking's number, which is higher than any in the list but those the
/// kernel just reported. `None` if the memory is not write-watched (nor,
/// with `guarded`, view memory, which is then guarded).
pub(crate) fn writes(start: usize, end: usize, guarded: bool) -> Option<(u64, Vec<(usize, u64)>)> {
    use windows_sys::Win32::System::Memory::GetWriteWatch;
    const WRITE_WATCH_FLAG_RESET: u32 = 1;
    let (first, last) = (start / PAGE, end.div_ceil(PAGE));
    let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    let j = &mut *j;
    #[cfg(feature = "diagnostics")]
    let started = std::time::Instant::now();
    // Clean mapped-view blocks are still protected. A write, kernel-write
    // announcement or remap invalidates the corresponding entries.
    if guarded && first < last && guards().clean.range(first..last).copied().eq(first..last) {
        j.asked += 1;
        #[cfg(feature = "diagnostics")]
        {
            use crate::copystat::{C, add};
            add(C::Watches, 1);
            add(C::WatchUs, started.elapsed().as_micros() as u64);
        }
        return Some(j.snapshot(first, last));
    }
    let mut count = (last - first) * PAGE / KERNEL_PAGE;
    j.found.resize(count.max(j.found.len()), 0);
    let mut granularity = 0;
    // SAFETY: the answer buffer holds `count` entries.
    let failed = unsafe {
        GetWriteWatch(
            WRITE_WATCH_FLAG_RESET,
            (first * PAGE) as *const _,
            (last - first) * PAGE,
            j.found.as_mut_ptr().cast(),
            &mut count,
            &mut granularity,
        )
    } != 0;
    let guarded = if failed {
        count = 0;
        match by_allocation(first * PAGE, last * PAGE, &mut j.found) {
            Some(pages) => pages,
            None if !guarded => return None,
            None => guard(first * PAGE, last * PAGE)?,
        }
    } else {
        Vec::new()
    };
    #[cfg(feature = "diagnostics")]
    {
        use crate::copystat::{C, add};
        add(C::Watches, 1);
        add(C::WatchUs, started.elapsed().as_micros() as u64);
    }
    j.asked += 1;
    let now = j.asked;
    let pages = j.found[..count].iter().map(|&at| at / PAGE).chain(guarded);
    for page in pages.collect::<Vec<_>>() {
        j.written.insert(page, now);
    }
    Some(j.snapshot(first, last))
}

/// The pages of `[start, end)` written, asked allocation by allocation:
/// the kernel refuses a range over several (a title buffer spans as many
/// allocations as the runtime made for it). Parts never committed cannot
/// have been written, so a range none of which is committed (a reserved
/// allocation the title has not committed yet) has no writes. `None` if a
/// committed part is not write-watched (or the range is one committed
/// allocation, already refused).
fn by_allocation(start: usize, end: usize, found: &mut Vec<usize>) -> Option<Vec<usize>> {
    use windows_sys::Win32::System::Memory::{
        GetWriteWatch, MEM_COMMIT, MEM_FREE, MEM_MAPPED, MEMORY_BASIC_INFORMATION, VirtualQuery,
    };
    const WRITE_WATCH_FLAG_RESET: u32 = 1;
    let query = |at: usize| {
        let mut info = MEMORY_BASIC_INFORMATION::default();
        // SAFETY: a query about an address; nothing is dereferenced.
        let n = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) };
        (n != 0).then_some(info)
    };
    let (mut pages, mut at, mut allocations, mut any) = (Vec::new(), start, 0, false);
    while at < end {
        let info = query(at)?;
        // A committed section view cannot belong to a MEM_WRITE_WATCH
        // reservation. Its other protection regions need no further queries.
        if info.State == MEM_COMMIT && info.Type == MEM_MAPPED {
            return None;
        }
        let base = info.AllocationBase as usize;
        let mut committed = info.State == MEM_COMMIT;
        let mut to = (info.BaseAddress as usize + info.RegionSize).min(end);
        // The rest of the allocation's regions in the range.
        while info.State != MEM_FREE && to < end {
            let next = query(to)?;
            if next.State == MEM_FREE || next.AllocationBase as usize != base {
                break;
            }
            committed |= next.State == MEM_COMMIT;
            to = (next.BaseAddress as usize + next.RegionSize).min(end);
        }
        allocations += 1;
        any |= committed;
        if committed {
            let mut count = (to - at) / KERNEL_PAGE;
            found.resize(count.max(found.len()), 0);
            let mut granularity = 0;
            // SAFETY: the answer buffer holds `count` entries.
            let failed = unsafe {
                GetWriteWatch(
                    WRITE_WATCH_FLAG_RESET,
                    at as *const _,
                    to - at,
                    found.as_mut_ptr().cast(),
                    &mut count,
                    &mut granularity,
                )
            } != 0;
            if failed {
                return None;
            }
            pages.extend(found[..count].iter().map(|&a| a / PAGE));
        }
        at = to;
    }
    (allocations > 1 || !any).then_some(pages)
}

/// Guarded blocks of view memory.
const BLOCK: usize = PAGE;

struct Guards {
    /// Blocks we made read-only.
    protected: BTreeSet<usize>,
    /// Of those, the ones not written since a copy took them.
    clean: BTreeSet<usize>,
    /// Announced kernel writes still to land, as block ranges `[first, end)`
    /// with how many announcements of that range are open: their blocks are
    /// left open until they have, however many refreshes come first. Kept
    /// as ranges, not per block, so a title-supplied length (saturated by
    /// `fread`'s size times count) costs nothing however large; counted, so
    /// a range announced again and again (overlapped reads into the same
    /// buffer, which never say they landed) stays one entry.
    announced: BTreeMap<(usize, usize), u32>,
}

impl Guards {
    /// Whether an announced kernel write still to land covers `block`.
    fn announced(&self, block: usize) -> bool {
        self.announced
            .keys()
            .any(|&(first, end)| (first..end).contains(&block))
    }
}

static GUARDS: Mutex<Guards> = Mutex::new(Guards {
    protected: BTreeSet::new(),
    clean: BTreeSet::new(),
    announced: BTreeMap::new(),
});

fn guards() -> std::sync::MutexGuard<'static, Guards> {
    GUARDS.lock().unwrap_or_else(|e| e.into_inner())
}

/// For view memory: the pages of `[start, end)`'s blocks written since
/// they were last guarded (all of a block's pages), guarding them again
/// before the caller copies them. `None` if the range contains no mapped view.
fn guard(start: usize, end: usize) -> Option<Vec<usize>> {
    if !has_mapped(start, end) {
        return None;
    }
    install();
    let mut g = guards();
    let mut written = Vec::new();
    let mut run: Option<usize> = None;
    let (first, last) = (start / BLOCK, end.div_ceil(BLOCK));
    for block in first..=last {
        let open = block < last && !g.clean.contains(&block);
        if open {
            written.extend(block * BLOCK / PAGE..(block + 1) * BLOCK / PAGE);
            if g.protected.contains(&block) {
                // Written through the runtime, still read-only.
                g.clean.insert(block);
            } else if !g.announced(block) {
                run.get_or_insert(block);
                continue;
            }
        }
        if let Some(from) = run.take() {
            protect(&mut g, from, block);
        }
    }
    Some(written)
}

/// Whether `[start, end)` contains a mapped view and no committed non-view
/// memory. Checks `start` first via [`mapped`] (fast path); when `start` is
/// not a view (e.g. an unmapped placeholder/hole inside a view pool), walks
/// `VirtualQuery` across `[start, end)` and rejects the range if any
/// committed non-view region is present so private/image memory is never
/// read-protected.
fn has_mapped(start: usize, end: usize) -> bool {
    if mapped(start) {
        return true;
    }
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_MAPPED, MEMORY_BASIC_INFORMATION, VirtualQuery,
    };
    let mut at = start;
    let mut found_mapped = false;
    while at < end {
        let mut info = MEMORY_BASIC_INFORMATION::default();
        // SAFETY: queries virtual memory metadata without dereferencing `at`.
        if unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) } == 0
            || info.RegionSize == 0
        {
            break;
        }
        if info.Type == MEM_MAPPED {
            found_mapped = true;
        } else if info.State == MEM_COMMIT {
            return false;
        }
        let Some(next) = (info.BaseAddress as usize).checked_add(info.RegionSize) else {
            break;
        };
        if next <= at {
            break;
        }
        at = next;
    }
    found_mapped
}

/// Whether the current allocation at `at` is a mapped view. Asking only for
/// the allocation type avoids enumerating pages with identical protection,
/// which can span an entire large view after its blocks were guarded.
fn mapped(at: usize) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_MAPPED, MEMORY_BASIC_INFORMATION, MemoryRegionInfo, QueryVirtualMemoryInformation,
        VirtualQuery, WIN32_MEMORY_REGION_INFORMATION,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut region = WIN32_MEMORY_REGION_INFORMATION::default();
    // SAFETY: both queries describe an address without dereferencing it. The
    // output buffers match their information classes.
    let queried = unsafe {
        QueryVirtualMemoryInformation(
            GetCurrentProcess(),
            at as *const _,
            MemoryRegionInfo,
            (&raw mut region).cast(),
            size_of_val(&region),
            std::ptr::null_mut(),
        )
    };
    if queried != 0 {
        // These are the Windows SDK's named bitfields in declaration order.
        // windows-sys exposes only the union's raw bits.
        const PRIVATE: u32 = 1 << 0;
        const MAPPED_DATA_FILE: u32 = 1 << 1;
        const MAPPED_IMAGE: u32 = 1 << 2;
        const MAPPED_PAGE_FILE: u32 = 1 << 3;
        const MAPPED_PHYSICAL: u32 = 1 << 4;
        // SAFETY: this union was filled by the successful query above.
        match unsafe { region.Anonymous.Flags } {
            MAPPED_DATA_FILE | MAPPED_PAGE_FILE | MAPPED_PHYSICAL => return true,
            PRIVATE | MAPPED_IMAGE => return false,
            _ => {}
        }
    }
    // Unknown flag combinations and failed allocation queries keep the
    // VirtualQuery type check, including for a free address.
    let mut info = MEMORY_BASIC_INFORMATION::default();
    // SAFETY: the output buffer has the required layout and size.
    let n = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) };
    n != 0 && info.Type == MEM_MAPPED
}

/// Makes blocks `[from, to)` read-only and notes them guarded and clean;
/// block by block if they span views, leaving out any that fail (unmapped).
fn protect(g: &mut Guards, from: usize, to: usize) {
    use windows_sys::Win32::System::Memory::{PAGE_READONLY, VirtualProtect};
    let set = |from: usize, to: usize| {
        let mut old = 0;
        // SAFETY: title view memory; only its protection changes.
        unsafe {
            VirtualProtect(
                (from * BLOCK) as *const _,
                (to - from) * BLOCK,
                PAGE_READONLY,
                &mut old,
            ) != 0
        }
    };
    let note = |g: &mut Guards, from: usize, to: usize| {
        g.protected.extend(from..to);
        g.clean.extend(from..to);
    };
    if set(from, to) {
        note(g, from, to);
        return;
    }
    for block in from..to {
        if set(block, block + 1) {
            note(g, block, block + 1);
        }
    }
}

/// Gives write access back to the guarded blocks of `[start, end)` and
/// notes them written; true if any was guarded.
fn open(g: &mut Guards, start: usize, end: usize) -> bool {
    use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};
    let blocks: Vec<usize> = g
        .protected
        .range(start / BLOCK..end.div_ceil(BLOCK))
        .copied()
        .collect();
    for &block in &blocks {
        g.protected.remove(&block);
        g.clean.remove(&block);
        let mut old = 0;
        // SAFETY: a block we made read-only.
        unsafe { VirtualProtect((block * BLOCK) as *const _, BLOCK, PAGE_READWRITE, &mut old) };
    }
    !blocks.is_empty()
}

/// Before our own write into title memory `[at, at + len)` through the
/// kernel (which does not fault into our handler): opens its guarded blocks.
/// For the title's kernel reads, which other threads' refreshes can
/// overtake, the runtime calls [`kernel_will_write`] instead.
pub(crate) fn writing(at: usize, len: usize) {
    // The length is the title's (`fread`'s size times count, saturated).
    open(&mut guards(), at, at.saturating_add(len));
}

/// Before the title's kernel read into its memory `[at, at + len)`: opens
/// the guarded blocks there and keeps them open until [`kernel_wrote`]. A
/// refresh on another thread in between would otherwise guard them again,
/// and the read would fail.
pub fn kernel_will_write(at: usize, len: usize) {
    let end = at.saturating_add(len);
    let mut g = guards();
    open(&mut g, at, end);
    *g.announced
        .entry((at / BLOCK, end.div_ceil(BLOCK)))
        .or_default() += 1;
}

/// The kernel write [`kernel_will_write`] announced for `[at, at + len)`
/// has landed (or failed); its blocks can be guarded again.
pub fn kernel_wrote(at: usize, len: usize) {
    let range = (at / BLOCK, at.saturating_add(len).div_ceil(BLOCK));
    let mut g = guards();
    if let Some(n) = g.announced.get_mut(&range) {
        *n -= 1;
        if *n == 0 {
            g.announced.remove(&range);
        }
    }
}

/// A storage worker finished writing these mapped guest pages through another
/// view of the same section. Their address protections are still in effect.
pub fn mapped_pages_written(addresses: impl Iterator<Item = usize>) {
    let mut addresses = addresses.peekable();
    if addresses.peek().is_none() {
        return;
    }
    let mut g = guards();
    for at in addresses {
        g.clean.remove(&(at / PAGE));
    }
}

/// The runtime mapped or unmapped title memory `[at, at + len)`: what it
/// shows is new and read-write, so it is no longer guarded, and kernel
/// writes announced wholly inside it are moot (an overlapped read that
/// never said it landed no longer keeps those blocks open).
pub fn title_memory_remapped(at: usize, len: usize) {
    let (first, end) = (at / BLOCK, at.saturating_add(len).div_ceil(BLOCK));
    let mut g = guards();
    let blocks: Vec<usize> = g.protected.range(first..end).copied().collect();
    for block in blocks {
        g.protected.remove(&block);
        g.clean.remove(&block);
    }
    g.announced
        .retain(|&(from, to), _| !(first <= from && to <= end));
}

/// Coordinates in-place CPU reads of title memory with mapping changes.
///
/// Only used when `VICTORY_MAPPING_RANGES=0` ([`by_range`]); otherwise reads
/// and changes hold only the bytes they touch ([`SPANS`]).
static MAPPING: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// Whether a read in place and a change of mapping keep apart only when
/// their bytes meet ([`SPANS`]), not whatever memory each touches
/// ([`MAPPING`]); `VICTORY_MAPPING_RANGES=0` holds all of title memory for
/// each. On by default.
pub(crate) fn by_range() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_MAPPING_RANGES").is_none_or(|v| v != "0"))
}

/// Reads of title memory in place under way, and changes of its mapping,
/// by the bytes they touch ([`by_range`]).
struct Spans {
    /// To tell them apart.
    next: u64,
    /// Reads under way: number, start and end.
    reads: Vec<(u64, usize, usize)>,
    /// Changes: number, start, end, and whether under way (else waiting for
    /// the reads over its bytes to end; no read begins over them meanwhile,
    /// so that one read after another cannot keep a change waiting).
    changes: Vec<(u64, usize, usize, bool)>,
    /// Threads waiting on [`SPANS_LEFT`].
    waiting: usize,
}

static SPANS: Mutex<Spans> = Mutex::new(Spans {
    next: 0,
    reads: Vec::new(),
    changes: Vec::new(),
    waiting: 0,
});

/// Told when a read or a change ends while a thread waits.
static SPANS_LEFT: std::sync::Condvar = std::sync::Condvar::new();

fn spans() -> std::sync::MutexGuard<'static, Spans> {
    SPANS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Waits until a read or a change ends.
fn spans_wait(
    mut s: std::sync::MutexGuard<'static, Spans>,
) -> std::sync::MutexGuard<'static, Spans> {
    s.waiting += 1;
    let mut s = SPANS_LEFT.wait(s).unwrap_or_else(|e| e.into_inner());
    s.waiting -= 1;
    s
}

/// Whether `[a, b)` and `[c, d)` share a byte.
fn meet(a: usize, b: usize, c: usize, d: usize) -> bool {
    a < d && c < b
}

impl Spans {
    fn read(&mut self, at: usize, end: usize) -> Span {
        self.next += 1;
        self.reads.push((self.next, at, end));
        Span(self.next, false)
    }
}

/// A read's or a change's place in [`SPANS`] (its number, and whether a
/// change), given up when dropped.
struct Span(u64, bool);

impl Drop for Span {
    fn drop(&mut self) {
        let mut s = spans();
        if self.1 {
            s.changes.retain(|c| c.0 != self.0);
        } else if let Some(i) = s.reads.iter().position(|r| r.0 == self.0) {
            s.reads.swap_remove(i);
        }
        if s.waiting > 0 {
            SPANS_LEFT.notify_all();
        }
    }
}

/// What a read or a change of mapping holds: its bytes ([`by_range`]) or
/// the lock over all of title memory.
enum Held<G> {
    All(#[allow(dead_code)] G),
    Bytes(#[allow(dead_code)] Span),
}

/// Since the last frame-rate line: changes of title memory's mapping, the
/// microseconds they waited for reads in place, and those they were under
/// way.
pub(crate) static REMAPS: [std::sync::atomic::AtomicU64; 3] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 3];

/// What the runtime holds while it changes what title memory shows
/// ([`title_memory_remapping`]).
pub struct Remapping {
    _held: Held<std::sync::RwLockWriteGuard<'static, ()>>,
    since: std::time::Instant,
}

impl Drop for Remapping {
    fn drop(&mut self) {
        REMAPS[2].fetch_add(
            self.since.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

/// The runtime is about to map, unmap or move physical pages of title
/// memory `[at, at + len)`: until what it returns is dropped, no read of
/// those bytes in place is under way and none begins
/// ([`title_memory_reading`]). Taken around the whole change, the unmapping
/// and the mapping again, over all the bytes whose mapping it changes.
pub fn title_memory_remapping(at: usize, len: usize) -> Remapping {
    use std::sync::atomic::Ordering::Relaxed;
    let started = std::time::Instant::now();
    let held = if by_range() {
        let end = at.saturating_add(len);
        let mut s = spans();
        s.next += 1;
        let id = s.next;
        s.changes.push((id, at, end, false));
        while s.reads.iter().any(|r| meet(r.1, r.2, at, end))
            || s.changes.iter().any(|c| c.3 && meet(c.1, c.2, at, end))
        {
            s = spans_wait(s);
        }
        if let Some(c) = s.changes.iter_mut().find(|c| c.0 == id) {
            c.3 = true;
        }
        Held::Bytes(Span(id, true))
    } else {
        Held::All(MAPPING.write().unwrap_or_else(|e| e.into_inner()))
    };
    REMAPS[0].fetch_add(1, Relaxed);
    REMAPS[1].fetch_add(started.elapsed().as_micros() as u64, Relaxed);
    Remapping {
        _held: held,
        since: std::time::Instant::now(),
    }
}

/// What a read of title memory in place holds ([`title_memory_reading`]).
pub struct Reading(#[allow(dead_code)] Held<std::sync::RwLockReadGuard<'static, ()>>);

/// For reading title memory `[at, at + len)` in place: until what it
/// returns is dropped, the runtime changes no mapping of those bytes, so
/// memory found mapped stays mapped. Not to be taken twice by one thread,
/// nor held across anything that maps title memory.
pub fn title_memory_reading(at: usize, len: usize) -> Reading {
    if !by_range() {
        return Reading(Held::All(MAPPING.read().unwrap_or_else(|e| e.into_inner())));
    }
    let end = at.saturating_add(len);
    let mut s = spans();
    while s.changes.iter().any(|c| meet(c.1, c.2, at, end)) {
        s = spans_wait(s);
    }
    Reading(Held::Bytes(s.read(at, end)))
}

/// [`title_memory_reading`] if it can be had at once: `None` while the
/// runtime changes a mapping of those bytes (or waits to), for a caller
/// that has another way to read and would rather not wait.
pub fn title_memory_reading_now(at: usize, len: usize) -> Option<Reading> {
    if !by_range() {
        return match MAPPING.try_read() {
            Ok(guard) => Some(Reading(Held::All(guard))),
            Err(std::sync::TryLockError::Poisoned(e)) => Some(Reading(Held::All(e.into_inner()))),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
    }
    let end = at.saturating_add(len);
    let mut s = spans();
    if s.changes.iter().any(|c| meet(c.1, c.2, at, end)) {
        return None;
    }
    Some(Reading(Held::Bytes(s.read(at, end))))
}

/// Registers the handler that takes writes to guarded blocks, once, ahead
/// of the crash reporter's.
fn install() {
    use windows_sys::Win32::Foundation::{EXCEPTION_ACCESS_VIOLATION, NTSTATUS};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH,
        EXCEPTION_POINTERS,
    };
    unsafe extern "system" fn on_write(info: *mut EXCEPTION_POINTERS) -> i32 {
        // SAFETY: the system passes the exception being dispatched.
        let r = unsafe { &*(*info).ExceptionRecord };
        const WRITE: usize = 1;
        if r.ExceptionCode != EXCEPTION_ACCESS_VIOLATION as NTSTATUS
            || r.NumberParameters < 2
            || r.ExceptionInformation[0] != WRITE
        {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let at = r.ExceptionInformation[1];
        // Held across both checks: another thread may have opened the block
        // since this one faulted, and a copy may have guarded it again since
        // (then it is ours again, not read-only for some other reason).
        let mut g = guards();
        if open(&mut g, at, at + 1) {
            #[cfg(feature = "diagnostics")]
            crate::copystat::add(crate::copystat::C::GuardFaults, 1);
            return EXCEPTION_CONTINUE_EXECUTION;
        }
        if writable(at) {
            EXCEPTION_CONTINUE_EXECUTION
        } else {
            EXCEPTION_CONTINUE_SEARCH
        }
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: the handler only touches our guard state and protections.
    ONCE.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, Some(on_write));
    });
}

/// Whether `at` is committed read-write memory now.
fn writable(at: usize) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_READWRITE, VirtualQuery,
    };
    // SAFETY: plain data filled in by the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size.
    let n = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) };
    n != 0 && info.State == MEM_COMMIT && info.Protect == PAGE_READWRITE
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
    };
    const MEM_WRITE_WATCH: u32 = 0x20_0000;

    #[test]
    fn allocation_query_recognizes_file_views_and_rejects_nonviews() {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY, UnmapViewOfFile,
        };
        let file = tempfile::tempfile().unwrap();
        file.set_len(PAGE as u64).unwrap();
        // SAFETY: a valid, open file and a read-only mapping of its contents.
        let section = unsafe {
            CreateFileMappingW(
                file.as_raw_handle(),
                std::ptr::null(),
                PAGE_READONLY,
                0,
                0,
                std::ptr::null(),
            )
        };
        assert!(!section.is_null());
        // SAFETY: an open section containing at least one page.
        let view = unsafe { MapViewOfFile(section, FILE_MAP_READ, 0, 0, PAGE) };
        assert!(!view.Value.is_null());
        assert!(mapped(view.Value as usize + 13));
        // SAFETY: our successful mapping and its section handle.
        unsafe {
            UnmapViewOfFile(view);
            CloseHandle(section);
        }
        assert!(!mapped((&raw const file) as usize)); // private stack allocation
        assert!(!mapped(mapped as *const () as usize)); // executable image
        assert!(!mapped(0)); // no allocation to query
    }

    #[test]
    fn written_pages_are_reported_to_every_asker() {
        // SAFETY: a fresh write-watched allocation, freed at the end.
        let p = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                4 * PAGE,
                MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH,
                PAGE_READWRITE,
            )
        } as usize;
        assert_ne!(p, 0);
        let (start, end) = (p, p + 4 * PAGE);
        // A previous test may have used these virtual addresses.
        let mut journal = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
        for page in start / PAGE..end / PAGE {
            journal.written.remove(&page);
        }
        drop(journal);
        let (_, before) = writes(start, end, false).expect("watched");
        assert!(before.is_empty());
        // SAFETY: committed pages of ours.
        unsafe { ((p + PAGE + 8) as *mut u8).write(1) };
        let (now, after) = writes(start, end, false).expect("watched");
        assert_eq!(after, [(p / PAGE + 1, now)]);
        // Another asker after the kernel's bit was cleared still sees it.
        let (later, again) = writes(p + PAGE, p + 2 * PAGE, false).expect("watched");
        assert!(later > now);
        assert_eq!(again, [(p / PAGE + 1, now)]);
        let mut plain = vec![0u8; PAGE];
        assert!(
            writes(
                plain.as_mut_ptr() as usize,
                plain.as_ptr() as usize + PAGE,
                true
            )
            .is_none()
        );
        // SAFETY: our allocation.
        unsafe { VirtualFree(p as *mut _, 0, MEM_RELEASE) };
    }

    /// Two write-watched allocations side by side (the runtime reserves
    /// title memory allocation by allocation), asked about as one range, as
    /// a buffer spanning them is: the pages written in each are reported.
    #[test]
    fn a_range_over_several_watched_allocations_is_watched() {
        let (a, b) = loop {
            // SAFETY: a probe reservation, released at once to find room for two.
            let span =
                unsafe { VirtualAlloc(std::ptr::null(), 4 * PAGE, MEM_RESERVE, PAGE_READWRITE) };
            assert!(!span.is_null());
            // SAFETY: the probe above.
            unsafe { VirtualFree(span, 0, MEM_RELEASE) };
            let at = |p: usize| {
                // SAFETY: a fixed address in the room just released.
                let got = unsafe {
                    VirtualAlloc(
                        p as *const _,
                        2 * PAGE,
                        MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH,
                        PAGE_READWRITE,
                    )
                };
                got as usize
            };
            let a = at(span as usize);
            let b = at(span as usize + 2 * PAGE);
            if a != 0 && b != 0 {
                break (a, b);
            }
            for p in [a, b].into_iter().filter(|&p| p != 0) {
                // SAFETY: ours.
                unsafe { VirtualFree(p as *mut _, 0, MEM_RELEASE) };
            }
        };
        let _ = writes(a, b + 2 * PAGE, false);
        // SAFETY: committed pages of ours.
        unsafe {
            ((a + 8) as *mut u8).write(1);
            ((b + PAGE + 8) as *mut u8).write(1);
        }
        let (now, found) = writes(a, b + 2 * PAGE, false).expect("watched");
        assert_eq!(found, [(a / PAGE, now), (b / PAGE + 1, now)]);
        // Not freed: the journal keeps these pages, and a later test given
        // the same addresses would find them written.
    }

    /// A range in an allocation reserved but not committed yet (as the
    /// title's are before it uses them) has no writes, and is asked again
    /// once committed.
    #[test]
    fn a_range_never_committed_has_no_writes() {
        // SAFETY: a fresh write-watched reservation, freed at the end.
        let p = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                2 * PAGE,
                MEM_RESERVE | MEM_WRITE_WATCH,
                PAGE_READWRITE,
            )
        } as usize;
        assert_ne!(p, 0);
        let (_, none) = writes(p, p + 2 * PAGE, false).expect("nothing can be written there");
        assert!(none.is_empty());
        // SAFETY: committing and writing a page of our reservation.
        unsafe {
            assert!(!VirtualAlloc(p as *const _, PAGE, MEM_COMMIT, PAGE_READWRITE).is_null());
            ((p + 8) as *mut u8).write(1);
        }
        let (now, found) = writes(p, p + 2 * PAGE, false).expect("watched");
        assert_eq!(found, [(p / PAGE, now)]);
        // Not freed: the journal keeps these pages, and a later test given
        // the same addresses would find them written.
    }

    /// A kernel read into a guarded view block fails; opened first, it
    /// lands and the block counts as written.
    #[test]
    fn kernel_reads_into_guarded_blocks_after_opening() {
        use std::io::{Read, Seek, Write};
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MapViewOfFile, UnmapViewOfFile,
        };
        // SAFETY: a pagefile section of two blocks, mapped read-write.
        let view = unsafe {
            let section = CreateFileMappingW(
                -1isize as _,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                (2 * BLOCK) as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, 2 * BLOCK).Value as usize
        };
        assert_ne!(view, 0);
        let end = view + 2 * BLOCK;
        let (first, initial) = writes(view, end, true).expect("new mapped view");
        assert_eq!(initial, [(view / PAGE, first), (view / PAGE + 1, first)]);
        let (next, clean) = writes(view + 8, end - 1, true).expect("same protected pages");
        assert!(next > first);
        assert_eq!(clean, initial);
        mapped_pages_written(std::iter::once(view + BLOCK));
        let (alias, updated) = writes(view, end, true).expect("alias write");
        assert_eq!(updated, [(view / PAGE, first), (view / PAGE + 1, alias)]);
        assert_eq!(writes(view, end, true).unwrap().1, updated);
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"from the disk").unwrap();
        // `ReadFile` inside: the kernel writes the view.
        let read = |file: &mut std::fs::File| {
            file.rewind().unwrap();
            // SAFETY: 13 bytes of our view, nothing else refers to them.
            let into = unsafe { std::slice::from_raw_parts_mut((view + 100) as *mut u8, 13) };
            file.read(into).is_ok_and(|n| n == 13)
        };
        assert!(
            !read(&mut file),
            "the kernel does not fault into our handler"
        );
        writing(view + 100, 13);
        assert!(read(&mut file));
        assert_eq!(
            // SAFETY: the bytes just read.
            unsafe { std::slice::from_raw_parts((view + 100) as *const u8, 13) },
            b"from the disk"
        );
        let (changed, pages) = writes(view, end, true).expect("kernel write visible");
        assert_eq!(pages, [(view / PAGE, changed), (view / PAGE + 1, alias)]);
        assert_eq!(writes(view, end, true).unwrap().1, pages);
        // SAFETY: our view.
        unsafe {
            UnmapViewOfFile(
                windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: view as *mut _,
                },
            )
        };
        title_memory_remapped(view, 2 * BLOCK);
    }

    /// A range that begins on an unmapped reserved hole before a mapped view
    /// is still guarded, whereas a range starting in committed private memory
    /// before a mapped view is rejected so private pages are never
    /// read-protected.
    #[test]
    fn guard_handles_unmapped_prefix_and_rejects_committed_private_prefix() {
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFileEx,
            PAGE_NOACCESS, UnmapViewOfFile,
        };
        // SAFETY: create a 1-block pagefile section.
        let section = unsafe {
            CreateFileMappingW(
                -1isize as _,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                BLOCK as u32,
                std::ptr::null(),
            )
        };
        assert!(!section.is_null());

        // Deterministically place an unmapped reserved block immediately before
        // a mapped view: reserve 2 blocks, release, then re-reserve block 0 and
        // map the view at block 1.
        let mut pair = None;
        for _ in 0..16 {
            // SAFETY: probe for a free 2-block-aligned address range.
            let probe =
                unsafe { VirtualAlloc(std::ptr::null(), 2 * BLOCK, MEM_RESERVE, PAGE_NOACCESS) }
                    as usize;
            assert_ne!(probe, 0);
            // SAFETY: freeing the probe we just reserved.
            assert_ne!(unsafe { VirtualFree(probe as *mut _, 0, MEM_RELEASE) }, 0);
            // SAFETY: reserve block 0 as an unmapped placeholder.
            let prefix =
                unsafe { VirtualAlloc(probe as *const _, BLOCK, MEM_RESERVE, PAGE_NOACCESS) }
                    as usize;
            if prefix != probe {
                if prefix != 0 {
                    // SAFETY: freeing our reservation.
                    unsafe { VirtualFree(prefix as *mut _, 0, MEM_RELEASE) };
                }
                continue;
            }
            // SAFETY: map the 1-block section immediately after the reserved prefix.
            let view = unsafe {
                MapViewOfFileEx(
                    section,
                    FILE_MAP_WRITE,
                    0,
                    0,
                    BLOCK,
                    (probe + BLOCK) as *const _,
                )
                .Value as usize
            };
            if view == probe + BLOCK {
                pair = Some((prefix, view));
                break;
            }
            if view != 0 {
                // SAFETY: unmapping our view.
                unsafe {
                    UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                        Value: view as *mut _,
                    });
                }
            }
            // SAFETY: freeing our prefix reservation.
            unsafe { VirtualFree(prefix as *mut _, 0, MEM_RELEASE) };
        }
        let (prefix, view) = pair.expect("reserve prefix block immediately before mapped view");

        // 1. Unmapped reserved prefix + mapped view: must stay guarded.
        assert!(!mapped(prefix));
        assert!(
            guard(prefix, view + BLOCK).is_some(),
            "range spanning an unmapped reserved prefix into a mapped view must stay guarded"
        );
        // SAFETY: writing inside our mapped read-write view.
        unsafe { (view as *mut u8).write(42) };
        let written = guard(prefix, view + BLOCK).expect("still guarded");
        assert!(written.contains(&(view / PAGE)));

        // 2. Committed private prefix + mapped view: must NOT read-protect private memory.
        // SAFETY: committing our reserved prefix block as private read-write memory.
        let committed =
            unsafe { VirtualAlloc(prefix as *const _, BLOCK, MEM_COMMIT, PAGE_READWRITE) };
        assert!(!committed.is_null());
        assert!(
            guard(prefix, view + BLOCK).is_none(),
            "range containing committed private memory must not be read-protected by guard"
        );
        // SAFETY: writing to the committed private block must still succeed without faulting.
        unsafe { (prefix as *mut u8).write(99) };

        // SAFETY: clean up our view and prefix reservation.
        unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: view as *mut _,
            });
            VirtualFree(prefix as *mut _, 0, MEM_RELEASE);
        }
        title_memory_remapped(view, BLOCK);
    }

    /// A write or mapping that runs past the end of the address space (a
    /// title's `fread` with a huge count) covers up to the end; the sum used
    /// to overflow. Up there no test has guarded blocks.
    #[test]
    fn ranges_past_the_address_space_end_do_not_overflow() {
        let top = usize::MAX - 16;
        writing(top, PAGE);
        title_memory_remapped(top, usize::MAX);
    }

    /// `fread`'s size times count saturates to `usize::MAX`: announcing and
    /// landing that kernel read is one range, not a step per block of the
    /// whole address space, and landing it leaves nothing announced.
    #[test]
    fn a_saturated_kernel_read_is_announced_in_one_step() {
        // Kernel half of the address space: no user allocation of another
        // test running at the same time can fall in the announced range.
        let at = 0xFFFF_8000_0000_0000;
        let range = (at / BLOCK, usize::MAX.div_ceil(BLOCK));
        kernel_will_write(at, usize::MAX);
        assert!(guards().announced(at / BLOCK + 12345));
        kernel_wrote(at, usize::MAX);
        assert!(!guards().announced.contains_key(&range));
    }

    /// A range announced again and again without landing (overlapped reads
    /// into one buffer) is one counted entry, and a remap over it drops it.
    #[test]
    fn repeated_announcements_are_counted_and_a_remap_drops_them() {
        let at = 0xFFFF_9000_0000_0000;
        let range = (at / BLOCK, (at + BLOCK).div_ceil(BLOCK));
        for _ in 0..1000 {
            kernel_will_write(at, BLOCK);
        }
        assert_eq!(guards().announced.get(&range), Some(&1000));
        kernel_wrote(at, BLOCK);
        assert_eq!(guards().announced.get(&range), Some(&999));
        title_memory_remapped(at, BLOCK);
        assert!(!guards().announced.contains_key(&range));
    }

    /// A kernel read announced with `kernel_will_write` survives a refresh
    /// on another thread before it lands; once `kernel_wrote` says it has,
    /// the next refresh guards its block again.
    #[test]
    fn an_announced_kernel_read_survives_a_refresh_before_it() {
        use std::io::{Read, Seek, Write};
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MapViewOfFile, UnmapViewOfFile,
        };
        // SAFETY: a pagefile section of two blocks, mapped read-write.
        let view = unsafe {
            let section = CreateFileMappingW(
                -1isize as _,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                (2 * BLOCK) as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, 2 * BLOCK).Value as usize
        };
        assert_ne!(view, 0);
        let end = view + 2 * BLOCK;
        let (first, initial) = writes(view, end, true).expect("new mapped view");
        assert_eq!(initial, [(view / PAGE, first), (view / PAGE + 1, first)]);
        let (next, clean) = writes(view + 8, end - 1, true).expect("same protected pages");
        assert!(next > first);
        assert_eq!(clean, initial);
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"from the disk").unwrap();
        let read = |file: &mut std::fs::File| {
            file.rewind().unwrap();
            // SAFETY: 13 bytes of our view, nothing else refers to them.
            let into = unsafe { std::slice::from_raw_parts_mut((view + 100) as *mut u8, 13) };
            file.read(into).is_ok_and(|n| n == 13)
        };
        kernel_will_write(view + 100, 13); // the title's thread, in `ReadFile`
        let _ = guard(view, view + 2 * BLOCK); // a refresh on another thread
        assert!(
            read(&mut file),
            "the refresh guarded the block again under the read"
        );
        kernel_wrote(view + 100, 13);
        let (changed, pages) = writes(view, end, true).expect("kernel write visible");
        assert_eq!(pages, [(view / PAGE, changed), (view / PAGE + 1, first)]);
        assert_eq!(writes(view, end, true).unwrap().1, pages);
        assert!(!read(&mut file), "guarded again once the read is done");
        // SAFETY: our view.
        unsafe {
            UnmapViewOfFile(
                windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: view as *mut _,
                },
            )
        };
        title_memory_remapped(view, 2 * BLOCK);
    }
}

#[cfg(test)]
mod mapping_tests {
    use super::{PAGE, by_range, title_memory_reading, title_memory_remapping};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    /// Addresses only these tests hold (nothing is read there).
    const HERE: usize = 0x7e00_0000_0000;

    /// A change of mapping waits for a read in place of its bytes that is
    /// under way, and such a read waits for the change.
    #[test]
    fn reads_in_place_and_remappings_wait_for_each_other() {
        // A reader holds; the change does not begin until it lets go.
        let changed = Arc::new(AtomicBool::new(false));
        let reading = title_memory_reading(HERE + PAGE / 2, PAGE);
        let (began, wait) = mpsc::channel();
        let change = {
            let changed = changed.clone();
            std::thread::spawn(move || {
                began.send(()).unwrap();
                let _changing = title_memory_remapping(HERE + PAGE, PAGE);
                changed.store(true, Ordering::SeqCst);
            })
        };
        wait.recv().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(!changed.load(Ordering::SeqCst));
        drop(reading);
        change.join().unwrap();
        assert!(changed.load(Ordering::SeqCst));
        // A change holds; the read does not begin until it is done.
        let read = Arc::new(AtomicBool::new(false));
        let changing = title_memory_remapping(HERE, 2 * PAGE);
        let (began, wait) = mpsc::channel();
        let reader = {
            let read = read.clone();
            std::thread::spawn(move || {
                began.send(()).unwrap();
                let _reading = title_memory_reading(HERE + PAGE + 16, 16);
                read.store(true, Ordering::SeqCst);
            })
        };
        wait.recv().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(!read.load(Ordering::SeqCst));
        drop(changing);
        reader.join().unwrap();
        assert!(read.load(Ordering::SeqCst));
    }

    /// Reads and changes whose bytes do not meet do not wait for each other,
    /// and a change waiting for a read keeps new reads of its bytes out (but
    /// not of others).
    #[test]
    fn reads_and_remappings_of_other_bytes_go_on() {
        if !by_range() {
            return; // VICTORY_MAPPING_RANGES=0: one lock over all of it
        }
        let at = HERE + 16 * PAGE;
        let reading = title_memory_reading(at, PAGE);
        // Another page changes while the read is under way, and another
        // read goes on while that change is.
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let changing = title_memory_remapping(at + PAGE, PAGE);
            let (read, was_read) = mpsc::channel();
            std::thread::spawn(move || {
                let _reading = title_memory_reading(at + 2 * PAGE, PAGE);
                read.send(()).unwrap();
            });
            let other = was_read.recv_timeout(Duration::from_secs(10)).is_ok();
            drop(changing);
            done.send(other).unwrap();
        });
        assert_eq!(
            finished.recv_timeout(Duration::from_secs(10)),
            Ok(true),
            "kept apart though their bytes do not meet"
        );
        // A change of the read's page waits, and keeps a new read of its
        // bytes out meanwhile; a read elsewhere is not kept out.
        let changed = Arc::new(AtomicBool::new(false));
        let change = {
            let changed = changed.clone();
            std::thread::spawn(move || {
                let _changing = title_memory_remapping(at, PAGE);
                changed.store(true, Ordering::SeqCst);
            })
        };
        while !super::spans().changes.iter().any(|c| c.1 == at) {
            std::thread::yield_now();
        }
        assert!(
            super::title_memory_reading_now(at + 8, 8).is_none(),
            "a read began over a change that waits"
        );
        assert!(super::title_memory_reading_now(at + 3 * PAGE, 8).is_some());
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !changed.load(Ordering::SeqCst),
            "the mapping changed under a read in place"
        );
        drop(reading);
        change.join().unwrap();
        assert!(changed.load(Ordering::SeqCst));
    }
}
