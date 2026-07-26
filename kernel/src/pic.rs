//! 8259A Programmable Interrupt Controller pair (master + slave).
//!
//! Every PC starts with the two cascaded 8259s the BIOS left configured for
//! real mode, where they raise vectors 8-15 (master) and 0x70-0x77 (slave).
//! In protected mode the first of those ranges collides head-on with the CPU's
//! own exception vectors: a timer IRQ would arrive as vector 8, which is the
//! double fault, and a keyboard IRQ as vector 9 - so the kernel could not tell
//! "the timer ticked" from "the machine is on fire". Worse, the exception
//! vectors that push an error code and the IRQs that don't would share
//! handlers, corrupting the stack at `iret`.
//!
//! So the controllers get reprogrammed to put IRQ0-15 on vectors
//! [`IRQ_VECTOR_BASE`]`..=`[`IRQ_VECTOR_LAST`] (32-47), immediately above the
//! 32 vectors Intel reserves for exceptions, and then every IRQ line is
//! masked. Masking matters because remapping is not the same as being ready:
//! the PIT is already ticking at boot, so the moment interrupts are enabled an
//! unmasked line would dispatch through the IDT to a vector nothing has
//! registered yet. Leaving the mask at 0xff means each driver unmasks its own
//! line when it actually has a handler.
//!
//! Interrupts *are* enabled now (see [`crate::interrupts`]), so this module
//! also provides the other half of servicing an IRQ: [`end_of_interrupt`],
//! which tells the controllers the line has been dealt with and they may raise
//! it again.

use core::arch::asm;
use spin::Mutex;

/// Vector that IRQ0 lands on after the remap. 32 is the first vector Intel
/// does not reserve for exceptions (0-31), which is exactly why it is the
/// conventional choice.
pub const IRQ_VECTOR_BASE: u8 = 32;
/// Number of IRQ lines the cascaded pair provides: 8 per controller, minus
/// the master's IRQ2 which the slave occupies - but the *vector* range still
/// covers all 16 slots.
pub const IRQ_COUNT: u8 = 16;
/// Vector that IRQ15, the last one, lands on.
pub const IRQ_VECTOR_LAST: u8 = IRQ_VECTOR_BASE + IRQ_COUNT - 1;

const _: () = assert!(
    IRQ_VECTOR_BASE >= 32,
    "IRQ vectors must not overlap the CPU's exception vectors (0-31)"
);
// An 8259's vector base is the top 5 bits of ICW2; the low 3 bits are the IRQ
// number within the controller, so the base has to be a multiple of 8.
const _: () = assert!(
    IRQ_VECTOR_BASE.is_multiple_of(8),
    "ICW2 only encodes bits 3..8"
);

/// Command port of the master PIC (and, with the data port, its ICW1/OCW2/OCW3
/// register).
const MASTER_COMMAND: u16 = 0x20;
const MASTER_DATA: u16 = 0x21;
const SLAVE_COMMAND: u16 = 0xa0;
const SLAVE_DATA: u16 = 0xa1;

/// ICW1: begin initialization, and expect an ICW4 as the fourth word. Bit 4
/// is what marks the byte as ICW1 at all; bit 0 requests ICW4.
const ICW1_INIT: u8 = 0x11;
/// ICW3 for the master: a bitmask of which of *its* lines a slave hangs off.
/// Bit 2 - the traditional cascade on IRQ2.
const ICW3_MASTER_CASCADE: u8 = 1 << 2;
/// ICW3 for the slave: not a mask but an identity - which master line it is
/// wired to, as a number.
const ICW3_SLAVE_IDENTITY: u8 = 2;
/// ICW4: 8086/8088 mode. Without it the controller stays in MCS-80/85 mode
/// and delivers a call address rather than a vector number.
const ICW4_8086_MODE: u8 = 0x01;

/// OCW2: non-specific end of interrupt - clears the highest-priority bit that
/// is set in the in-service register, which is the IRQ currently being
/// handled. Until it is written, that line and every lower-priority one stay
/// blocked.
const OCW2_END_OF_INTERRUPT: u8 = 0x20;
/// OCW3: make the next read of the command port return the in-service
/// register, i.e. which IRQs the controller thinks are being handled right
/// now. Used to tell a real IRQ from a spurious one.
const OCW3_READ_IN_SERVICE: u8 = 0x0b;

/// The lowest-priority line on each controller, and so the one a spurious
/// interrupt is reported on: when a line asserts just long enough to make the
/// 8259 raise INTR and then drops before the CPU acknowledges it, the
/// controller has no vector to supply and substitutes its IRQ7 (master) or
/// IRQ15 (slave).
const SPURIOUS_LINE: u8 = 7;

