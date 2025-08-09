#!/usr/bin/env python3
import socket, wave, struct, time

DEST_IP = "0.0.0.0"
DEST_PORT = 5005
FS = 8000

sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.bind((DEST_IP, DEST_PORT))
print(f"Listening on UDP {DEST_IP}:{DEST_PORT}... writing to capture.wav")

wf = wave.open("capture.wav", "wb")
wf.setnchannels(1)
wf.setsampwidth(2)  # int16
wf.setframerate(FS)

try:
    while True:
        data, _ = sock.recvfrom(4096)
        wf.writeframesraw(data)
except KeyboardInterrupt:
    pass
finally:
    wf.close()
    sock.close()
    print("Saved capture.wav")
