import objc, datetime
from CoreBluetooth import (
    CBCentralManager,
    CBPeripheral,
)
 
class BLEScanner:
    def __init__(self):
        self.manager = CBCentralManager.alloc().initWithDelegate_queue_options_(self, None, None)
        self.devices = set()  # Avoid duplicates
 
    def centralManagerDidUpdateState_(self, manager):
        """Callback when Bluetooth state changes (e.g., powered on)."""
        if manager.state() == CBCentralManager.State.PoweredOn:
            print("Scanning for BLE devices... (Press Ctrl+C to stop)")
            manager.scanForPeripheralsWithServices_options_(None, None)  # Scan all services
        else:
            print(f"Bluetooth not available. State: {manager.state()}")
 
    def centralManager_didDiscoverPeripheral_advertisementData_RSSI_(self, manager, peripheral, adv_data, rssi):
        """Callback when a BLE device is discovered."""
        device_name = peripheral.name() or "Unknown"
        device_uuid = peripheral.identifier().UUIDString()  # Unique device ID
        print(f"Found: {device_name} (UUID: {device_uuid}, RSSI: {rssi})")
        self.devices.add((device_name, device_uuid))
 
if __name__ == "__main__":
    scanner = BLEScanner()
    try:
        # Run the event loop to listen for discoveries
        import AppKit
        AppKit.NSRunLoop.currentRunLoop().runUntilDate_(datetime.date(2028,1,1))
    except KeyboardInterrupt:
        print("\nScan stopped. Discovered devices:")
        for name, uuid in scanner.devices:
            print(f"- {name}: {uuid}")