/// Mask value that disables every line on a controller.
const MASK_ALL: u8 = 0xff;

/// Port 0x80 is the POST diagnostic register: writing it has no effect on any
/// machine or emulator this kernel targets, which makes it the standard way to
/// spend one bus cycle. The 8259 needs a short settling time between
/// initialization words, and it is old enough that "the next `out` is soon
/// enough" is not guaranteed on fast CPUs.
const IO_DELAY_PORT: u16 = 0x80;

/// One of the two controllers. `vector_offset` is remembered rather than read
/// back because an 8259 offers no way to read ICW2 - see [`state`].
struct Pic {
    command: u16,
    data: u16,
    vector_offset: u8,
}

impl Pic {
    /// Runs the four-word initialization sequence, which also resets the
    /// controller's mask register to "everything enabled".
    ///
    /// # Safety
    /// Writes directly to the 8259's I/O ports, so it must not race with
    /// anything else touching them - in practice: call it with interrupts
    /// masked, from [`init`], once.
    unsafe fn initialize(&self, cascade_word: u8) {
        unsafe {
            outb(self.command, ICW1_INIT);
            io_delay();
            outb(self.data, self.vector_offset);
            io_delay();
            outb(self.data, cascade_word);
            io_delay();
            outb(self.data, ICW4_8086_MODE);
            io_delay();
        }
    }

    /// Writes the interrupt mask register: a set bit disables that line.
    ///
    /// # Safety
    /// As for [`Pic::initialize`]. Clearing a bit here makes the corresponding
    /// IRQ deliverable, so the caller must have registered a handler for its
    /// vector first.
    unsafe fn set_mask(&self, mask: u8) {
        unsafe { outb(self.data, mask) };
    }

    /// Reads the interrupt mask register back out of the hardware.
    fn mask(&self) -> u8 {
        // SAFETY: a read of the data port with no OCW3 in flight returns the
        // IMR and has no side effects on the controller.
        unsafe { inb(self.data) }
    }
}

/// Whether `vector` is one of the ones [`init`] pointed the IRQ lines at, and
/// if so which line raised it.
pub fn irq_of_vector(vector: u8) -> Option<u8> {
    if (IRQ_VECTOR_BASE..=IRQ_VECTOR_LAST).contains(&vector) {
        Some(vector - IRQ_VECTOR_BASE)
    } else {
        None
    }
}

/// What [`end_of_interrupt`] found the IRQ to be.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Acknowledged {
    /// A real IRQ, now acknowledged: the controllers may raise it again.
    Serviced,
    /// A spurious IRQ - the controller raised INTR but had no line still
    /// asserted to attribute it to, so it substituted its lowest-priority
    /// vector. There is nothing in service to acknowledge, and sending an EOI
    /// anyway would clear a *different*, genuinely in-service IRQ.
    Spurious,
}

/// Tells the controllers that the IRQ which raised `irq` has been serviced, so
/// that line (and every lower-priority one) can be raised again.
///
/// Returns [`Acknowledged::Spurious`] for a phantom interrupt, which must not
/// be acknowledged as if it were real - see [`Acknowledged`]. For a spurious
/// *slave* interrupt the master still gets an EOI, because from its side the
/// cascade line really was in service.
///
/// Deliberately takes no lock, unlike the rest of this module: it runs in
/// interrupt context, where blocking on a lock the interrupted code happens to
/// hold would hang the kernel outright. It doesn't need one either - the ports
/// and the bytes written are fixed constants, so there is no shared state here
/// to protect.
pub fn end_of_interrupt(irq: u8) -> Acknowledged {
    let from_slave = irq >= 8;
    let line = irq % 8;

    // Only the lowest-priority line of a controller can be spurious, so this
    // is the only case worth an extra pair of port accesses.
    if line == SPURIOUS_LINE {
        let command = if from_slave {
            SLAVE_COMMAND
        } else {
            MASTER_COMMAND
        };
        // SAFETY: OCW3 selects which register the *next* read of the command
        // port returns; neither the write nor the read changes what the
        // controller is doing.
        let in_service = unsafe {
            outb(command, OCW3_READ_IN_SERVICE);
            inb(command) & (1 << line) != 0
        };
        if !in_service {
            if from_slave {
                // SAFETY: as below - the master's cascade line is genuinely in
                // service even when the slave's interrupt turns out not to be.
                unsafe { outb(MASTER_COMMAND, OCW2_END_OF_INTERRUPT) };
            }
            return Acknowledged::Spurious;
        }
    }

    // SAFETY: OCW2 to a command port only clears an in-service bit, and the
    // slave has to be cleared before the master, or the master unblocks the
    // cascade line while the slave still thinks it is being serviced.
    unsafe {
        if from_slave {
            outb(SLAVE_COMMAND, OCW2_END_OF_INTERRUPT);
        }
        outb(MASTER_COMMAND, OCW2_END_OF_INTERRUPT);
    }
    Acknowledged::Serviced
}

