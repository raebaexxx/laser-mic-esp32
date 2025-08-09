/*
  LaserMicESP32.ino
  - Samples analog on GPIO34 (ADC1_CH6) at ~8 kHz
  - Sends 16-bit PCM frames via UDP to a receiver
  - Educational use only
*/
#include <WiFi.h>
#include <WiFiUdp.h>

// ====== CONFIG ======
const char* WIFI_SSID = "YOUR_WIFI";
const char* WIFI_PASS = "YOUR_PASS";
IPAddress DEST_IP(192,168,1,100);   // set to your PC IP
const uint16_t DEST_PORT = 5005;
const int ADC_PIN = 34;             // ADC1 (works with WiFi)
const int FS = 8000;                // sample rate
const int FRAME = 256;              // samples per packet
// ====================

WiFiUDP udp;
int16_t buf[FRAME];

void setup() {
  Serial.begin(115200);
  WiFi.mode(WIFI_STA);
  WiFi.begin(WIFI_SSID, WIFI_PASS);
  Serial.print("WiFi...");
  while (WiFi.status() != WL_CONNECTED) { delay(250); Serial.print("."); }
  Serial.println("\nConnected: " + WiFi.localIP().toString());

  analogReadResolution(12);                 // 0..4095
  analogSetPinAttenuation(ADC_PIN, ADC_11db); // up to ~3.3V
}

void loop() {
  const uint32_t usPerSample = 1000000UL / FS;
  uint32_t t0 = micros();
  for (int i = 0; i < FRAME; i++) {
    int val = analogRead(ADC_PIN);     // ~2048 around mid-bias
    int16_t s = (int16_t)val - 2048;   // center to 0
    buf[i] = s << 4;                   // scale to int16 (rough gain)
    while ((micros() - t0) < (uint32_t)((i + 1) * usPerSample)) { /* wait */ }
  }
  udp.beginPacket(DEST_IP, DEST_PORT);
  udp.write((uint8_t*)buf, sizeof(buf));
  udp.endPacket();
}
