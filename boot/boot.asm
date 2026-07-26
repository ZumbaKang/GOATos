; GOATos stage-1 bootloader.
;
; This is a hand-written, from-scratch MBR boot sector: no GRUB, no
; Multiboot, nothing but what the BIOS gives us for free. The BIOS loads
; this single 512-byte sector to 0x7C00 and jumps to it in 16-bit real mode.
; From here we:
;   1. load the kernel (following sectors on the same disk) into memory
;   2. enable the A20 line, so we can address memory past 1MiB
;   3. set up a flat GDT and switch the CPU into 32-bit protected mode
;   4. jump into the kernel's entry point
;
; KERNEL_SECTORS is supplied at assemble time (`nasm -D KERNEL_SECTORS=N`)
; by the Makefile, computed from the kernel binary's actual size, so this
; file never needs to be hand-edited as the kernel grows.

[org 0x7c00]
[bits 16]

KERNEL_LOAD_SEGMENT equ 0x1000   ; kernel is loaded at physical 0x10000
KERNEL_LOAD_ADDR    equ 0x10000

start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7c00

    mov [boot_drive], dl

    mov si, msg_loading
    call print_string

    call get_disk_geometry
    call load_kernel
    call enable_a20
    lgdt [gdt_descriptor]

    mov eax, cr0
    or eax, 1
    mov cr0, eax

    jmp CODE_SEG:protected_mode_entry

; ---------------------------------------------------------------------------
; Real-mode helpers
; ---------------------------------------------------------------------------

print_string:
    pusha
    mov ah, 0x0e
.next_char:
    lodsb
    cmp al, 0
    je .done
    int 0x10
    jmp .next_char
.done:
    popa
    ret

; Asks the BIOS for the boot drive's CHS geometry (INT 13h/AH=08h). We use
; classic CHS reads rather than the newer LBA "extended read" service
; (AH=42h) because CHS is the oldest, most universally-implemented BIOS disk
; service - notably more reliably supported across simplified/browser x86
; emulators (e.g. v86) than the extended read path.
get_disk_geometry:
    push es
    xor di, di
    mov es, di            ; some BIOSes misbehave unless ES:DI = 0:0 here
    mov ah, 0x08
    mov dl, [boot_drive]
    int 0x13
    pop es
    jc disk_error

    and cl, 0x3f         ; bits 0-5 of CL: sectors per track
    mov [sectors_per_track], cl
    movzx ax, dh         ; DH: max head number (0-based) -> head count
    inc ax
    mov [heads_per_cylinder], al

    ; sectors_per_cylinder = heads_per_cylinder * sectors_per_track
    xor ax, ax
    mov al, [heads_per_cylinder]
    mov bl, [sectors_per_track]
    mul bl
    mov [sectors_per_cylinder], ax
    ret

; Loads KERNEL_SECTORS sectors (starting right after this boot sector, i.e.
; LBA 1) from the boot drive into memory starting at
; KERNEL_LOAD_SEGMENT:0x0000, one sector at a time via INT 13h/AH=02h - the
; classic CHS-based BIOS disk read.
load_kernel:
    mov word [sectors_remaining], KERNEL_SECTORS
    mov dword [current_lba], 1        ; sector 0 is this boot sector
    mov word [current_segment], KERNEL_LOAD_SEGMENT

.read_loop:
    cmp word [sectors_remaining], 0
    je .done

    call read_one_sector_chs

    dec word [sectors_remaining]
    inc dword [current_lba]
    add word [current_segment], 32    ; 512 bytes / 16 bytes-per-paragraph

    jmp .read_loop

.done:
    ret

; Reads the single sector at [current_lba] into [current_segment]:0x0000.
read_one_sector_chs:
    ; eax = current_lba
    mov eax, [current_lba]
    xor edx, edx
    movzx ecx, word [sectors_per_cylinder]
    div ecx                            ; eax = cylinder, edx = (lba mod sectors_per_cylinder)
    mov [tmp_cylinder], ax

    mov eax, edx                       ; eax = lba mod sectors_per_cylinder
    xor edx, edx
    movzx ecx, byte [sectors_per_track]
    div ecx                            ; eax = head, edx = sector - 1
    mov [tmp_head], al
    inc dl
    mov [tmp_sector], dl

    mov ax, [tmp_cylinder]
    mov ch, al                         ; cylinder bits 0-7
    mov cl, ah                         ; cylinder bits 8-9 (top bits of ax)
    shl cl, 6
    mov dl, [tmp_sector]
    or cl, dl                          ; CL = cylinder[9:8]<<6 | sector[5:0]

    mov dh, [tmp_head]
    mov dl, [boot_drive]

    mov ax, [current_segment]
    mov es, ax
    xor bx, bx

    mov ax, 0x0201                     ; AH=02h (read), AL=1 sector
    int 0x13
    jc disk_error
    ret

disk_error:
    mov si, msg_disk_error
    call print_string
.hang:
    hlt
    jmp .hang

sectors_per_track: db 0
heads_per_cylinder: db 0
sectors_per_cylinder: dw 0
tmp_cylinder: dw 0
tmp_head: db 0
tmp_sector: db 0

sectors_remaining: dw 0
current_lba: dd 0
current_segment: dw 0

; Fast A20 gate. Works on essentially every machine QEMU/v86 emulate.
enable_a20:
    in al, 0x92
    or al, 2
    out 0x92, al
    ret

boot_drive: db 0

msg_loading: db "GOATos: loading kernel...", 13, 10, 0
msg_disk_error: db "GOATos: disk read error!", 13, 10, 0

; ---------------------------------------------------------------------------
; GDT: flat 4GiB code + data segments, so protected mode addressing is just
; plain physical addresses.
; ---------------------------------------------------------------------------
align 8
gdt_start:
    dq 0                         ; null descriptor

gdt_code:
    dw 0xffff                    ; limit (low)
    dw 0x0000                    ; base (low)
    db 0x00                      ; base (mid)
    db 10011010b                 ; present, ring 0, code, executable, readable
    db 11001111b                 ; 4KiB granularity, 32-bit, limit (high)
    db 0x00                      ; base (high)

gdt_data:
    dw 0xffff
    dw 0x0000
    db 0x00
    db 10010010b                 ; present, ring 0, data, writable
    db 11001111b
    db 0x00
gdt_end:

gdt_descriptor:
    dw gdt_end - gdt_start - 1
    dd gdt_start

CODE_SEG equ gdt_code - gdt_start
DATA_SEG equ gdt_data - gdt_start

; ---------------------------------------------------------------------------
; Protected mode entry
; ---------------------------------------------------------------------------
[bits 32]
protected_mode_entry:
    mov ax, DATA_SEG
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax

    jmp KERNEL_LOAD_ADDR

; Pad the boot sector to exactly 512 bytes and add the mandatory BIOS
; boot signature at the very end.
times 510-($-$$) db 0
dw 0xaa55
