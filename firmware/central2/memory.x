/* nice!nano, Adafruit UF2 bootloader 0.6.0, SoftDevice: not found.
 * MBR: 0x00000..0x01000; application: 0x01000..0xED000.
 * Keep 0xED000..0xF4000 for app data, and 0xF4000..0x100000
 * for the bootloader, its configuration, MBR parameters and settings.
 * Reserve the first 8 RAM bytes for MBR interrupt forwarding.
 */
MEMORY
{
    FLASH : ORIGIN = 0x00001000, LENGTH = 0xEC000
    RAM   : ORIGIN = 0x20000008, LENGTH = 256K - 8
}
