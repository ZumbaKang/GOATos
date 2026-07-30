//! Task State Segments, and the private stack the double-fault handler runs on.
//!
//! On 32-bit x86 there is no equivalent of x86-64's Interrupt Stack Table: an
//! interrupt taken while already in ring 0 keeps using the stack it
//! interrupted, and the TSS's `ss0`/`esp0` are consulted *only* on a
//! privilege-level change. So the one and only way to guarantee that a
//! handler starts on a known-good stack is a **task gate**: the CPU performs a
//! full hardware task switch, saving the interrupted registers into the TSS
//! named by the task register and loading a complete new register set - `esp`
//! included - from the TSS named by the gate.
//!
//! That is why this module defines *two* TSSes:
//!
//! - [`main_task`] - where the CPU dumps the interrupted kernel's registers.
//!   A task switch cannot happen without somewhere to save the outgoing state,
//!   so this exists purely to be written to (and, later, to hold `ss0`/`esp0`
//!   once there is a ring 3 to come back from). Its saved `eip`/`esp` are what
//!   makes the double-fault report useful.
//! - [`double_fault_task`] - a fully-populated register image pointing at the
//!   double-fault handler, with its own stack, so the handler runs even if the
//!   interrupted `esp` is garbage.
//!
//! Neither TSS is ever *scheduled*: GOATos does not use hardware task
//! switching for multitasking (nobody has since the 486 - it's slow and
//! 64-bit mode dropped it). This is a task switch used strictly as "the
//! architectural way to change stacks in a fault handler".

use core::cell::UnsafeCell;
use core::mem::size_of;

use crate::gdt;

/// A 32-bit Task State Segment, in hardware layout.
///
/// Every field except the segment selectors, `eip`, `esp` and `iomap_base` is
/// here because the CPU insists on saving or restoring it during a task
/// switch, not because this kernel has any use for it.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct TaskStateSegment {
    /// Selector of the TSS that switched *to* this task ("back link"). Set by
    /// the CPU on a nested task switch; reading it back proves which task
    /// faulted.
    previous_task_link: u16,
    _reserved0: u16,
    /// Ring-0 stack, used only when entering ring 0 from a less privileged
    /// ring. Unused today (there is no ring 3 yet) - and note that it would
    /// *not* help here: a fault taken in ring 0 never switches stacks.
    esp0: u32,
    ss0: u16,
    _reserved1: u16,
    esp1: u32,
    ss1: u16,
    _reserved2: u16,
    esp2: u32,
    ss2: u16,
    _reserved3: u16,
    /// Page-directory base. Loaded into `CR3` on a task switch. Left at zero
    /// until [`set_page_directory`] runs (paging setup, roadmap 2.3); after
    /// that both TSSes must carry the kernel's real directory, or a
    /// double-fault task switch would load a page directory of zeroes.
    cr3: u32,
    eip: u32,
    eflags: u32,
    eax: u32,
    ecx: u32,
    edx: u32,
    ebx: u32,
    esp: u32,
    ebp: u32,
    esi: u32,
    edi: u32,
    es: u16,
    _reserved4: u16,
    cs: u16,
    _reserved5: u16,
    ss: u16,
    _reserved6: u16,
    ds: u16,
    _reserved7: u16,
    fs: u16,
    _reserved8: u16,
    gs: u16,
    _reserved9: u16,
    ldt_selector: u16,
    _reserved10: u16,
    /// Bit 0 is the debug-trap flag: set, it raises a debug exception on every
    /// switch into this task. Left clear.
    debug_trap: u16,
    /// Offset of the I/O permission bitmap. Pointing it at (or past) the end
    /// of the segment means "no bitmap", which is what a kernel with no ring-3
    /// code wants.
    iomap_base: u16,
}

