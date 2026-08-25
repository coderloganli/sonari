/*
 * MLA2 Device Simulator
 * Protocol: MLA2-BLE V1
 *
 * BLE Service: 0000FFF0 | Write: 0000FFF1 | Notify: 0000FFF2
 * Device name: MLA2-XXXX (last 4 of MAC)
 * MPU6050 via bit-bang I2C: SDA=21, SCL=22
 *
 * Behavior:
 *   Not connected : advertise continuously
 *   Connected     : wait for CMD_HEARTBEAT enable, then
 *                   send accel report every 100ms (Sec.16)
 *                   send status heartbeat every 5s  (Sec.17)
 */

#include <BLEDevice.h>
#include <BLEServer.h>
#include <BLEUtils.h>
#include <BLE2902.h>

// ============================================================
// Bit-bang I2C / MPU6050
// ============================================================

#define SDA_PIN  21
#define SCL_PIN  22
#define MPU_ADDR 0x68
#define I2C_DELAY 5
#define MPU_WHO_AM_I_REG 0x75
#define MPU_PWR_MGMT_1_REG 0x6B
#define MPU_ACCEL_CONFIG_REG 0x1C
#define MPU_ACCEL_XOUT_H_REG 0x3B
#define MPU_EXPECTED_WHO_AM_I 0x68
#define ACCEL_DEBUG_INTERVAL_MS 1000

void i2c_init() {
    pinMode(SDA_PIN, INPUT_PULLUP);
    pinMode(SCL_PIN, INPUT_PULLUP);
    delayMicroseconds(100);
}

void sda_high() { pinMode(SDA_PIN, INPUT_PULLUP); }
void sda_low()  { pinMode(SDA_PIN, OUTPUT); digitalWrite(SDA_PIN, LOW); }
void scl_high() { pinMode(SCL_PIN, INPUT_PULLUP); delayMicroseconds(I2C_DELAY); }
void scl_low()  { pinMode(SCL_PIN, OUTPUT); digitalWrite(SCL_PIN, LOW); delayMicroseconds(I2C_DELAY); }
void i2c_start() { sda_high(); scl_high(); sda_low(); delayMicroseconds(I2C_DELAY); scl_low(); }
void i2c_stop()  { sda_low(); scl_high(); sda_high(); delayMicroseconds(I2C_DELAY); }

bool i2c_write_byte(uint8_t b) {
    for (int i = 7; i >= 0; i--) { if (b & (1 << i)) sda_high(); else sda_low(); scl_high(); scl_low(); }
    sda_high(); scl_high(); bool ack = (digitalRead(SDA_PIN) == LOW); scl_low(); return ack;
}

uint8_t i2c_read_byte(bool ack) {
    uint8_t b = 0; sda_high();
    for (int i = 7; i >= 0; i--) { scl_high(); if (digitalRead(SDA_PIN)) b |= (1 << i); scl_low(); }
    if (ack) sda_low(); else sda_high(); scl_high(); scl_low(); sda_high(); return b;
}

void i2c_recover() {
    pinMode(SDA_PIN, INPUT_PULLUP); pinMode(SCL_PIN, INPUT_PULLUP);
    for (int i = 0; i < 16; i++) { scl_low(); scl_high(); if (digitalRead(SDA_PIN)) break; }
    i2c_stop();
}

bool mpu_write(uint8_t reg, uint8_t val) {
    i2c_start();
    if (!i2c_write_byte(MPU_ADDR << 1)) { i2c_stop(); return false; }
    i2c_write_byte(reg); i2c_write_byte(val); i2c_stop(); return true;
}

bool mpu_read(uint8_t reg, uint8_t* buf, uint8_t len) {
    i2c_start();
    if (!i2c_write_byte(MPU_ADDR << 1)) { i2c_stop(); return false; }
    i2c_write_byte(reg);
    i2c_start();
    if (!i2c_write_byte((MPU_ADDR << 1) | 1)) { i2c_stop(); return false; }
    for (uint8_t i = 0; i < len; i++) buf[i] = i2c_read_byte(i < len - 1);
    i2c_stop(); return true;
}

bool mpuReady = false;
unsigned long lastAccelDebugMs = 0;
uint32_t mpuReadFailureCount = 0;

bool mpu_read_u8(uint8_t reg, uint8_t* value) {
    return mpu_read(reg, value, 1);
}

