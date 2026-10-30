/* nRF54LM20A application core, Secure mode, without a bootloader.
 * Reserve the KMU push area at the start of SRAM and the final 1 KiB.
 * Reference: Zephyr dts/vendor/nordic/nrf54lm20_a_b.dtsi.
 */
MEMORY
{
    FLASH : ORIGIN = 0x00000000, LENGTH = 2036K
    RAM   : ORIGIN = 0x20000060, LENGTH = 511K - 0x60
}
