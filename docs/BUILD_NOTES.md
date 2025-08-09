# Build Notes

- Keep the photodiode and first op-amp stage shielded from ambient light; use a short tube or lens hood.
- Start with a **very low-power** 650 nm laser. Never point at eyes. Use goggles where appropriate.
- Use a solid tripod for the laser and sensor to minimize mechanical noise.
- Prefer ADC1 pins on ESP32 (GPIO 32–39) when Wi‑Fi is active.
- Tune HPF/LPF values to taste; aim for ~300–3200 Hz voice band.
