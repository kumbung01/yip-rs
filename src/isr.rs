macro_rules! impl_yip_egu_handlers {
    ($($irq:ident),+ $(,)?) => {
        $(
            impl interrupt::typelevel::Handler<
                interrupt::typelevel::$irq,
            > for YipInterruptHandler {
                unsafe fn on_interrupt() {
                    // TIMER10 keeps running through connected RX completion
                    // in diagnostic builds. Capture before locking/dispatch.
                    #[cfg(all(feature = "nrf54lm20", feature = "defmt"))]
                    embassy_nrf::pac::TIMER10.tasks_capture(5).write_value(1);

                    #[cfg(all(feature = "nrf52840", feature = "defmt"))]
                    $crate::hardware::clock::capture_ack_irq();

                    IRQ_STACK.lock(|cell| {
                        let mut slot = cell.borrow_mut();

                        if let Some(role) = slot.as_mut() {
                            role.on_interrupt();
                        }
                    });
                }
            }
        )+
    };
}

pub(crate) use impl_yip_egu_handlers;
