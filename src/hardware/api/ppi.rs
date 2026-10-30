/// Required hardware API. Every chip backend must implement all methods.
pub(crate) trait PpiDriver {
    type Clock;
    type Egu;

    fn clear(&mut self);
    fn enable(&mut self);
    fn disable(&mut self);
    fn configure_central(&mut self, clock: &mut Self::Clock, egu: &mut Self::Egu);
    fn configure_sync(&mut self, clock: &mut Self::Clock, egu: &mut Self::Egu);
    fn configure_peripheral_prepare_rx(&mut self, clock: &mut Self::Clock, egu: &mut Self::Egu);
    fn configure_peripheral_next_rx(&mut self, clock: &mut Self::Clock, egu: &mut Self::Egu);
    fn configure_peripheral_tx_ack(&mut self, egu: &mut Self::Egu);
}
