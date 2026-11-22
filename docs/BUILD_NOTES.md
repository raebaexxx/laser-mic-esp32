# Build notes

## Host workspace

Requirements:

- Rust stable 1.82 or newer (`rustup` recommended);
- no external system libraries, Python, NumPy or SciPy;
- any OS with `std` support.

Commands:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

The workspace uses `rust-toolchain.toml` with the stable channel.  The host
crates are dependency-free, so the first build is fast and offline-friendly
after the toolchain is installed.

### Running the digital twin

```bash
cargo run -p granite-sim --bin granite-twin -- selftest
cargo run -p granite-sim --bin granite-twin -- suite --out out/suite
```

The suite writes `out/suite/index.html` and one directory per scenario.
Each directory contains `report.html` (static engineering report) and
`scope.html` (animated waveform, spectrum, packet stream and audio playback).
`out/suite/showcase.html` is the interactive dashboard that switches between
all scenarios in one page.

### Running the host

```bash
cargo run -p granite-host --bin granite-host -- analyze \
  --wav out/suite/healthy/capture.wav \
  --reference out/suite/healthy/reference.txt \
  --out out/host-healthy
```

### Live UDP

```bash
cargo run -p granite-host --bin granite-host -- listen \
  --bind 0.0.0.0:5005 --duration 10 --out out/live
```

If the host firewall blocks UDP 5005, allow the port or choose another with
`--bind 0.0.0.0:PORT` and set `GRANITE_HOST_PORT` when building the firmware.

## Firmware

The firmware is **not** a member of the host workspace.  It needs the Espressif
Rust toolchain because ESP32 classic is Xtensa, and the ESP-IDF SDK.

The source in this repository has been checked and linked with:

```bash
cd firmware/granite-firmware
. "$HOME/export-esp.sh"
cargo check
cargo build --release
```

against ESP-IDF v5.5.  It has not been flashed or run on hardware.

### Install the toolchain

```bash
cargo install espup espflash
espup install
. "$HOME/export-esp.sh"
```

`espup` installs a toolchain named `esp` and the Xtensa target.  The firmware's
`rust-toolchain.toml` selects it automatically when you run Cargo inside
`firmware/granite-firmware`.

### Configure

The firmware uses compile-time environment variables.  The defaults in
`src/main.rs` are placeholders:

```bash
export GRANITE_WIFI_SSID="my-network"
export GRANITE_WIFI_PASS="my-password"
export GRANITE_HOST_IP="192.168.1.50"
export GRANITE_HOST_PORT="5005"
```

`GRANITE_HOST_PORT` is optional and defaults to 5005.

### Build and flash

```bash
cd firmware/granite-firmware
cargo build --release
espflash flash --monitor target/xtensa-esp32-espidf/release/granite-firmware
```

Or, because `.cargo/config.toml` sets a runner:

```bash
cargo run --release
```

The serial monitor runs at 115200 baud.

### ESP-IDF version

`firmware/granite-firmware/.cargo/config.toml` sets:

```toml
MCU = "esp32"
ESP_IDF_VERSION = "v5.5"
ESP_IDF_TOOLS_INSTALL_DIR = "global"
```

If `esp-idf-sys` reports that the ESP-IDF version is unsupported, change
`ESP_IDF_VERSION` to the version required by the installed `esp-idf-svc`
release (for example `v5.6`).  The firmware source uses the stable
`esp-idf-hal 0.47` I2S and `esp-idf-svc 0.53` WiFi APIs.  ESP-IDF v5.5 is the
current baseline because `esp-idf-svc 0.53` uses the `n_report` field of
`wifi_event_neighbor_report_t`, which is present in v5.5 but not in v5.4.

### If the build fails

- **`can't find crate for core` / target missing:** run `espup install` and
  source `$HOME/export-esp.sh` in the same shell.
- **`ldproxy` not found:** install `espflash` and ensure `~/.cargo/bin` is in
  `PATH`.
- **`ESP-IDF` download/build errors:** delete the ESP-IDF tools cache
  (`~/.espressif` on Linux) and retry; check network/proxy settings.
- **Wrong target:** the firmware targets `xtensa-esp32-espidf`.  For ESP32-S3
  change the target to `xtensa-esp32s3-espidf`, update `MCU = "esp32s3"`, and
  check the pin assignments.
- **`unexpected cfg` warnings:** the `#![allow(unexpected_cfgs)]` attribute in
  `main.rs` covers the ESP-IDF cfg names; keep it.
- **WiFi connect fails:** verify SSID/password, use a 2.4 GHz network (ESP32
  classic does not support 5 GHz), and check the serial log.

### I2S bring-up

If the host receives packets but the signal is flat or noisy:

1. Check MCLK ≈ 12.288 MHz, BCLK ≈ 3.072 MHz, LRCK ≈ 48 kHz.
2. Confirm the ADC is actually outputting data (DOUT toggles).
3. Check the 24-bit alignment.  The firmware reads 32-bit slots and shifts
   right by 8.  If your ADC uses 24-bit slots, set `SlotBitWidth::Bits24` and
   read 6-byte stereo frames instead.
4. Check the analog bias and AC coupling.
5. Confirm the ADC's I2S format is Philips and it is in slave mode (the ESP32
   generates the clocks).

## Reproducible outputs

The digital-twin seeds are deterministic.  `suite --seed N` changes the noise
and impairment realisation but keeps the same scenario definitions.  The
default seed is `0x5eed1234`.

## Cleaning

```bash
cargo clean
rm -rf out
```

The `out/` directory contains reports, WAVs and CSVs and is ignored by git.