struct ChainedPics {
    master: Pic,
    slave: Pic,
}

static PICS: Mutex<ChainedPics> = Mutex::new(ChainedPics {
    master: Pic {
        command: MASTER_COMMAND,
        data: MASTER_DATA,
        vector_offset: IRQ_VECTOR_BASE,
    },
    slave: Pic {
        command: SLAVE_COMMAND,
        data: SLAVE_DATA,
        vector_offset: IRQ_VECTOR_BASE + 8,
    },
});

/// What the controllers are set to: the vector range they were programmed
/// with, and the masks read back from them.
pub struct State {
    /// Vector IRQ0 was programmed to raise.
    pub vector_base: u8,
    /// Vector IRQ15 was programmed to raise.
    pub vector_last: u8,
    /// Master's interrupt mask register, as the hardware reports it. A set bit
    /// is a *disabled* line, so 0xff means "nothing can get through".
    pub master_mask: u8,
    /// Slave's interrupt mask register.
    pub slave_mask: u8,
}

impl State {
    /// Whether every IRQ line is currently disabled.
    pub fn all_masked(&self) -> bool {
        self.master_mask == MASK_ALL && self.slave_mask == MASK_ALL
    }
}

/// Remaps both controllers above the exception vectors and masks every IRQ
/// line.
///
/// Infallible by design, like the other drivers: the 8259 has no failure to
/// report (there is nothing to read back but the mask), and a kernel that
/// cannot set up its interrupt controller has no better move than to carry on
/// with interrupts still off, which is the state it was already in.
///
/// Safe to call before the IDT has any IRQ handlers - that is the intended
/// order, since remapping is what makes registering them meaningful - but it
/// must run before interrupts are enabled.
pub fn init() {
    let pics = PICS.lock();

    // SAFETY: single-threaded with interrupts masked (`cli` from `boot.asm` is
    // still in effect at this point in boot), so nothing else is talking to
    // these ports, and no IRQ can be delivered mid-sequence - an 8259 that is
    // halfway through its four initialization words would otherwise deliver a
    // garbage vector.
    unsafe {
        pics.master.initialize(ICW3_MASTER_CASCADE);
        pics.slave.initialize(ICW3_SLAVE_IDENTITY);

        // Initialization left both masks clear, i.e. every line enabled. Undo
        // that: the PIT is already ticking, and there is no handler for vector
        // 32 yet.
        pics.master.set_mask(MASK_ALL);
        pics.slave.set_mask(MASK_ALL);
    }
}

/// Reads back what [`init`] programmed.
///
/// The masks come from the hardware. The vector range cannot: ICW2 is
/// write-only, so the only honest source for it is the offset this driver
/// programmed. (An external check is still possible - QEMU's monitor prints
/// the controllers' `irq_base`, which is how the remap was verified.)
pub fn state() -> State {
    let pics = PICS.lock();
    State {
        vector_base: pics.master.vector_offset,
        vector_last: pics.slave.vector_offset + 7,
        master_mask: pics.master.mask(),
        slave_mask: pics.slave.mask(),
    }
}

/// Writes `value` to I/O port `port`.
///
/// # Safety
/// Port I/O talks straight to a device; the caller must know that `port`
/// belongs to the device it thinks it does and that `value` is meaningful to
/// it, since a wrong write can leave hardware in a state no later code can fix.
unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!(
            "outb %al, %dx",
            in("dx") port,
            in("al") value,
            options(att_syntax, nomem, nostack, preserves_flags),
        );
    }
}

/// Reads a byte from I/O port `port`.
///
/// # Safety
/// As for [`outb`]: reading some ports has side effects on the device (it can
/// acknowledge an event or advance an internal register), so the caller must
/// know what reading `port` does.
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!(
            "inb %dx, %al",
            in("dx") port,
            out("al") value,
            options(att_syntax, nomem, nostack, preserves_flags),
        );
    }
    value
}

/// Burns one I/O bus cycle, to give the 8259 time to digest the previous word.
fn io_delay() {
    // SAFETY: 0x80 is the POST code register - write-only, unused once the
    // BIOS has finished, and harmless to scribble on.
    unsafe { outb(IO_DELAY_PORT, 0) };
}
