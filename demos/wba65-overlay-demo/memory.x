MEMORY
{
  /* STM32WBA65RI Application Partition (offset 64KB for bootloader) */
  FLASH (rx) : ORIGIN = 0x08010000, LENGTH = 1984K
  RAM   (rwx): ORIGIN = 0x20000000, LENGTH = 512K
}

_stack_start = ORIGIN(RAM) + LENGTH(RAM);
