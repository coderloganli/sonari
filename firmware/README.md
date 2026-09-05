# Device Firmware

The paired hardware accessory speaks a vendor BLE protocol over GATT. Rather than develop
the app's device-control path against physical units, this directory holds a firmware
emulator that stands in for one: an ESP32 advertises under the same name and service UUIDs,
implements the protocol's command set, and streams the accelerometer reports and status
heartbeats the real device sends. The app cannot tell the difference, so the control path
and its tests run on a desk instead of a device.

- `BLE-Protocol.md` is the device protocol the emulator implements. Command and field
  names are generalized from the vendor's own; opcodes, byte layouts and timings are not.
- `device_simulator/` is the emulator. It reads an MPU-6050 over a bit-banged I2C driver
  written against the GPIO pins rather than through the Arduino Wire library, which keeps
  the timing and the bus recovery path explicit.

## BLE identity

| | |
|---|---|
| Device name prefix | `MLA2-` |
| Service | `0000FFF0-0000-1000-8000-00805F9B34FB` |
| Write characteristic | `0000FFF1-0000-1000-8000-00805F9B34FB` |
| Notify characteristic | `0000FFF2-0000-1000-8000-00805F9B34FB` |

The device advertises continuously while disconnected. Once connected it stays quiet until
the app enables reporting with `0x44`, after which it sends an accelerometer report every
100 ms and a status heartbeat every 5 s.

## Running it

Flash `device_simulator/device_simulator.ino` to an ESP32 with an MPU-6050 on `SDA` 21 and
`SCL` 22. Serial runs at 115200 and logs the I2C probe result, received opcodes and
heartbeat sequence numbers.

While testing app discovery and connection, leave the emulator free to advertise and do not
run a second BLE client against it from the host. Two centrals competing for the same
peripheral is the most common reason a scan comes up empty.
