macro_rules! impl_yip_egu_handlers {
    ($($irq:ident),+ $(,)?) => {
        $(
            impl interrupt::typelevel::Handler<
                interrupt::typelevel::$irq,
            > for YipInterruptHandler {
                unsafe fn on_interrupt() {
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
