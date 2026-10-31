# nRF54 peripheral hardware

The peripheral API follows the nRF52 scan / receive / ACK state machine.
GRTC supplies absolute microsecond timestamps; TIMER10 captures RXADDRESS
relative to the scheduled start and supplies receive timeouts. This replaces
the nRF52 RTC tick plus fractional TIMER CC0 start. Connection periods and drift
are stored in integer microseconds, with no scale parameter or fixed-point
arithmetic in the nRF54 path. The quarter-step drift correction is quantized
to whole microseconds; the early receive window is retained. The nRF52 backend
keeps its RTC fixed-point representation internally.

## Instruction cache lifecycle

The nRF54 peripheral enables ICACHE at the beginning of `start()` and disables
it after cleanup in `stop()`, with synchronization barriers. Firmware `main()`
does not configure it separately. This is independent of defmt and the
reduce-rram-latency feature. Resync leaves the cache enabled while the role runs.
The cache is a CPU-wide resource; this lifecycle applies globally.

## RRAM latency feature

Enable YIP's optional `reduce-rram-latency` feature together with `nrf54lm20`
to reduce the peripheral's connected RX interrupt wake-up latency. Before
arming RX, it sets `RRAMC.POWER.LOWPOWERCONFIG.MODE` to Standby and triggers
RRAM wake-up. RX completion (including failures) and peripheral stop set the
mode back to PowerOff. This behavior does not depend on `defmt`.

The feature is disabled by default and has no effect on other chip backends or
the central role. With it disabled, YIP does not modify RRAMC power settings.
Standby increases idle power while an RX window is scheduled. This changes
global RRAMC settings, so applications sharing RRAMC must coordinate this policy.
The `firmware/nrf54-peripheral` package enables the feature explicitly.

## Resources

- RADIO, TIMER10 and EGU10 in the RADIO DPPI domain (DPPIC10).
- One owned GRTC CC0..10 in the PERI domain; Embassy retains CC11.
- Eleven RADIO DPPI channels, one PERI channel and one PPIB21 -> PPIB11 bridge pair.
- Four RADIO groups: RX, RX error, RX completion and ACK start.

`DeviceContext::new` takes eleven RADIO channel tokens and four group tokens.
Central retains its three-channel routing, with the remaining owned channels disabled.
Stopping disarms only the owned GRTC compare, never the shared counter or CC11.

## Fixed connected RX / ACK routes

The role explicitly calls `configure_rx()` once after successful sync. Subsequent prepare/RX/ACK
transitions disable owned channels, wait for register writes to complete, and
arm a new channel mask. They do not rewrite publishers, subscribers, shorts or
group membership. There is no cached layout or initialization check. Stop disarms
the routes; `configure_sync()` installs scan routes on restart/resync, and the
next successful sync explicitly installs connected routes again.
Channel numbers below are array positions, not hardware channel IDs.

| Channel | Publishers | Subscribers |
| --- | --- | --- |
| 0 | GRTC through PPIB | EGU PrepareRx::Wake, Rx::Rxen |
| 1 | RXADDRESS | TIMER capture 2 |
| 2 | TIMER compare 1, CRCERROR | error group DIS, EGU Rx::Failure |
| 3 | CRCOK, EGU Rx::Failure | RADIO DISABLE, EGU Rx::Done, completion group DIS |
| 4 | DISABLED | EGU Rx::Disabled, TxAck::Disabled |
| 5 | EGU Rx::Disabled | TXEN, RX group DIS |
| 6 | EGU TxAck::Disabled | EGU TxAck::Done |
| 7 | EGU Rx::Done, TxAck::Done | EGU TimeslotEnd |
| 8 | TXREADY, EGU TxAck::Start | RADIO START, ACK-start group DIS |
| 9 | EGU PrepareRx::Wake | EGU Start |
| 10 | EGU Rx::Rxen | RXEN, TIMER START, all three RX group EN tasks |

EGU numbers are part of the shared `Trigger` API. IRQ indices remain 0
(TimeslotEnd) and 1 (Start). Internal relays descend from 15:
Rx::{Rxen,Failure,Done,Disabled}=15..12, PrepareRx::Wake=11,
TxAck::{Disabled,Done,Start}=10..8. Relay interrupts are never enabled.
Sync and connected RX reuse Rx::Rxen with different bindings installed at
explicit mode transitions.
EGU flags do not replay events when an output channel is enabled.

| State | Initially enabled RADIO channels |
| --- | --- |
| Preparation wakeup | 0, 9 |
| Scheduled RX | 0, 4, 7, 10 |
| ACK completion armed | 4, 6, 7 |
| ACK ready to start | 4, 6, 7, 8 |

Actual RX start enables the RX/error groups (1,2,5) and completion group (2,3).
Failure disables automatic TXEN before forwarding completion. CRCOK or forwarded
failure disables both completion sources. Successful RX DISABLED ramps TX up
and disarms the RX group. ACK completion routing is armed before channel 8;
a DSB makes ACK data/PACKETPTR writes visible before START. Channel 8 disables
itself after the first request. If TXREADY already happened, software triggers
TxAck::Start through that same one-shot route; there is no busy waiting.
Connected RX uses RXREADY_START, with READY_START off.

Scan/sync uses a separate graph, with GRTC -> EGU Rx::Rxen -> RXEN/TIMER START
and completion on DISABLED. Peripheral TIMER CC1 compare uses STOP/CLEAR
shorts instead of completion-channel STOP/CLEAR subscriptions. On early CRCOK
or CRCERROR, the timer continues until CC1 or the next RX setup, which explicitly
stops and clears it. CC2 ADDRESS capture remains available for sync processing.
Central retains its existing CC0/DPPI timer cleanup. Scan and central retain READY_START. Switching
between sync and connected mode rewires endpoints with all owned channels off.
The added EGU hops change hardware propagation delay; board verification is
required for timing and drift, especially sync acquisition and automatic TXEN.

## Validation

Timing instrumentation has been removed. Packet-content, duplicate/empty-packet
and application data/statistics logs are commented out; error and connection
status logs remain available.

From the repository root:

```powershell
rustc --test --edition 2024 src/hardware/nrf54/clock/schedule.rs --target x86_64-pc-windows-msvc -o target/nrf54-schedule-tests.exe
./target/nrf54-schedule-tests.exe
cargo build --manifest-path firmware/central/Cargo.toml --example nrf54_peripheral --release --target thumbv8m.main-none-eabihf
```

The tests cover scan phase, receive-window margins, drift in both directions,
expired/invalid references, preparation wakeup and timestamps beyond 32 bits.
The example links a complete peripheral, including EGU10 interrupt handling.

Board verification is still required: scan acquisition against an nRF52 central,
multi-fragment RX / ACK, heartbeat drift, CRC-error and timeout paths, and
stop/restart while Embassy timers continue to operate. The 100 us scheduling
headroom and RADIO/PPIB propagation delays must be checked on the actual board.

References: [Nordic DPPI / domain routing][dppi], [nRF54LM20 TIMER registers][timer],
and the local Embassy `time_driver.rs` for the CCL-then-CCH compare sequence.

[dppi]: https://academy.nordicsemi.com/courses/nrf54l-series-express-course/lessons/lesson-2-power-domains-event-system-and-gpio/topic/event-system-and-dppi/
[timer]: https://docs.nordicsemi.com/r/bundle/ps_nrf54lm20a/page/timer.html-topic