void mpuInit() {
    i2c_init();
    i2c_recover();
    i2c_start();
    bool ack = i2c_write_byte(MPU_ADDR << 1);
    i2c_stop();
    if (!ack) {
        Serial.println("[MPU] Not found");
        return;
    }

    uint8_t whoAmI = 0;
    if (!mpu_read_u8(MPU_WHO_AM_I_REG, &whoAmI)) {
        Serial.println("[MPU] WHO_AM_I read failed");
        return;
    }
    Serial.printf("[MPU] WHO_AM_I=0x%02X\n", whoAmI);
    if (whoAmI != MPU_EXPECTED_WHO_AM_I) {
        Serial.println("[MPU] Unexpected WHO_AM_I");
        return;
    }

    if (!mpu_write(MPU_PWR_MGMT_1_REG, 0x80)) {
        Serial.println("[MPU] Reset write failed");
        return;
    }
    delay(150);
    if (!mpu_write(MPU_PWR_MGMT_1_REG, 0x00)) {
        Serial.println("[MPU] Wake write failed");
        return;
    }
    delay(100);
    if (!mpu_write(0x6C, 0x00)) {
        Serial.println("[MPU] PWR_MGMT_2 write failed");
        return;
    }
    if (!mpu_write(MPU_ACCEL_CONFIG_REG, 0x08)) {
        Serial.println("[MPU] ACCEL_CONFIG write failed");
        return;
    }

    uint8_t pwrMgmt1 = 0xFF;
    if (!mpu_read_u8(MPU_PWR_MGMT_1_REG, &pwrMgmt1)) {
        Serial.println("[MPU] PWR_MGMT_1 verify failed");
        return;
    }
    Serial.printf("[MPU] PWR_MGMT_1=0x%02X\n", pwrMgmt1);
    mpuReady = true;
    Serial.println("[MPU] OK");
}

bool mpuReadAccel(int16_t* ax, int16_t* ay, int16_t* az) {
    if (!mpuReady) {
        return false;
    }

    uint8_t buf[6] = {0};
    if (!mpu_read(MPU_ACCEL_XOUT_H_REG, buf, 6)) {
        mpuReadFailureCount++;
        if (mpuReadFailureCount <= 5 || (mpuReadFailureCount % 20) == 0) {
            Serial.printf("[MPU] accel read failed count=%lu\n", (unsigned long)mpuReadFailureCount);
        }
        return false;
    }

    *ax = (buf[0] << 8) | buf[1];
    *ay = (buf[2] << 8) | buf[3];
    *az = (buf[4] << 8) | buf[5];
    return true;
}

// ============================================================
// BLE
// ============================================================

#define SERVICE_UUID     "0000FFF0-0000-1000-8000-00805F9B34FB"
#define CHAR_WRITE_UUID  "0000FFF1-0000-1000-8000-00805F9B34FB"
#define CHAR_NOTIFY_UUID "0000FFF2-0000-1000-8000-00805F9B34FB"

char deviceName[16] = "MLA2-0000";

// ============================================================
// Command codes
// ============================================================

#define CMD_VERSION     0x01
#define CMD_GRIP_SWITCH 0x41
#define CMD_GRIP_PARAMS 0x42
#define CMD_QUERY_GRIP  0x43
#define CMD_HEARTBEAT   0x44
#define CMD_VIBRATION   0x45
#define CMD_TEMPERATURE 0x46
#define CMD_SET_GRIP    0x47
#define CMD_SET_VIB     0x48
#define CMD_SET_HEAT    0x49
#define CMD_QUERY_VIB   0x4A
#define CMD_QUERY_HEAT  0x4B
#define CMD_FORCE_OFF   0x4C
#define CMD_LINEAR      0x4D
#define CMD_ROTATION    0x4E
#define CMD_MAC_ADDR    0x4F

// ============================================================
// Device state
// ============================================================

struct DeviceState {
    uint8_t  gripSwitch         = 0;
    uint16_t gripWorkTime       = 0;
    uint16_t gripParams[4]      = {0};
    uint8_t  gripGear           = 0;
    uint8_t  vibStrength        = 0;
    uint8_t  vibDuration        = 0;
    uint8_t  vibManualStrength  = 0;
    uint8_t  vibManualGear      = 0;
    uint8_t  tempSetting        = 0;
    uint8_t  tempDuration       = 20;
    uint8_t  heatLeft           = 0;
    uint8_t  heatManualTemp     = 0;
    uint8_t  heatManualDuration = 0;
    uint8_t  heatManualGear     = 0;
    uint8_t  linearGear         = 0;
    uint8_t  linearDuration     = 0;
    uint8_t  rotGear            = 0;
    uint8_t  rotDuration        = 0;
    uint8_t  orientation        = 1;
    uint8_t  eventFlag          = 0;
    uint16_t reportCount        = 0;
    uint8_t  heartbeatSeq       = 0;
} state;

