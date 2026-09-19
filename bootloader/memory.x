MEMORY
{
  /* Bootloader resides in Bank 1 Sector 0..7 (64 KB) */
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 64K
  /* Bootloader working RAM */
  RAM   (rwx): ORIGIN = 0x20000000, LENGTH = 64K
}

_stack_start = ORIGIN(RAM) + LENGTH(RAM);
