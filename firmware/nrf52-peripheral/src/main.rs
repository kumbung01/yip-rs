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
const PERIPHERAL_ID: u8 = 0;
const IDLE_TIMEOUT: Duration = Duration::from_secs(10);

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = embassy_nrf::config::Config::default();
    config.hfclk_source = embassy_nrf::config::HfclkSource::ExternalXtal;
    config.lfclk_source = embassy_nrf::config::LfclkSource::ExternalXtal;
    let p = embassy_nrf::init(config);

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

    let session = yip::peripheral::init(device, PERIPHERAL_ID);

    // nRF52840 DK button pins, active-low with pull-ups.
    let b1 = Input::new(p.P0_11, Pull::Up);
    let b2 = Input::new(p.P0_12, Pull::Up);
    let b3 = Input::new(p.P0_24, Pull::Up);
    let b4 = Input::new(p.P0_25, Pull::Up);

    spawner.spawn(gpio_task(b1, b2, b3, b4).unwrap());

    defmt::info!(
        "nRF52 peripheral {} initialized; press a button to start",
        PERIPHERAL_ID
    );

    spawner.spawn(session_task(session).unwrap());
}

#[embassy_executor::task]
async fn session_task(session: YipSession) {
    defmt::info!("session {} start", PERIPHERAL_ID);
    let (rx, mut tx, conn) = session.split();

    loop {
        let pressed = COUNTS.receive().await;
        yip::peripheral::start();
        let mut deadline = Instant::now() + IDLE_TIMEOUT;
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
                    deadline = Instant::now() + IDLE_TIMEOUT;
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
                Either4::Third(packet) => match core::str::from_utf8(packet.as_slice()) {
                    Ok(text) => defmt::info!("data: {}", text),
                    Err(_) => defmt::warn!("invalid UTF-8"),
                },
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

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    defmt::error!("panic: {}", defmt::Display2Format(info));
    loop {
        core::hint::spin_loop();
    }
}