bool deviceConnected    = false;
bool needRestartAdv     = false;
bool reportingEnabled   = false;
unsigned long lastAccelMs     = 0;
unsigned long lastHeartbeatMs = 0;

BLEServer*         pServer     = NULL;
BLECharacteristic* pNotifyChar = NULL;

// ============================================================
// Notify helper
// ============================================================

void sendNotify(const uint8_t* data, size_t len) {
    if (!deviceConnected || !pNotifyChar) return;
    pNotifyChar->setValue((uint8_t*)data, len);
    pNotifyChar->notify();
}

// ============================================================
// Section 16: Accelerometer report (18 bytes, every 100ms)
// ============================================================

void sendAccelReport() {
    int16_t ax = 0;
    int16_t ay = 0;
    int16_t az = 0;
    if (!mpuReadAccel(&ax, &ay, &az)) {
        return;
    }

    state.reportCount++;

    unsigned long now = millis();
    if (now - lastAccelDebugMs >= ACCEL_DEBUG_INTERVAL_MS) {
        lastAccelDebugMs = now;
        Serial.printf("[ACCEL] x=%d y=%d z=%d count=%u\n", ax, ay, az, state.reportCount);
    }

    uint8_t pkt[18];
    pkt[0]  = CMD_HEARTBEAT;
    pkt[1]  = (uint8_t)(ax >> 8);   pkt[2]  = (uint8_t)(ax & 0xFF);
    pkt[3]  = (uint8_t)(ay >> 8);   pkt[4]  = (uint8_t)(ay & 0xFF);
    pkt[5]  = (uint8_t)(az >> 8);   pkt[6]  = (uint8_t)(az & 0xFF);
    pkt[7]  = (uint8_t)(state.reportCount >> 8);
    pkt[8]  = (uint8_t)(state.reportCount & 0xFF);
    pkt[9]  = state.tempSetting;
    pkt[10] = state.heatLeft;
    pkt[11] = state.vibStrength;
    pkt[12] = state.gripGear;
    pkt[13] = (state.tempSetting > 0 ? 0x01 : 0)
            | (state.vibStrength > 0 ? 0x02 : 0)
            | (state.gripSwitch  > 0 ? 0x10 : 0);
    pkt[14] = state.linearGear;
    pkt[15] = state.linearDuration;
    pkt[16] = state.orientation;
    pkt[17] = state.eventFlag;
    sendNotify(pkt, 18);
}

// ============================================================
// Section 17: Status heartbeat (3 bytes, every 5s)
// ============================================================

void sendStatusHeartbeat() {
    state.heartbeatSeq++;
    uint8_t pkt[3] = { CMD_HEARTBEAT, state.heartbeatSeq, 1 };
    sendNotify(pkt, 3);
    Serial.printf("[HB] seq=%u\n", state.heartbeatSeq);
}

void resetReportingState() {
    reportingEnabled   = false;
    state.reportCount  = 0;
    state.heartbeatSeq = 0;
    lastAccelMs        = millis();
    lastHeartbeatMs    = millis();
}

void setHeartbeatEnabled(bool enabled) {
    reportingEnabled   = enabled;
    state.heartbeatSeq = 0;
    lastAccelMs        = millis();
    lastHeartbeatMs    = millis();
    Serial.printf("[HB] reporting=%s\n", enabled ? "on" : "off");
}

// ============================================================
// Command handler
// ============================================================