impl TaskStateSegment {
    const EMPTY: TaskStateSegment = TaskStateSegment {
        previous_task_link: 0,
        _reserved0: 0,
        esp0: 0,
        ss0: 0,
        _reserved1: 0,
        esp1: 0,
        ss1: 0,
        _reserved2: 0,
        esp2: 0,
        ss2: 0,
        _reserved3: 0,
        cr3: 0,
        eip: 0,
        eflags: 0,
        eax: 0,
        ecx: 0,
        edx: 0,
        ebx: 0,
        esp: 0,
        ebp: 0,
        esi: 0,
        edi: 0,
        es: 0,
        _reserved4: 0,
        cs: 0,
        _reserved5: 0,
        ss: 0,
        _reserved6: 0,
        ds: 0,
        _reserved7: 0,
        fs: 0,
        _reserved8: 0,
        gs: 0,
        _reserved9: 0,
        ldt_selector: 0,
        _reserved10: 0,
        debug_trap: 0,
        iomap_base: size_of::<TaskStateSegment>() as u16,
    };
}

// The hardware layout is fixed at 104 bytes; a mismatch would mean a field is
// missing or mis-sized, and the CPU would save registers over each other.
const _: () = assert!(size_of::<TaskStateSegment>() == 104);

/// Both TSSes are written by the CPU (the outgoing register dump, and the busy
/// bits' companion back link), so they cannot live in `.rodata` the way the
/// GDT's descriptors used to.
struct TssCell(UnsafeCell<TaskStateSegment>);

// SAFETY: GOATos is single-CPU and single-threaded. The only concurrent writer
// is the CPU itself during a task switch, and a task switch only happens on a
// double fault - after which the handler halts, so nothing races with the
// read-back in `interrupted_state`.
unsafe impl Sync for TssCell {}

static MAIN_TSS: TssCell = TssCell(UnsafeCell::new(TaskStateSegment::EMPTY));
static DOUBLE_FAULT_TSS: TssCell = TssCell(UnsafeCell::new(TaskStateSegment::EMPTY));

// Laid out in `entry.s` immediately below the unmapped stack guard page, so a
// runaway kernel-stack overflow faults before it can reach this region. Only
// the addresses matter here - the bytes themselves are never read as data.
extern "C" {
    static double_fault_stack_bottom: u8;
    static double_fault_stack_top: u8;
}

/// Where a TSS lives and how big it is - what [`crate::gdt`] needs to build a
/// descriptor for it.
pub struct Location {
    pub base: u32,
    /// Last valid byte's offset, i.e. size - 1, which is what a descriptor's
    /// limit field holds. A TSS descriptor's limit must cover at least the
    /// 104-byte hardware layout or the CPU raises an invalid-TSS fault.
    pub limit: u32,
}

/// The state the CPU saved for the task that was interrupted, read back out of
/// the main TSS after a switch.
pub struct InterruptedState {
    pub eip: u32,
    pub cs: u16,
    pub eflags: u32,
    pub esp: u32,
    pub ss: u16,
    pub ebp: u32,
    /// Which TSS the CPU switched away from, as recorded in the *incoming*
    /// task's back link. Should be the main TSS's selector.
    pub previous_task_link: u16,
}

