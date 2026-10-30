# nRF54 peripheral hardware

The peripheral API follows the nRF52 scan / receive / ACK state machine.
GRTC supplies absolute microsecond timestamps; TIMER10 captures RXADDRESS
relative to the scheduled start and supplies receive timeouts. This replaces
the nRF52 RTC tick plus fractional TIMER CC0 start. Connection periods and drift
are stored in integer microseconds, with no scale parameter or fixed-point
arithmetic in the nRF54 path. The quarter-step drift correction is quantized
to whole microseconds; the early receive window is retained. The nRF52 backend
keeps its RTC fixed-point representation internally.

## Resources

- RADIO, **TIMER10**, EGU10 (RADIO domain / DPPIC10).
- One owned GRTC CC0..10 (PERI domain / DPPIC20). CC11 remains owned by Embassy.
- Five RADIO DPPI channels and one PERI DPPI channel.
- PPIB21 -> PPIB11 bridge, as allocated in `firmware/central`.
- Two RADIO DPPI groups, both gating address capture and automatic ACK.

`DeviceContext::new` accepts TIMER10 explicitly, five RADIO channel tokens,
and two group tokens. This prevents allocating a TIMER in a different DPPI domain.
Stopping a role disarms only its GRTC compare channel; it never stops the shared
GRTC counter or Embassy's CC11 timer service.

## Receive / ACK routes

Channel numbers here are array positions, not hardcoded hardware channel IDs.

| RADIO channel | Publishers | Subscribers |
| --- | --- | --- |
| 0 | PPIB receive from GRTC compare | TIMER start, RADIO RXEN, both group EN tasks |
| 1 | RADIO RXADDRESS | TIMER capture 2 |
| 2 | TIMER compare 1, RADIO CRCERROR | error group DIS, EGU task 2 |
| 3 | RADIO CRCOK, EGU event 2 | TimeslotEnd task, RADIO DISABLE, TIMER stop/clear |
| 4 | RADIO DISABLED | RADIO TXEN, one-shot group DIS |

The error path disables ACK before forwarding to completion through EGU event 2.
Its interrupt is never enabled, preserving the invariant that only Start or
TimeslotEnd is an enabled EGU interrupt. Two groups are needed because one
DPPI task cannot subscribe to two different channels.

Scan mode ends on RADIO DISABLED (including timeout), without auto-ACK.
Prepare mode forwards GRTC compare to Start. ACK mode forwards RADIO DISABLED
to TimeslotEnd. During connected RX, only RXREADY automatically triggers START.
RX DISABLED triggers TXEN to overlap ACK ramp-up with CPU processing; TX may
remain in TXIDLE until ACK data is ready. `configure_peripheral_tx_ack` first
installs the DISABLED completion route, then arms a one-shot START route:

| Reused RADIO channel | Publishers | Subscribers |
| --- | --- | --- |
| 0 | RADIO DISABLED | TimeslotEnd task |
| 1 | RADIO TXREADY, EGU event 3 | RADIO START, one-shot group DIS |

The START group contains channel 1, so the first request disables its own route.
After arming, if RADIO is already TXIDLE, software triggers EGU event 3 to handle
a READY that occurred before connection. Both requests use the same one-shot
route; there is no competing direct CPU START or ramp-up polling loop. Event 3
does not enable an IRQ. A valid CRC is accepted even when TXREADY preceded the
receive handler, since the ACK still has not started. Scan/central retain
READY -> START. No new RADIO channels or groups are allocated for ACK START.
Each mode disconnects old endpoints before connecting new ones;
cleanup uses the owned GRTC channel rather than assuming CC0.

## ACK timing diagnostics

With `defmt`, connected RX keeps TIMER10 running after completion so interrupt
latency includes CPU sleep/wakeup time. DPPI captures completion (CRCOK on a
successful receive) in CC3 and RX DISABLED / TXEN in CC4. The IRQ handler captures
CC5 before locking or dispatch. `disabled_rx` uses CC0 for its entry and the
instant just before updating PACKETPTR. CC2 remains the RXADDRESS sync capture.
The timeout channel is disabled at RX DISABLED to prevent a second completion
while preparing the ACK; the timer is stopped/cleared when arming ACK completion.

`ACK timing` is printed after arming an empty ACK. Its fields are microseconds
at the configured 1 MHz TIMER frequency:

- `event_to_irq_us`: completion event to the first capture in the IRQ handler.
- `irq_to_handler_us`: IRQ capture to `disabled_rx` entry capture.
- `handler_to_ptr_us`: entry capture to the capture just before PACKETPTR setup.
- `txen_to_ptr_us`: RX DISABLED / TXEN capture to the pre-PACKETPTR capture.

Captures add diagnostic overhead and have 1 us resolution. Without `defmt`, the
original TIMER stop/clear completion route is retained.

`ACK cycles` adds DWT snapshots on both chip backends. Fields are CPU cycles,
not microseconds: `events` covers RADIO event reads/clears; `header` includes
validation and header decoding; `rx` covers receive processing; `commit` covers
ACK/retry bookkeeping; `build` covers outgoing payload construction and queue
access; `power` covers power control and sequence-bit setup; `regs` covers
PACKETPTR/TXADDRESS writes; `arm` covers PPI/EGU setup. Logging occurs after ACK arming. Instrumentation
adds overhead, and these fields exclude the existing pre-PACKETPTR diagnostic
captures between `power` and `regs`, so their sum is not a full-handler total.

At startup, `ACK memory config` reports the existing cache enable state, RRAM
low-power mode bits and constant-latency status. It does not change them.
The phase measurements localize cost but do not by themselves prove that RRAM
instruction fetches are responsible; that requires a controlled comparison.

## Validation

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
