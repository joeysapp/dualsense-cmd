Build a highly robust cross-platform CLI that will allow users to connect their PlayStation DualSense controllers to their computers (arm/intel macOS, linux machines) and have them:
- execute configurable shell commands on controller actions
- handle state (gyro, accelerometer, basic position using quaternions)

Use the most appropriate language, framework and libraries for the most reliable, fast and usable build possible (e.g. using golang/rust/cpp over reference nodejs usage if the benefits are worth it, be it performance, reliability or available libraries eg serialport.)

Two high-level examples of required behavior:
- Use a DualSense controller to fire various `curl` commands to interface with the ../axi-server/src/index.js server. Curl the endpoint/info to get available commands and build a simple up/down X/Y movement system. 
- [IMPORTANT] Use a DualSense controller to send commands over a websocket connection - performance here is key.

Build the CLI tool and provide these configurations in JSON files at ./config or otherwise. Know that this is will be served as a public package on github to be installed by others.

If stuck, look at the reference legacy learning project (./ref) to connect to a PlayStation DualSense controller and communicate with it over nodejs.