void handleCommand(const uint8_t* data, size_t len) {
    if (len == 0) return;
    uint8_t cmd = data[0];
    Serial.printf("[RX] cmd=0x%02X len=%u\n", cmd, (unsigned)len);

    switch (cmd) {
    case CMD_HEARTBEAT: {
        if (len >= 3) {
            setHeartbeatEnabled(data[2] != 0);
            Serial.printf("[HB] control seq=%u enable=%u\n", data[1], data[2]);
        } else if (len >= 2) {
            setHeartbeatEnabled(data[1] != 0);
            Serial.printf("[HB] legacy enable=%u\n", data[1]);
        } else {
            Serial.println("[WARN] Invalid CMD_HEARTBEAT payload");
        }
        break;
    }

    case CMD_GRIP_SWITCH: {
        if (len >= 2) state.gripSwitch = data[1];
        if (len >= 4) state.gripWorkTime = data[2] | (data[3] << 8);
        state.gripGear = (state.gripSwitch > 0) ? 1 : 0;
        uint8_t ack[4] = { CMD_GRIP_SWITCH, state.gripSwitch,
            (uint8_t)(state.gripWorkTime & 0xFF), (uint8_t)(state.gripWorkTime >> 8) };
        sendNotify(ack, 4);
        break;
    }
    case CMD_GRIP_PARAMS: {
        for (int i = 0; i < 4 && (1 + i*2 + 1) < (int)len; i++)
            state.gripParams[i] = data[1 + i*2] | (data[2 + i*2] << 8);
        uint8_t ack[9] = { CMD_GRIP_PARAMS };
        for (int i = 0; i < 4; i++) {
            ack[1 + i*2] = state.gripParams[i] & 0xFF;
            ack[2 + i*2] = (state.gripParams[i] >> 8) & 0xFF;
        }
        sendNotify(ack, 9);
        break;
    }
    case CMD_VIBRATION: {
        if (len >= 2) state.vibStrength = data[1];
        if (len >= 3) state.vibDuration = data[2];
        uint8_t ack[3] = { CMD_VIBRATION, state.vibStrength, state.vibDuration };
        sendNotify(ack, 3);
        break;
    }
    case CMD_TEMPERATURE: {
        if (len >= 2) state.tempSetting = data[1];
        if (len >= 3) state.tempDuration = data[2];
        state.heatLeft = state.tempDuration;
        uint8_t ack[3] = { CMD_TEMPERATURE, state.tempSetting, state.tempDuration };
        sendNotify(ack, 3);
        break;
    }
    case CMD_SET_GRIP: {
        for (int i = 0; i < 4 && (1 + i*2 + 1) < (int)len; i++)
            state.gripParams[i] = data[1 + i*2] | (data[2 + i*2] << 8);
        if (len >= 10) state.gripGear = data[9];
        uint8_t ack[10] = { CMD_SET_GRIP };
        for (int i = 0; i < 4; i++) {
            ack[1 + i*2] = state.gripParams[i] & 0xFF;
            ack[2 + i*2] = (state.gripParams[i] >> 8) & 0xFF;
        }
        ack[9] = state.gripGear;
        sendNotify(ack, 10);
        break;
    }
    case CMD_SET_VIB: {
        if (len >= 2) state.vibManualStrength = data[1];
        if (len >= 3) state.vibManualGear = data[2];
        uint8_t ack[3] = { CMD_SET_VIB, state.vibManualStrength, state.vibManualGear };
        sendNotify(ack, 3);
        break;
    }
    case CMD_SET_HEAT: {
        if (len >= 2) state.heatManualTemp = data[1];
        if (len >= 3) state.heatManualDuration = data[2];
        if (len >= 4) state.heatManualGear = data[3];
        uint8_t ack[4] = { CMD_SET_HEAT, state.heatManualTemp, state.heatManualDuration, state.heatManualGear };
        sendNotify(ack, 4);
        break;
    }
    case CMD_QUERY_GRIP: {
        uint8_t ack[10] = { CMD_QUERY_GRIP };
        for (int i = 0; i < 4; i++) {
            ack[1 + i*2] = state.gripParams[i] & 0xFF;
            ack[2 + i*2] = (state.gripParams[i] >> 8) & 0xFF;
        }
        ack[9] = state.gripGear;
        sendNotify(ack, 10);
        break;
    }
    case CMD_QUERY_VIB: {
        uint8_t ack[3] = { CMD_QUERY_VIB, state.vibManualStrength, state.vibManualGear };
        sendNotify(ack, 3);
        break;
    }
    case CMD_QUERY_HEAT: {
        uint8_t ack[4] = { CMD_QUERY_HEAT, state.heatManualTemp, state.heatManualDuration, state.heatManualGear };
        sendNotify(ack, 4);
        break;
    }
    case CMD_FORCE_OFF: {
        state.vibStrength  = 0;
        state.tempSetting  = 0;
        state.gripSwitch  = 0;
        state.gripGear    = 0;
        state.linearGear      = 0;
        state.rotGear      = 0;
        uint8_t ack[1] = { CMD_FORCE_OFF };
        sendNotify(ack, 1);
        Serial.println("[STATE] Force OFF");
        break;
    }
    case CMD_LINEAR: {
        if (len >= 2) state.linearGear = data[1];
        if (len >= 3) state.linearDuration = data[2];
        uint8_t ack[3] = { CMD_LINEAR, state.linearGear, state.linearDuration };
        sendNotify(ack, 3);
        break;
    }
    case CMD_ROTATION: {
        if (len >= 2) state.rotGear = data[1];
        if (len >= 3) state.rotDuration = data[2];
        uint8_t ack[3] = { CMD_ROTATION, state.rotGear, state.rotDuration };
        sendNotify(ack, 3);
        break;
    }
    case CMD_VERSION: {
        uint8_t ack[9] = { CMD_VERSION, 0x01, 1, 0, 2, 0, 1, 1, 0 };
        sendNotify(ack, 9);
        break;
    }
    case CMD_MAC_ADDR: {
        const uint8_t* mac = BLEDevice::getAddress().getNative();
        uint8_t ack[7] = { CMD_MAC_ADDR };
        memcpy(ack + 1, mac, 6);
        sendNotify(ack, 7);
        break;
    }
    default:
        Serial.printf("[WARN] Unknown cmd 0x%02X\n", cmd);
        break;
    }
}

