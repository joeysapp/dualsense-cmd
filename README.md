# DualSense-CMD

A high-performance, cross-platform CLI and GUI tool for mapping PlayStation DualSense controller inputs to shell commands, HTTP requests, and WebSocket messages.

## Features

- **Modern GUI**: Tauri-based visual interface for easy monitoring and configuration.
- **3D Visualization**: Real-time orientation and motion tracking using integrated spatial core.
- **WebSocket Streaming**: High-speed (60fps+) state streaming for games and interactive apps.
- **Custom Mappings**: Trigger shell commands or REST API calls from any button or stick movement.
- **Template Support**: Inject controller state into commands using Handlebars templates.

## Quickstart

### GUI (Tauri)
Launch the visual interface:
```bash
npm install
npm run tauri dev
```

### CLI
Install and use the command-line tool:
```bash
cargo install --path .

dualsense-cmd monitor   # Real-time state viewer
dualsense-cmd 3d        # 3D motion visualizer
dualsense-cmd run       # Run mapper with config
```

## CLI Reference

| Command | Description |
|---------|-------------|
| `list` | List connected DualSense controllers |
| `monitor` | Show controller state (supports `--json`, `--raw`) |
| `3d` | Open 3D visualization of orientation and motion |
| `run` | Execute input mappings defined in config |
| `init` | Generate a sample configuration file |
| `validate` | Check configuration file for errors |

## Known Issues

- **Rumble**: Currently nonfunctional in configurations and not callable via CLI.
- **LED Control**: Manual LED setting and monitoring via configuration is nonfunctional.

## Roadmap

- **Configuration Studio**: Visual editor for creating and managing mapping profiles.
- **Monitor Mode**: Act as a system-wide monitor allowing users to pipe output to other applications while maintaining a live view.
- **Advanced Response Curves**: Exponential and S-Curve support for fine-tuned stick control.
- **Macro Recorder**: Record input sequences and replay them as actions.

## License
MIT License - see [LICENSE](LICENSE) for details.
