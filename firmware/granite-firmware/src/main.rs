//! ESP32 firmware for the granite-slab laser-vibrometer NDT system.
//!
//! The microcontroller does one job: acquire mono 24-bit samples from an
//! external I2S ADC (PCM1808 or similar) at 48 kHz and stream them to the host
//! in framed UDP packets.  All DSP, modal analysis, classification and
//! reporting run on the PC, so firmware changes cannot silently change the
//! measurement method.
//!
//! Wiring (ESP32 classic, I2S0):
//!
//! | ESP32 | PCM1808 | direction |
//! |-------|---------|-----------|
//! | GPIO26| BCK     | output    |
//! | GPIO25| LRCK    | output    |
//! | GPIO27| SCKI    | output (MCLK = 256 fs = 12.288 MHz) |
//! | GPIO33| DOUT    | input     |
//! | 3V3   | VDD     | power     |
//! | GND   | GND     | ground    |
//!
//! Credentials and the host address are compiled in from environment
//! variables so that no secrets live in the repository:
//!
//! ```text
//! GRANITE_WIFI_SSID=my-network
//! GRANITE_WIFI_PASS=my-password
//! GRANITE_HOST_IP=192.168.1.50
//! GRANITE_HOST_PORT=5005
//! ```
//!
//! See `firmware/README.md` for the full build and flashing procedure.

#![allow(unknown_lints)]
#![allow(unexpected_cfgs)]

use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::i2s::config::{
    DataBitWidth, MclkMultiple, SlotBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig,
    StdSlotConfig,
};
use esp_idf_svc::hal::i2s::{I2sDriver, I2sRx};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition, sys::link_patches};
use granite_protocol::{
    encode_packet, encode_samples, PacketHeader, SampleFormat, HEADER_LEN, MAX_PAYLOAD_LEN,
};
use log::{info, warn};

const SAMPLE_RATE_HZ: u32 = 48_000;
const FRAMES_PER_PACKET: usize = 256;
const BYTES_PER_FRAME: usize = 8; // two 32-bit stereo slots
const DEVICE_ID: u32 = 0x4752_4e54; // ASCII "GRNT" little-endian

const WIFI_SSID: &str = match option_env!("GRANITE_WIFI_SSID") {
    Some(value) => value,
    None => "granite-ndt",
};
const WIFI_PASS: &str = match option_env!("GRANITE_WIFI_PASS") {
    Some(value) => value,
    None => "granite123",
};
const HOST_IP: &str = match option_env!("GRANITE_HOST_IP") {
    Some(value) => value,
    None => "192.168.1.100",
};

fn host_port() -> u16 {
    option_env!("GRANITE_HOST_PORT")
        .and_then(|value| value.parse().ok())
        .unwrap_or(5005)
}

fn main() {
    if let Err(error) = run() {
        // `main` cannot return a Result in this template (harness = false).
        // Stay alive so the serial log is visible instead of silently
        // resetting the board.
        log::error!("fatal: {error:#}");
        loop {
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

fn run() -> Result<()> {
    link_patches();
    EspLogger::initialize_default();
    info!(
        "granite-firmware starting: {} Hz, {} frames/packet, device 0x{:08x}",
        SAMPLE_RATE_HZ, FRAMES_PER_PACKET, DEVICE_ID
    );

    let peripherals = Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))?,
        sys_loop,
    )?;
    connect_wifi(&mut wifi)?;
    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
    info!("WiFi up, DHCP: {ip_info:?}");

    let host_ip: Ipv4Addr = HOST_IP
        .parse()
        .map_err(|_| anyhow!("invalid GRANITE_HOST_IP '{HOST_IP}'"))?;
    let host_port = host_port();
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect((host_ip, host_port))?;
    // Not all ESP-IDF std builds implement write timeouts; a blocking send is
    // acceptable at 154 kB/s, so do not make startup fail if this is
    // unsupported.
    let _ = socket.set_write_timeout(Some(Duration::from_millis(100)));
    info!("streaming to {host_ip}:{host_port}");

    if let Err(error) = send_status(
        &socket,
        &format!(
            "granite-esp32 ready: fs={SAMPLE_RATE_HZ} channels=1 format=i24 packet_frames={FRAMES_PER_PACKET}"
        ),
    ) {
        warn!("status packet send failed: {error}");
    }

    // `peripherals.modem` was moved into the WiFi driver; the remaining
    // peripherals are still available individually.
    let i2s_config = StdConfig::new(
        esp_idf_svc::hal::i2s::config::Config::new()
            .dma_buffer_count(8)
            .frames_per_buffer(FRAMES_PER_PACKET as u32),
        StdClkConfig::from_sample_rate_hz(SAMPLE_RATE_HZ).mclk_multiple(MclkMultiple::M256),
        StdSlotConfig::philips_slot_default(DataBitWidth::Bits24, SlotMode::Stereo)
            .slot_bit_width(SlotBitWidth::Bits32),
        StdGpioConfig::default(),
    );

    let mut i2s = I2sDriver::<I2sRx>::new_std_rx(
        peripherals.i2s0,
        &i2s_config,
        peripherals.pins.gpio26,       // BCLK
        peripherals.pins.gpio33,       // DIN (ADC data)
        Some(peripherals.pins.gpio27), // MCLK
        peripherals.pins.gpio25,       // LRCK / WS
    )?;
    i2s.rx_enable()?;
    info!("I2S0 receiver enabled");

    stream_audio(&socket, &mut i2s)
}

fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> Result<()> {
    let configuration = Configuration::Client(ClientConfiguration {
        ssid: WIFI_SSID
            .try_into()
            .map_err(|_| anyhow!("WiFi SSID is too long"))?,
        bssid: None,
        auth_method: AuthMethod::WPA2Personal,
        password: WIFI_PASS
            .try_into()
            .map_err(|_| anyhow!("WiFi password is too long"))?,
        channel: None,
        ..Default::default()
    });
    wifi.set_configuration(&configuration)?;
    wifi.start()?;
    info!("WiFi started");
    wifi.connect()
        .map_err(|error| anyhow!("WiFi connect failed (ssid '{WIFI_SSID}'): {error}"))?;
    info!("WiFi connected");
    wifi.wait_netif_up()?;
    info!("network interface up");
    Ok(())
}

fn stream_audio(socket: &UdpSocket, i2s: &mut I2sDriver<'_, I2sRx>) -> Result<()> {
    let mut raw = vec![0u8; FRAMES_PER_PACKET * BYTES_PER_FRAME];
    let mut samples = vec![0.0f32; FRAMES_PER_PACKET];
    let mut packet = vec![0u8; HEADER_LEN + FRAMES_PER_PACKET * 3 + 32];
    let mut sequence = 0u32;
    let mut first_sample_index = 0u64;
    let mut sent_packets = 0u64;
    let mut failed_sends = 0u64;
    let mut last_report = Instant::now();
    let timeout = TickType::new_millis(200).ticks();

    loop {
        let bytes_read = match i2s.read(&mut raw, timeout) {
            Ok(bytes) => bytes,
            Err(error) => {
                warn!("I2S read failed: {error}");
                continue;
            }
        };
        let frames = (bytes_read / BYTES_PER_FRAME).min(FRAMES_PER_PACKET);
        if frames == 0 {
            continue;
        }

        // Each stereo frame is two 32-bit slots; the PCM1808 puts a 24-bit
        // sample in the top bits of the left slot.  Shift it down to a signed
        // 24-bit value and normalise to [-1, 1].
        for frame in 0..frames {
            let offset = frame * BYTES_PER_FRAME;
            let left = i32::from_le_bytes([
                raw[offset],
                raw[offset + 1],
                raw[offset + 2],
                raw[offset + 3],
            ]);
            let value = (left >> 8) as f32 / 8_388_608.0;
            samples[frame] = value.clamp(-1.0, 1.0);
        }

        let header = PacketHeader::data(
            sequence,
            DEVICE_ID,
            first_sample_index,
            first_sample_index * 1_000_000_000 / SAMPLE_RATE_HZ as u64,
            SAMPLE_RATE_HZ,
            1,
            frames as u16,
            SampleFormat::I24Packed,
            24,
        );
        let written = encode_samples(&header, &samples[..frames], &mut packet)?;
        match socket.send(&packet[..written]) {
            Ok(count) if count == written => sent_packets += 1,
            Ok(count) => {
                failed_sends += 1;
                warn!("short UDP send: {count}/{written} bytes");
            }
            Err(error) => {
                failed_sends += 1;
                warn!("UDP send failed: {error}");
            }
        }

        sequence = sequence.wrapping_add(1);
        first_sample_index += frames as u64;

        if last_report.elapsed() >= Duration::from_secs(5) {
            let seconds = first_sample_index as f64 / SAMPLE_RATE_HZ as f64;
            info!(
                "streaming: {seconds:.1} s captured, {sent_packets} packets sent, {failed_sends} send errors"
            );
            last_report = Instant::now();
        }
    }
}

fn send_status(socket: &UdpSocket, text: &str) -> Result<()> {
    let text = &text[..text.len().min(MAX_PAYLOAD_LEN.min(400))];
    let header = PacketHeader::status(0, DEVICE_ID, 0, SAMPLE_RATE_HZ, text);
    let mut buffer = vec![0u8; HEADER_LEN + text.len() + 16];
    let written = encode_packet(&header, text.as_bytes(), &mut buffer)?;
    socket.send(&buffer[..written])?;
    Ok(())
}
