/* Reference Linker Script for RAM Code Overlays on ARM Cortex-M (e.g. STM32WBA65RI) */

MEMORY
{
  /* Example allocation in SRAM1 (448 KB total @ 0x20000000) */
  /* Main RAM for app bss, data, heap, and stack */
  RAM (rwx) : ORIGIN = 0x20000000, LENGTH = 384K

  /* Dedicated 32 KB Overlay Slot A in SRAM */
  RAM_OVERLAY_A (rwx) : ORIGIN = 0x20060000, LENGTH = 32K

  /* Dedicated 32 KB Overlay Slot B in SRAM (for ping-pong prefetching) */
  RAM_OVERLAY_B (rwx) : ORIGIN = 0x20068000, LENGTH = 32K
}

SECTIONS
{
  /* Overlay Slot A reserved section */
  .ram_overlay_a (NOLOAD) : ALIGN(16)
  {
    __sram_overlay_a_start = .;
    . += LENGTH(RAM_OVERLAY_A);
    __sram_overlay_a_end = .;
  } > RAM_OVERLAY_A

  /* Overlay Slot B reserved section */
  .ram_overlay_b (NOLOAD) : ALIGN(16)
  {
    __sram_overlay_b_start = .;
    . += LENGTH(RAM_OVERLAY_B);
    __sram_overlay_b_end = .;
  } > RAM_OVERLAY_B
}
