# TODOS
- Update monitor and renderer functionality
  - Rumble Visualizer: Add rumble information
  - Led Visualizer: See LED information
  - Deadzone Visualizer: See the raw vs processed stick values.  
- Add renderer documentation to README
- Add intent / clarity to README
- Full Exposure of DualSense Controller
  - [FIX] Rumble does not work in configurations, not callable. Same with LEDs.
- Macro Recorder: Record a sequence of inputs and replay them (useful for drawing specific patterns).
- Response Curves: Allow users to map stick input to output using curves (Exponential, S-Curve) for finer control.
  - [INFO] Part of the spatial-core integration currently
- Future Physics Simulation hooks:
  - Wind: A global force vector applied to all particles.
  - Gravity: Constant downward acceleration.
  - Drag/Friction: Opposing force proportional to velocity.
  - Springs: Hooke's Law for connecting particles (great for "soft" pen strokes).

### Community Recommended Crates
- `glam` or `nalgebra`: For vector math (you already use `nalgebra`!).
- `bevy`: If you want a full game engine later, but for now `macroquad` is easier for 2D visualization.
- `ratatui`: For building rich Terminal UIs (better than just printing text).

### CI/CD & Quality
- GitHub Actions: Set up a `.github/workflows/ci.yml` to run `cargo test` and `cargo clippy` on every push.
- Clippy: Run `cargo clippy` often. It teaches you idiomatic Rust.
- Formatting: Use `cargo fmt` to keep code consistent.
