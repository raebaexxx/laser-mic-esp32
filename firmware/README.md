# granite-firmware

Rust ESP-IDF firmware for the granite laser-vibrometer NDT system.

The firmware acquires mono 24-bit samples from an external I2S ADC at 48 kHz
and streams them to the PC in framed UDP packets.  It does not run DSP or
classification.

## Hardware

- ESP32 classic (Xtensa) dev board;
- PCM1808 (or compatible) 24-bit I2S ADC breakout;
- analog front-end from `docs/hardware.md`;
- 5 V and 3.3 V supplies;
- WiFi access point and a PC on the same network.

### Wiring

| ESP32 | PCM1808 | Direction |
|---|---|---|
| GPIO26 | BCK | output |
| GPIO25 | LRCK | output |
| GPIO27 | SCKI | output (MCLK 12.288 MHz) |
| GPIO33 | DOUT | input |
| 3V3 | VDD (digital) | power |
| 5V | VCC (analog) | power |
| GND | GND | ground |

Confirm the logic levels of your ADC breakout.  The ESP32 is not 5 V tolerant.

## Build

```bash
cargo install espup espflash
espup install
. "$HOME/export-esp.sh"

cd firmware/granite-firmware
```

Set the credentials and host address:

```bash
export GRANITE_WIFI_SSID="my-network"
export GRANITE_WIFI_PASS="my-password"
export GRANITE_HOST_IP="192.168.1.50"
# optional, default 5005
export GRANITE_HOST_PORT="5005"
```

Build:

```bash
cargo build --release
```

Flash and monitor:

```bash
cargo run --release
```

or:

```bash
espflash flash --monitor target/xtensa-esp32-espidf/release/granite-firmware
```

The serial monitor runs at 115200 baud.

## What it does

1. Initialises NVS, the system event loop and WiFi station mode.
2. Connects to `GRANITE_WIFI_SSID` with WPA2.
3. Creates a UDP socket connected to `GRANITE_HOST_IP:GRANITE_HOST_PORT`.
4. Sends one status packet (`granite-esp32 ready: ...`).
5. Configures I2S0:
   - 48 kHz;
   - Philips I2S, stereo slots;
   - 32-bit slots, 24 valid bits;
   - MCLK = 256 × fs;
   - 8 DMA buffers × 256 frames.
6. Reads 256 stereo frames at a time, takes the left 24-bit sample, encodes a
   `granite-protocol` packet, and sends it.
7. Logs a status line every 5 seconds.

Packet size: `52 + 256 × 3 = 820` bytes.  Packet rate: 187.5/s.  Data rate:
about 154 kB/s.

## Configuration

The constants are at the top of `src/main.rs`:

```rust
const SAMPLE_RATE_HZ: u32 = 48_000;
const FRAMES_PER_PACKET: usize = 256;
const DEVICE_ID: u32 = 0x4752_4e54;
```

The compile-time environment variables are read with `option_env!`, so changing
them requires a rebuild (Cargo does not always notice environment changes; use
`cargo build --release` after `touch src/main.rs` if needed).

Pin assignments are in the `I2sDriver::new_std_rx` call.

## Protocol

See `docs/protocol.md`.  The firmware uses:

- `SampleFormat::I24Packed`;
- one channel;
- `first_sample_index` = running sample count;
- `timestamp_ns` = `first_sample_index × 1e9 / fs`;
- `sequence` = data-packet counter;
- CRC32 over header + payload.

Status packets use kind 2 and a separate sequence value of 0; they are not fed
to the host's stream assembler.

## Troubleshooting

| Symptom | Check |
|---|---|
| `WiFi connect failed` | SSID/password, 2.4 GHz network, signal strength |
| `invalid GRANITE_HOST_IP` | use a dotted IPv4 address, not a hostname |
| No packets on host | host IP/port, firewall, same subnet, `ping` the ESP32 |
| Host sees CRC errors | WiFi corruption; check RSSI, reduce distance, check MTU |
| Flat/noisy waveform | ADC clocks, analog bias, laser alignment, I2S slot width |
| `I2S read failed` | DMA config, MCLK, pin conflicts, ADC not clocking |
| Clipping warning on host | reduce analog gain or laser power |
| Build can't find `core` | run `espup install` and source `export-esp.sh` |
| Unsupported ESP-IDF version | change `ESP_IDF_VERSION` in `.cargo/config.toml` |

### Verifying the clocks

With a scope or logic analyser:

- MCLK (GPIO27): 12.288 MHz;
- BCLK (GPIO26): 3.072 MHz;
- LRCK (GPIO25): 48 kHz, 50% duty;
- DOUT (GPIO33): 24-bit data, left channel first.

If the ADC expects 24-bit slots instead of 32-bit slots, change
`SlotBitWidth::Bits32` to `Bits24` and adapt the read loop to 6-byte stereo
frames.  The current code assumes 32-bit slots with the 24-bit sample in the
top bits, which is the common PCM1808 configuration.

## Limitations

- No WiFi reconnection logic; if the access point drops, reboot the board.
- No watchdog or brown-out handling beyond the ESP-IDF defaults.
- No local buffering; a long WiFi stall becomes a host-side gap.
- I2S DMA overflow is not reported; if the loop stalls badly, samples can be
  lost without a visible host-side gap.  Keep the WiFi link healthy and watch
  the packet-loss/jitter statistics.
- No NTP or wall-clock timestamp; the timestamp is sample-clock based.
- Fixed sample rate and packet size.
- The firmware compiles and links with ESP-IDF v5.5, but it has not been
  flashed or run on hardware.  Treat the first flash as hardware bring-up.
