# Device Firmware

The paired hardware accessory speaks a vendor BLE protocol over GATT. Rather than develop
the app's device-control path against physical units, this directory holds a firmware
emulator that stands in for one: an ESP32 advertises under the same name and service UUIDs,
implements the protocol's command set, and streams the accelerometer reports and status
heartbeats the real device sends. The app cannot tell the difference, so the control path
and its tests run on a desk instead of a device.

`MLA2-BLE-Protocol.md` is a copy of the device protocol the emulator implements.
`meilang_simulator/` is the emulator itself. It reads an MPU-6050 over a bit-banged I2C
driver written against the GPIO pins rather than through the Arduino Wire library, which
keeps the timing and the bus recovery path explicit.

---

# Firmware Debug Notes

- `meilang_simulator/` is the BLE device-side simulator used for App discovery and connection.
- For App debugging, the computer must only provide power and firmware flashing.
- Do not run any PC-side BLE client scripts while testing App discovery/connection.
- The simulator must remain free to advertise so the iPhone App can scan and connect.

## Expected BLE identity

- Device name prefix: `MLA2-`
- Service UUID: `0000FFF0-0000-1000-8000-00805F9B34FB`
- Write characteristic: `0000FFF1-0000-1000-8000-00805F9B34FB`
- Notify characteristic: `0000FFF2-0000-1000-8000-00805F9B34FB`

## Repo layout

- Protocol copy: [MLA2-BLE-Protocol.md](./MLA2-BLE-Protocol.md)
- Simulator source: [meilang_simulator/meilang_simulator.ino](./meilang_simulator/meilang_simulator.ino)

## App-side discovery requirements

- The device must keep advertising over BLE.
- The advertised local name must remain non-empty.
- The device name must continue matching the App whitelist in `command.js`.
- After connection, the simulator waits for `CMD_HEARTBEAT` enable before emitting reports.
