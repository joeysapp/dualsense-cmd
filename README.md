# dualsense-cmd
A fast, cross-platform CLI for mapping PlayStation DualSense controller inputs to shell commands, HTTP requests, and WebSocket messages.

## Features

- **Cross-platform**: Works on macOS (Intel & Apple Silicon) and Linux
- **Multiple output modes**:
  - Execute shell commands
  - Send HTTP requests (REST APIs)
  - Stream data over WebSockets (high performance)
- **Full controller support**:
  - All buttons (face, D-pad, shoulder, system)
  - Analog sticks with configurable deadzones
  - Triggers with analog values
  - Touchpad (2-finger tracking)
  - Gyroscope and accelerometer
  - Quaternion-based orientation tracking
- **Haptic feedback**: Control LED colors and rumble motors
- **Template support**: Use controller state in commands with Handlebars templates
- **Connection types**: USB and Bluetooth

## Installation

### From source (requires Rust)

```bash
# Clone the repository
git clone https://github.com/yourusername/dualsense-pipes.git
cd dualsense-pipes

# Build release binary (todo: have this provided by github release.)
cargo build --release

# Install to ~/.cargo/bin
cargo install --path .
```

### Pre-built binaries

(TODO) Download from the [Releases](https://github.com/yourusername/dualsense-pipes/releases) page.

### Platform-specific setup

#### macOS

No additional setup required. The binary works with both USB and Bluetooth connections.

#### Linux

Add udev rules for HID access without root:

```bash
# Create udev rule
sudo tee /etc/udev/rules.d/70-dualsense.rules << 'EOF'
# DualSense
KERNEL=="hidraw*", ATTRS{idVendor}=="054c", ATTRS{idProduct}=="0ce6", MODE="0666"
# DualSense Edge
KERNEL=="hidraw*", ATTRS{idVendor}=="054c", ATTRS{idProduct}=="0df2", MODE="0666"
EOF

# Reload rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

## Quick Start

```bash
# List connected controllers
dualsense-cmd list

# Monitor controller state in real-time
dualsense-cmd monitor

# Run with default configuration
dualsense-cmd run

# Run with a specific config file
dualsense-cmd run -c ./config/axidraw.json

# Generate a new configuration
dualsense-cmd init -p websocket -o ./my-config.json
```

## Usage

### Commands

| Command | Description |
|---------|-------------|
| `run` | Run the controller mapper with a configuration |
| `list` | List connected DualSense controllers |
| `monitor` | Show controller state in real-time |
| `init` | Generate a sample configuration file |
| `validate` | Validate a configuration file |
| `test-ws` | Test WebSocket connection |

### CLI Options

```
dualsense-cmd [OPTIONS] <COMMAND>

Options:
  -c, --config <PATH>    Configuration file or directory [default: ./config]
  -v, --verbose          Verbose output (use -vv for trace)
  -h, --help             Print help
  -V, --version          Print version
```

### Run Options

```
dualsense-cmd run [OPTIONS]

Options:
  -p, --profile <FILE>   Configuration file to use
      --dry-run          Show actions without executing
```

## Configuration

Configurations are JSON files that define how controller inputs map to actions.

### Basic Structure

```json
{
  "name": "My Configuration",
  "poll_rate": 100,
  "deadzone": 0.1,

  "buttons": {
    "cross": {
      "trigger": "press",
      "command": "echo 'Hello!'"
    }
  },

  "led": {
    "connected_color": { "r": 0, "g": 128, "b": 255 }
  }
}
```

### Action Types

#### Shell Command
```json
{
  "command": "curl -X POST http://api.example.com/action"
}
```

#### HTTP Request
```json
{
  "http": {
    "method": "POST",
    "path": "/move",
    "body": "{\"dx\": {{left_stick_x}}, \"dy\": {{left_stick_y}}}"
  }
}
```

#### WebSocket Message
```json
{
  "websocket": {
    "message": "{\"type\": \"button\", \"name\": \"cross\"}",
    "binary": false
  }
}
```

### Trigger Types

| Trigger | Description |
|---------|-------------|
| `press` | When button is pressed |
| `release` | When button is released |
| `hold` | While button is held |
| `change` | On any state change |

### Template Variables

All actions support Handlebars templates with these variables:
**TODO** Legacy implementation, needs to provide spatial state too

| Variable | Type | Description |
|----------|------|-------------|
| `cross`, `circle`, etc. | bool | Button states |
| `left_stick_x`, `left_stick_y` | float | Left stick (-1.0 to 1.0) |
| `right_stick_x`, `right_stick_y` | float | Right stick (-1.0 to 1.0) |
| `l2_trigger`, `r2_trigger` | float | Trigger values (0.0 to 1.0) |
| `roll`, `pitch`, `yaw` | float | Orientation in radians |
| `quat_w`, `quat_x`, `quat_y`, `quat_z` | float | Orientation quaternion |
| `gyro_x`, `gyro_y`, `gyro_z` | float | Angular velocity (rad/s) |
| `accel_x`, `accel_y`, `accel_z` | float | Acceleration (G) |
| `battery_percent` | int | Battery level (0-100) |
| `touch1_x`, `touch1_y`, `touch1_active` | mixed | First touch point |
| `touch2_x`, `touch2_y`, `touch2_active` | mixed | Second touch point |

### Feedback

#### Rumble
**TODO** Does not work
```json
{
  "rumble": {
    "left": 128,
    "right": 128,
    "duration_ms": 200
  }
}
```

#### LED Color
```json
{
  "led": { "r": 255, "g": 0, "b": 0 }
}
```

### Example Configurations

See the `config/` directory for complete examples:
- `example.json` - Basic shell commands
- `curl-commands.json` - REST API via curl commands

## WebSocket Streaming

For real-time applications, use WebSocket streaming at >16ms or less. This sends controller state at ~60fps, ideal for:
- Game input
- Real-time visualization
- Motion tracking
- Remote control applications

## Connecting Your Controller

### USB
Simply plug in your DualSense controller via USB-C cable.

### Bluetooth
1. Turn off the controller if it's on
2. Hold **Create** (left of touchpad) + **PS** button until the light bar flashes
3. Pair via your system's Bluetooth settings
4. The light bar will turn solid when connected

## Building from Source

### Prerequisites

- Rust 1.70+ (install via [rustup](https://rustup.rs/))
- Platform-specific:
  - **macOS**: Xcode Command Line Tools
  - **Linux**: `libudev-dev` and `libhidapi-dev`

### Build

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test
```

### Cross-compilation

```bash
# Install targets
rustup target add x86_64-apple-darwin    # Intel Mac
rustup target add aarch64-apple-darwin   # Apple Silicon
rustup target add x86_64-unknown-linux-gnu
rustup target add aarch64-unknown-linux-gnu

# Build for specific target
cargo build --release --target aarch64-apple-darwin
```

## Troubleshooting

### Controller connected but can't hear anything from it (macOS)

1. In System Settings, go to Bluetooth and click the grey `i` on the DS
2. Click Game Controller Settings
3. Click the blue text 'Identify'
4. Rerun the monitor and you should see your input now

### Controller not found

1. Ensure the controller is connected and powered on
2. Check if it appears in `dualsense-cmd list`
3. On Linux, verify udev rules are set up correctly
4. Try disconnecting and reconnecting

### Permission denied (Linux)

```bash
# Quick fix (temporary)
sudo chmod 666 /dev/hidraw*

# Permanent fix: set up udev rules (see installation section)
```

### Bluetooth connection drops

- Keep the controller within range (~10m)
- Reduce interference from other Bluetooth devices
- Try USB for more reliable connections

### High latency

- Increase `poll_rate` in config (up to 250)
- Use USB instead of Bluetooth
- Reduce `state_interval_ms` for WebSocket streaming

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contributing

Contributions welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## Acknowledgments

- [hidapi](https://github.com/libusb/hidapi) - Cross-platform HID library
- [pydualsense](https://github.com/flok/pydualsense) - Reference implementation
- Sony for the excellent DualSense controller
