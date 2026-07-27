/* Cooperative context switch (roadmap 4.3 / 4.4).
 *
 * void context_switch(uint32_t *old_esp, uint32_t *new_esp);
 *
 * Saves the callee-saved registers (System V i386: ebx, esi, edi, ebp) plus
 * the return address already on the stack onto the current stack, writes the
 * resulting %esp into *old_esp, loads %esp from *new_esp, restores the
 * matching registers, and `ret`s into the other task.
 *
 * A freshly spawned task's stack is primed with four zeroed callee-saved
 * slots and a return address that lands in `task_trampoline`, so the first
 * switch into it looks just like resuming a task that previously yielded.
 */

.section .text
.global context_switch
.type context_switch, @function
context_switch:
    /* 4(%esp) = old_esp, 8(%esp) = new_esp — grab both before we push. */
    movl 4(%esp), %eax
    movl 8(%esp), %edx

    pushl %ebp
    pushl %edi
    pushl %esi
    pushl %ebx

    /* Park this task's stack pointer where the scheduler can find it. */
    movl %esp, (%eax)

    /* Resume the other task. */
    movl (%edx), %esp

    popl %ebx
    popl %esi
    popl %edi
    popl %ebp

    ret
.size context_switch, .-context_switch

/* Landing pad for a freshly spawned task. `spawn` plants the real entry
 * pointer in the word above this return address; the `ret` from
 * `context_switch` arrives here with %esp pointing at that word. */
.global task_trampoline
.type task_trampoline, @function
task_trampoline:
    popl %eax
    /* SysV: at `call`, (%esp + 4) must be 16-byte aligned. After the pop,
     * %esp is 16-aligned (spawn aligned the high end of the stack); open a
     * 12-byte gap so the call's pushed return address lands on alignment. */
    subl $12, %esp
    call *%eax
    /* `fn() -> !` must not return; park if it ever does. */
1:
    hlt
    jmp 1b
.size task_trampoline, .-task_trampoline