/// Fills in both TSSes. Must run before [`crate::gdt::init`], which builds
/// descriptors for them and loads the task register.
///
/// `double_fault_entry` is where the CPU starts executing after the switch. It
/// must never return: there is no task to return *to* (the interrupted one is
/// by definition broken), and a `ret` would pop whatever the CPU left on the
/// new stack.
pub fn init(double_fault_entry: extern "C" fn() -> !) {
    // SAFETY: single-threaded, interrupts masked, and no task switch can have
    // happened yet - the task register isn't even loaded.
    let main = unsafe { &mut *MAIN_TSS.0.get() };
    let double_fault = unsafe { &mut *DOUBLE_FAULT_TSS.0.get() };

    // Write both out in full rather than relying on the statics' zero
    // initialisers: an all-zero static lands in `.bss`, which carries no file
    // data for `objcopy -O binary` to emit, so the flat-binary loader in
    // `boot/boot.asm` never writes those bytes (see `idt::init`).
    *main = TaskStateSegment::EMPTY;
    *double_fault = TaskStateSegment::EMPTY;

    double_fault.eip = double_fault_entry as usize as u32;
    // Grows down from the top of the private stack.
    double_fault.esp = stack_top();
    double_fault.ebp = double_fault.esp;
    // `esp0`/`ss0` are irrelevant while everything is ring 0, but a TSS whose
    // ring-0 stack is null is a trap waiting for the first ring-3 code, so
    // point them at the same private stack.
    double_fault.esp0 = double_fault.esp;
    double_fault.ss0 = gdt::KERNEL_DATA_SELECTOR;
    double_fault.cs = gdt::KERNEL_CODE_SELECTOR;
    double_fault.ss = gdt::KERNEL_DATA_SELECTOR;
    double_fault.ds = gdt::KERNEL_DATA_SELECTOR;
    double_fault.es = gdt::KERNEL_DATA_SELECTOR;
    double_fault.fs = gdt::KERNEL_DATA_SELECTOR;
    double_fault.gs = gdt::KERNEL_DATA_SELECTOR;
    // Bit 1 reads as 1 on every x86; IF stays clear so the handler can't be
    // interrupted while reporting.
    double_fault.eflags = 0x0000_0002;
}

/// Records the active page directory in both TSSes.
///
/// A hardware task switch loads `CR3` from the incoming TSS, so once paging
/// is on this has to match the directory [`crate::memory::paging`] installed -
/// otherwise the double-fault handler would land on a directory of zeroes and
/// triple-fault before printing anything. Safe to call with paging still off:
/// the fields are ignored until a task switch actually happens.
pub fn set_page_directory(cr3: u32) {
    // SAFETY: single-threaded, and no task switch is in flight (those only
    // happen on a double fault, which does not return). Writing both keeps
    // the interrupted-state save and the handler's own image in agreement.
    let main = unsafe { &mut *MAIN_TSS.0.get() };
    let double_fault = unsafe { &mut *DOUBLE_FAULT_TSS.0.get() };
    main.cr3 = cr3;
    double_fault.cr3 = cr3;
}

/// Bottom and top (exclusive) of the double-fault handler's private stack.
/// Printing this next to the `esp` the handler actually runs on is what proves
/// the stack switch happened.
pub fn double_fault_stack_range() -> (u32, u32) {
    // Only the addresses of these assembly symbols are taken - they name
    // objects that exist for the life of the kernel.
    let bottom = core::ptr::addr_of!(double_fault_stack_bottom) as u32;
    let top = core::ptr::addr_of!(double_fault_stack_top) as u32;
    (bottom, top)
}

fn stack_top() -> u32 {
    double_fault_stack_range().1
}

/// Where the interrupted kernel's registers get saved.
pub fn main_task() -> Location {
    Location {
        base: MAIN_TSS.0.get() as u32,
        limit: (size_of::<TaskStateSegment>() - 1) as u32,
    }
}

/// The register image the CPU loads when it takes the double-fault task gate.
pub fn double_fault_task() -> Location {
    Location {
        base: DOUBLE_FAULT_TSS.0.get() as u32,
        limit: (size_of::<TaskStateSegment>() - 1) as u32,
    }
}

/// Reads back what the CPU saved about the interrupted task.
///
/// Only meaningful from inside a handler entered through a task gate; before
/// the first task switch the main TSS is all zeroes.
pub fn interrupted_state() -> InterruptedState {
    // SAFETY: single-threaded; the only other writer is the CPU during the
    // task switch that got us here, which is complete by definition.
    let main = unsafe { &*MAIN_TSS.0.get() };
    let double_fault = unsafe { &*DOUBLE_FAULT_TSS.0.get() };
    InterruptedState {
        eip: main.eip,
        cs: main.cs,
        eflags: main.eflags,
        esp: main.esp,
        ss: main.ss,
        ebp: main.ebp,
        previous_task_link: double_fault.previous_task_link,
    }
}
