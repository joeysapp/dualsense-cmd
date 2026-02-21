# Product Roadmap and Immediate Goals
**Full Exposure of DualSense APIs**:
  - [FIX] Rumble does not work in configurations
  - [REF] AxiDraw configuration is still not fluid - a bit jittery over HTTP, so for now:
    - [BUILD] Build a simple configuration and a simple Rust-based GUI application to let the user control a floating cube in 3D space as well as X/Y/Z rotations. Use typical gamepad designs for this attached to the new `Quaternion`.
  - Configurable settings through this package (colors, rumbles, all)
  
### Linear Algebra (Quaternions, Geometry, etc.) + 
Core Components of ideated lin alg library
  - [BUILD] `Quaternion` structs with fundamental and necessary operations for 3D rotations and controls using the 
  - `Linear Algebra` features for rendering and collision calcs
    - e.g. using the dot product of face normals to determine visibility of a surface in rendering
  - `Projections` of 3D objects onto 2D planes for rendering
	- Website canvases, SVGs, AxiDraw plotting
	- Calculating how 'far away' objects being projected should be to simulate 3D motion on 2D surface
  - `Particle` struct (position, velocity, acceleration, mass).
- Full DualSense API Utilization and Spatial Awareness
  - Real-world positioning using accelerometer and possible bluetooth latencies
  - Tilt-to-Wind: Map Controller Roll/Pitch -> Wind Vector.
  - Force Feedback: Rumble based on particle speed or collision.
  - Adaptive Triggers: Stiffen the triggers based on "tension" or "wind resistance".

### Feature Ideas
- Macro Recorder: Record a sequence of inputs and replay them (useful for drawing specific patterns).
- Response Curves: Allow users to map stick input to output using curves (Exponential, S-Curve) for finer control.
- Deadzone Visualizer: A TUI (Text User Interface) or simple web view to see the raw vs processed stick values.
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
