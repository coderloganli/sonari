# MLA2 BLE Device Protocol

The protocol the paired accessory speaks, and the one the simulator in
`device_simulator/` implements.

> Command and field names here are generalized from the vendor's own. Opcodes, byte
> layouts, lengths and timings are unchanged.

## Basics

- **Device name** is `MLA2` followed by part of the MAC address.
- **Framing** is fixed layout. A packet is delimited by its length, and multi-packet
  payloads are assembled in order and checksummed.
- **Modes.** The device powers on in manual mode and switches to BLE mode once it accepts
  a command. Manual controls are inactive while a BLE session is open.

## Opcode index

| Opcode | Direction | Meaning |
|---|---|---|
| `0x01` | app to device | Query firmware version |
| `0x41` | app to device | Grip on or off |
| `0x42` | app to device | Set grip strength parameters |
| `0x43` | app to device | Query grip state |
| `0x44` | both | Reporting control, telemetry and status heartbeat |
| `0x45` | app to device | Set vibration |
| `0x46` | app to device | Set heater |
| `0x47` | app to device | Set grip parameters and level |
| `0x48` | app to device | Set manual vibration defaults |
| `0x49` | app to device | Set manual heater defaults |
| `0x4A` | app to device | Query manual vibration defaults |
| `0x4B` | app to device | Query manual heater defaults |
| `0x4C` | app to device | Force off |
| `0x4D` | app to device | Set linear actuator |
| `0x4E` | app to device | Set rotation |
| `0x4F` | app to device | Query MAC address |

Every command is acknowledged with a reply carrying the same opcode.

## Force off, `0x4C`

Request is the opcode alone. The device clears every actuator and replies with the opcode
alone.

## Query firmware version, `0x01`

| Field | Bytes | Notes |
|---|---|---|
| Opcode | 1 | `0x01` |
| CSN | 1 | Physical device identifier |
| Firmware version | 3 | Distinguishes production batches of the same device |
| Protocol stack version | 2 | |
| Hardware version | 2 | |

Used by firmware update: within the same hardware and protocol stack version, the firmware
version is what an update compares against.

## Query MAC address, `0x4F`

| Field | Bytes |
|---|---|
| Opcode | 1 |
| MAC address | 6 |

## Accelerometer report, `0x44`, device initiated

Sent every 100 ms while reporting is enabled.

| Field | Bytes | Notes |
|---|---|---|
| Opcode | 1 | `0x44` |
| Acceleration X | 2 | |
| Acceleration Y | 2 | |
| Acceleration Z | 2 | |
| Report count | 2 | Cumulative |
| Heater setpoint | 1 | Percent |
| Heater time remaining | 1 | Minutes |
| Vibration level | 1 | Percent |
| Grip level | 1 | 1 to 4 in manual mode, 0 when the app is in control |
| Status bits | 1 | bit0 heater on, bit1 vibration on, bit4 grip owner, 0 manual and 1 app |
| Linear actuator speed | 1 | 0 when there is nothing to report |
| Linear actuator mode | 1 | Automatic or manual |
| Orientation | 1 | 1 upright, 2 flat A, 3 flat B |
| Event flag | 1 | Session event marker |

## Status heartbeat, `0x44`, device initiated

Sent every 5 s. The app treats 6 to 12 consecutive missing heartbeats, that is 30 to 60
seconds of silence, as a lost connection.

| Field | Bytes | Notes |
|---|---|---|
| Opcode | 1 | `0x44` |
| Frame number | 1 | Wraps |
| Reporting flag | 1 | 0 disables reporting, non zero enables it |

## Notes

- Grip levels 1 to 4 apply only in manual mode. The field reads 0 once the app takes
  control.
- Heater time remaining shows the configured duration while the heater is off, and the
  remaining duration while it is on.
- Linear actuator speed replies 0 when no movement has been commanded.
- Percentage fields run 0 to 100, where 0 means off.
