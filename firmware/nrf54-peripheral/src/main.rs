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
const PERIPHERAL_ID: u8 = 1;
const SEND_INTERVAL: Duration = Duration::from_secs(3);

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = embassy_nrf::config::Config::default();
    // config.clock_speed = embassy_nrf::config::ClockSpeed::CK128;
    config.hfclk_source = embassy_nrf::config::HfclkSource::ExternalXtal;
    config.lfclk_source = embassy_nrf::config::LfclkSource::ExternalXtal;
    let p = embassy_nrf::init(config);

    // Enable the RRAM instruction cache before initializing the radio role.
    // Enabling it also invalidates any previous cache entries.
    // embassy_nrf::pac::ICACHE
    //     .enable()
    //     .write(|w| w.set_enable(true));
    // cortex_m::asm::dsb();
    // cortex_m::asm::isb();
    //
    // embassy_nrf::pac::RRAMC
    //     .power()
    //     .config()
    //     .modify(|w| w.set_accesstimeout(2000 * 32)); // 300µs

    bind_interrupts!(
        struct Irqs {
            EGU10 => YipInterruptHandler;
        }
    );

    let device = DeviceContext::new(
        p.RADIO,
        p.TIMER10,
        p.GRTC_CH0,
        ppi_channels![
            p.PPI10_CH0,
            p.PPI10_CH1,
            p.PPI10_CH2,
            p.PPI10_CH3,
            p.PPI10_CH4,
            p.PPI10_CH5,
            p.PPI10_CH6,
            p.PPI10_CH7,
            p.PPI10_CH8,
            p.PPI10_CH9,
            p.PPI10_CH10,
        ],
        ppi_channels![p.PPI20_CH0],
        (p.PPIB21_CH0, p.PPIB11_CH0),
        [
            core::convert::Into::into(p.PPI10_GROUP0),
            core::convert::Into::into(p.PPI10_GROUP1),
            core::convert::Into::into(p.PPI10_GROUP2),
            core::convert::Into::into(p.PPI10_GROUP3),
        ],
        Irqs,
        0,
    );

    let session = yip::peripheral::init(device, PERIPHERAL_ID);

    // nRF54 DK button pins, active-low with pull-ups.
    let b1 = Input::new(p.P1_26, Pull::Up);
    let b2 = Input::new(p.P1_09, Pull::Up);
    let b3 = Input::new(p.P1_08, Pull::Up);
    let b4 = Input::new(p.P0_05, Pull::Up);

    spawner.spawn(gpio_task(b1, b2, b3, b4).unwrap());

    defmt::info!(
        "nRF54 peripheral {} initialized; press a button to start",
        PERIPHERAL_ID
    );

    spawner.spawn(session_task(session).unwrap());
}

#[embassy_executor::task]
async fn session_task(session: YipSession) {
    defmt::info!("session {} start", PERIPHERAL_ID);
    let (rx, mut tx, conn) = session.split();
    let mut stats = CommunicationStats::default();

    let pressed = COUNTS.receive().await;
    yip::peripheral::start();
    let mut next_send = Instant::now() + SEND_INTERVAL;
    let mut next_value = 1u32;
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
            Timer::at(next_send),
        )
        .await
        {
            Either4::First(pressed) => {
                if connected {
                    stats.send(&mut tx, pressed);
                } else {
                    pending_button = Some(pressed);
                }
            }
            Either4::Second(()) => {
                connected = !connected;
                if connected {
                    defmt::info!("session connected");
                    next_send = Instant::now() + SEND_INTERVAL;
                    if let Some(pressed) = pending_button.take() {
                        stats.send(&mut tx, pressed);
                    }
                } else {
                    defmt::info!("session disconnected");
                }
            }
            Either4::Third(packet) => {
                stats.rx_packets += 1;
                match core::str::from_utf8(packet.as_slice()) {
                    Ok(text) => defmt::info!("data: {}", text),
                    Err(_) => defmt::warn!("invalid UTF-8"),
                }
                defmt::info!(
                    "stats: tx_attempts {} tx_queued {} tx_failed {} rx_packets {}",
                    stats.tx_attempts,
                    stats.tx_queued,
                    stats.tx_attempts - stats.tx_queued,
                    stats.rx_packets,
                );
            }
            Either4::Fourth(()) => {
                next_send = Instant::now() + SEND_INTERVAL;
                if connected {
                    stats.send(&mut tx, next_value);
                    next_value = next_value % 4 + 1;
                }
            }
        }
    }
}

#[derive(Default)]
struct CommunicationStats {
    tx_attempts: u64,
    tx_queued: u64,
    rx_packets: u64,
}

impl CommunicationStats {
    fn send(&mut self, tx: &mut YipWriter, value: u32) {
        let mut arr = [0u8; 200];
        arr[..4].copy_from_slice(&value.to_le_bytes());
        self.tx_attempts += 1;
        if tx.try_write(&arr).is_ok() {
            self.tx_queued += 1;
            defmt::info!("send: attempt {} value {}", self.tx_attempts, value);
        } else {
            defmt::warn!("send failed: attempt {} value {}", self.tx_attempts, value);
        }
    }
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
