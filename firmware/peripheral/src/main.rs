#![no_std]
#![no_main]
use defmt_rtt as _;
use embassy_futures::select::{Either4, select4};
use embassy_time::{Duration, Instant, Timer};

use embassy_nrf::bind_interrupts;
use embassy_nrf::gpio::{Input, Pull};
use embassy_sync::channel::Channel;
use yip::DeviceContext;
use yip::YipSession;

use yip::YipWriter;
use yip::ppi_channels;

use embassy_executor::Spawner;

use yip::peripheral::YipInterruptHandler;
const LINKS: usize = 1;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = embassy_nrf::config::Config::default();
    config.hfclk_source = embassy_nrf::config::HfclkSource::ExternalXtal;
    // config.lfclk_source = embassy_nrf::config::LfclkSource::Synthesized;
    config.lfclk_source = embassy_nrf::config::LfclkSource::ExternalXtal;
    let p = embassy_nrf::init(config);

    // let clock = embassy_nrf::pac::CLOCK;
    //     clock.events_hfclkstarted().write_value(0);
    //     clock.tasks_hfclkstart().write_value(1);
    //     while clock.events_hfclkstarted().read() == 0 {}
    //     defmt::info!("HFCLK is startted");

    bind_interrupts!(
        struct Irqs {
            EGU2_SWI2 => YipInterruptHandler;
        }
    );

    let device = DeviceContext::new(
        p.RADIO,
        p.TIMER2,
        p.RTC2,
        ppi_channels![
            p.PPI_CH0, p.PPI_CH1, p.PPI_CH2, p.PPI_CH3, p.PPI_CH4, p.PPI_CH5, p.PPI_CH6, p.PPI_CH7,
        ],
        p.PPI_GROUP0,
        p.EGU2,
        Irqs,
        0,
    );

    let session = yip::peripheral::init(device, 0);

    let b1 = Input::new(p.P0_11, Pull::Up);
    let b2 = Input::new(p.P0_12, Pull::Up);
    let b3 = Input::new(p.P0_24, Pull::Up);
    let b4 = Input::new(p.P0_25, Pull::Up);

    spawner.spawn(gpio_task(b1, b2, b3, b4).unwrap());

    defmt::info!("Yip initialized");

    spawner.spawn(session_task(session, 0).unwrap());
}

#[embassy_executor::task(pool_size = LINKS)]
async fn session_task(session: YipSession, id: usize) {
    defmt::info!("session {} start", id);
    let (rx, mut tx, conn) = session.split();

    loop {
        let pressed = COUNTS.receive().await;
        yip::peripheral::start();
        let mut deadline = Instant::now() + Duration::from_secs(10);
        let mut connected = false;
        let mut pending_button = Some(pressed);

        loop {
            let connection_changed = async {
                if connected {
                    conn.wait_disconnected().await;
                } else {
                    conn.wait_connected().await;
                }
            };

            match select4(
                COUNTS.receive(),
                connection_changed,
                rx.read(),
                Timer::at(deadline),
            )
            .await
            {
                Either4::First(pressed) => {
                    deadline = Instant::now() + Duration::from_secs(10);
                    if connected {
                        send_button(&mut tx, pressed);
                    } else {
                        pending_button = Some(pressed);
                    }
                }
                Either4::Second(()) => {
                    connected = !connected;
                    if connected {
                        defmt::info!("session connected");
                        if let Some(pressed) = pending_button.take() {
                            send_button(&mut tx, pressed);
                        }
                    } else {
                        defmt::info!("session disconnected");
                    }
                }
                Either4::Third(_packet) => {
                    // defmt::info!("data: {}", _packet.as_slice());
                }
                Either4::Fourth(()) => {
                    yip::peripheral::stop();
                    defmt::info!("peripheral stopped");
                    break;
                }
            }
        }
    }
}

fn send_button(tx: &mut YipWriter, pressed: u32) {
    let mut arr = [0u8; 200];
    let bytes = pressed.to_le_bytes();
    arr[..bytes.len()].copy_from_slice(&bytes);
    let _ = tx.try_write(&arr);
}

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

pub static COUNTS: Channel<CriticalSectionRawMutex, u32, 16> = Channel::new();

#[embassy_executor::task]
async fn gpio_task(
    mut b1: Input<'static>,
    mut b2: Input<'static>,
    mut b3: Input<'static>,
    mut b4: Input<'static>,
) {
    loop {
        let id = match select4(
            b1.wait_for_low(),
            b2.wait_for_low(),
            b3.wait_for_low(),
            b4.wait_for_low(),
        )
        .await
        {
            Either4::First(_) => 1,
            Either4::Second(_) => 2,
            Either4::Third(_) => 3,
            Either4::Fourth(_) => 4,
        };

        defmt::info!("버튼 {} 눌림", id);
        if id == 2 {
            dump_sync_state();
        }
        // 여기서 Channel로 id 전달 가능
        if COUNTS.try_send(id).is_err() {
            defmt::info!("send {} failed!", id);
        }

        // 누르고 있는 동안 반복 처리되지 않도록 해제 대기
        match id {
            1 => b1.wait_for_high().await,
            2 => b2.wait_for_high().await,
            3 => b3.wait_for_high().await,
            4 => b4.wait_for_high().await,
            _ => unreachable!(),
        }
    }
}

fn dump_sync_state() {
    use embassy_nrf::pac;

    // CC3 is unused by the peripheral clock; capture without changing CC0/CC1.
    pac::TIMER2.tasks_capture(3).write_value(1);
    defmt::info!(
        "sync diag rtc now {} cc0 {} event0 {} evten {=u32:08x}",
        pac::RTC2.counter().read().0,
        pac::RTC2.cc(0).read().0,
        pac::RTC2.events_compare(0).read(),
        pac::RTC2.evten().read().0,
    );
    defmt::info!(
        "sync diag timer now {} cc0 {} cc1 {} event0 {} event1 {} ppi {=u32:08x}",
        pac::TIMER2.cc(3).read(),
        pac::TIMER2.cc(0).read(),
        pac::TIMER2.cc(1).read(),
        pac::TIMER2.events_compare(0).read(),
        pac::TIMER2.events_compare(1).read(),
        pac::PPI.chen().read().0,
    );
    defmt::info!(
        "sync diag radio state {} rxready {} disabled {} egu event0 {} inten {=u32:08x}",
        pac::RADIO.state().read().0,
        pac::RADIO.events_rxready().read(),
        pac::RADIO.events_disabled().read(),
        pac::EGU2.events_triggered(0).read(),
        pac::EGU2.inten().read().0,
    );
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    defmt::error!("panic: {}", defmt::Display2Format(info));
    loop {
        core::hint::spin_loop();
    }
}