// ============================================================
// BLE callbacks
// ============================================================

class ServerCB : public BLEServerCallbacks {
    void onConnect(BLEServer* s) {
        deviceConnected = true;
        resetReportingState();
        Serial.println("[BLE] Connected");
    }
    void onDisconnect(BLEServer* s) {
        deviceConnected = false;
        needRestartAdv  = true;
        resetReportingState();
        Serial.println("[BLE] Disconnected — will restart advertising");
    }
};

class WriteCB : public BLECharacteristicCallbacks {
    void onWrite(BLECharacteristic* c) {
        String v = c->getValue();
        if (v.length() > 0)
            handleCommand((const uint8_t*)v.c_str(), v.length());
    }
};

// ============================================================
// Setup
// ============================================================

void setup() {
    Serial.begin(115200);
    delay(300);

    mpuInit();

    BLEDevice::init("temp");
    const uint8_t* mac = BLEDevice::getAddress().getNative();
    snprintf(deviceName, sizeof(deviceName), "MLA2-%02X%02X", mac[1], mac[0]);
    BLEDevice::deinit(false);
    delay(100);

    BLEDevice::init(deviceName);
    pServer = BLEDevice::createServer();
    pServer->setCallbacks(new ServerCB());

    BLEService* svc = pServer->createService(SERVICE_UUID);
    BLECharacteristic* wc = svc->createCharacteristic(CHAR_WRITE_UUID,
        BLECharacteristic::PROPERTY_WRITE | BLECharacteristic::PROPERTY_WRITE_NR);
    wc->setCallbacks(new WriteCB());
    pNotifyChar = svc->createCharacteristic(CHAR_NOTIFY_UUID,
        BLECharacteristic::PROPERTY_NOTIFY | BLECharacteristic::PROPERTY_INDICATE);
    pNotifyChar->addDescriptor(new BLE2902());
    svc->start();

    BLEDevice::getAdvertising()->addServiceUUID(SERVICE_UUID);
    BLEDevice::getAdvertising()->setScanResponse(true);
    BLEDevice::startAdvertising();

    Serial.printf("\n=== MLA2 Simulator — %s ===\n", deviceName);
    Serial.println("[BLE] Advertising...");
}

// ============================================================
// Loop
// ============================================================

void loop() {
    if (needRestartAdv) {
        needRestartAdv = false;
        delay(200);
        pServer->startAdvertising();
        Serial.println("[BLE] Advertising...");
    }

    if (!deviceConnected) {
        delay(10);
        return;
    }

    if (!reportingEnabled) {
        delay(10);
        return;
    }

    unsigned long now = millis();

    if (now - lastAccelMs >= 100) {
        lastAccelMs = now;
        sendAccelReport();
    }

    if (now - lastHeartbeatMs >= 5000) {
        lastHeartbeatMs = now;
        sendStatusHeartbeat();
    }

    delay(1);
}
