ENTRY(_start)
SECTIONS {
  . = 0x0;
  .text : { *(.text*) }
  .rodata : { *(.rodata*) }
  .data : { *(.data*) }
  .bss : { *(.bss*) }
  /DISCARD/ : { *(.riscv.attributes) *(.comment) *(.eh_frame) }
}
