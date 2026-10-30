use embassy_nrf::pac::radio::vals::Txpower;

#[derive(Clone, Copy, Default)]
pub(crate) struct RadioEvents {
    pub crc_ok: bool,
    pub disabled: bool,
    pub txready: bool,
    pub rxready: bool,
}

/// Required hardware API. Every chip backend must implement all methods.
pub(crate) trait RadioDriver {
    fn init(&mut self);
    /// # Safety
    /// The DMA buffer must remain valid and exclusively accessible to the radio
    /// until the transfer completes or the radio has been disabled.
    unsafe fn set_packetptr(&mut self, packet: *mut u8);
    fn set_txaddress(&mut self, pipe: u8);
    fn set_rxaddress(&mut self, pipe: u8);
    fn set_channel(&mut self, channel: u8);
    fn take_interrupt_events(&mut self) -> RadioEvents;
    fn irq_set_disabled(&mut self, set: bool);
    fn irq_set_crcok(&mut self, set: bool);
    fn get_rssi_sample(&mut self) -> u8;
    fn get_rxcrc(&mut self) -> u32;
    fn set_tx_power(&mut self, tx_power: Txpower);
    fn disable(&mut self);
}
