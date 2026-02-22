import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import { listen } from "@tauri-apps/api/event";
import {
  AppShell,
  Text,
  Group,
  Stack,
  Button,
  Card,
  Badge,
  Grid,
  Progress,
  Slider,
  ColorPicker,
  Title,
  Paper,
  Box,
  Container,
} from "@mantine/core";
import { Canvas, useFrame } from "@react-three/fiber";
import { Box as ThreeBox, OrbitControls, Grid as ThreeGrid } from "@react-three/drei";
import * as THREE from "three";

interface ControllerInfo {
  index: number;
  product: string;
  serial: string;
  connection: string;
}

interface ControllerState {
  buttons: {
    cross: boolean;
    circle: boolean;
    square: boolean;
    triangle: boolean;
    l1: boolean;
    r1: boolean;
    l2_button: boolean;
    r2_button: boolean;
    dpad_up: boolean;
    dpad_down: boolean;
    dpad_left: boolean;
    dpad_right: boolean;
    l3: boolean;
    r3: boolean;
    options: boolean;
    create: boolean;
    ps: boolean;
    touchpad: boolean;
    mute: boolean;
  };
  left_stick: { x: number, y: number };
  right_stick: { x: number, y: number };
  triggers: { l2: number, r2: number };
  battery: { level: number, charging: boolean, fully_charged: boolean };
  touchpad: {
    finger1: { active: boolean, id: number, x: number, y: number };
    finger2: { active: boolean, id: number, x: number, y: number };
  };
  gyroscope: { x: number, y: number, z: number };
  accelerometer: { x: number, y: number, z: number };
  timestamp: number;
}

interface SpatialState {
  position: [number, number, number];
  velocity: [number, number, number];
  orientation: [number, number, number, number]; // w, x, y, z
}

function ControllerModel({ spatial }: { spatial: SpatialState }) {
  const meshRef = useRef<THREE.Mesh>(null);

  useFrame(() => {
    if (meshRef.current) {
      // Create quaternion from [w, x, y, z]
      const [w, x, y, z] = spatial.orientation;
      // In Three.js, quaternion is x, y, z, w
      meshRef.current.quaternion.set(x, y, z, w);
      
      // Update position (from mm to units, assuming 100mm = 1 unit)
      const [px, py, pz] = spatial.position;
      meshRef.current.position.set(px / 100, py / 100, pz / 100);
    }
  });

  return (
    <ThreeBox ref={meshRef} args={[1.6, 0.8, 0.4]}>
      <meshStandardMaterial color="#3399FF" />
    </ThreeBox>
  );
}

function App() {
  const [controllers, setControllers] = useState<ControllerInfo[]>([]);
  const [connected, setConnected] = useState(false);
  const [isTauri, setIsTauri] = useState(true);

  const [state, setState] = useState<ControllerState | null>(null);
  const [spatial, setSpatial] = useState<SpatialState>({
    position: [0, 0, 0],
    velocity: [0, 0, 0],
    orientation: [1, 0, 0, 0],
  });

  const [ledColor, setLedColor] = useState("#0080FF");
  const [rumbleLeft, setRumbleLeft] = useState(0);
  const [rumbleRight, setRumbleRight] = useState(0);

  useEffect(() => {
    // Check if we're in Tauri
    if (!(window as any).__TAURI__) {
      console.warn("Not running in Tauri environment. Tauri commands will not work.");
      setIsTauri(false);
      return;
    }

    const fetchControllers = async () => {
      try {
        console.log("Fetching controllers...");
        const list = await invoke<ControllerInfo[]>("list_controllers");
        console.log("Found controllers:", list);
        setControllers(list);
      } catch (e) {
        console.error("Failed to fetch controllers:", e);
      }
    };

    fetchControllers();
    const interval = setInterval(fetchControllers, 5000);

    // Listen for state updates
    const unlistenState = listen<ControllerState>("controller-state", (event) => {
      setState(event.payload);
      setConnected(true);
    });

    const unlistenSpatial = listen<SpatialState>("spatial-state", (event) => {
      setSpatial(event.payload);
    });

    const unlistenDisconnected = listen("controller-disconnected", () => {
      setConnected(false);
      setState(null);
    });

    return () => {
      clearInterval(interval);
      unlistenState.then(fn => fn());
      unlistenSpatial.then(fn => fn());
      unlistenDisconnected.then(fn => fn());
    };
  }, []);

  const handleConnect = async () => {
    try {
      await invoke("connect_controller");
    } catch (e) {
      console.error(e);
    }
  };

  const handleLedChange = (color: string) => {
    setLedColor(color);
    const r = parseInt(color.slice(1, 3), 16);
    const g = parseInt(color.slice(3, 5), 16);
    const b = parseInt(color.slice(5, 7), 16);
    invoke("set_led", { r, g, b });
  };

  const handleRumbleChange = (val: number, side: 'left' | 'right') => {
    if (side === 'left') setRumbleLeft(val);
    else setRumbleRight(val);
    
    invoke("set_rumble", { 
      left: side === 'left' ? val : rumbleLeft, 
      right: side === 'right' ? val : rumbleRight,
      duration_ms: 100
    });
  };

  const handleResetSpatial = async () => {
    try {
      await invoke("reset_spatial");
    } catch (e) {
      console.error(e);
    }
  };

  return (
    <AppShell
      padding="md"
      header={{ height: 60 }}
    >
      <AppShell.Header p="md">
        <Group justify="space-between">
          <Title order={3} c="blue">DualSense CMD GUI</Title>
          {!isTauri && <Badge color="red">Running in Browser</Badge>}
          {isTauri && (
            <Badge color={connected ? "green" : "red"} variant="filled">
              {connected ? "Connected" : "Disconnected"}
            </Badge>
          )}
        </Group>
      </AppShell.Header>

      <AppShell.Main>
        {!isTauri && (
          <Container py="xl">
            <Paper p="xl" withBorder shadow="md">
              <Stack align="center">
                <Title order={2} c="red">Tauri Not Detected</Title>
                <Text>This application needs access to your system's USB/Bluetooth hardware.</Text>
                <Text fw={700}>Please run the application using:</Text>
                <Paper p="xs" withBorder bg="dark.7">
                  <Text ff="monospace">npm run tauri dev</Text>
                </Paper>
                <Text size="sm" c="dimmed">The browser version only shows the UI and cannot connect to controllers.</Text>
              </Stack>
            </Paper>
          </Container>
        )}
        <Grid style={{ display: !isTauri ? 'none' : 'flex' }}>
          <Grid.Col span={4}>
            <Stack>
              <Paper p="md" withBorder>
                <Title order={5} mb="sm">Controllers</Title>
                {controllers.length === 0 ? (
                  <Text size="sm" c="dimmed">No controllers found</Text>
                ) : (
                  <Stack gap="xs">
                    {controllers.map(c => (
                      <Card key={c.serial} withBorder padding="sm">
                        <Text fw={500} size="sm">{c.product}</Text>
                        <Text size="xs" c="dimmed">{c.connection} - {c.serial}</Text>
                        {!connected && (
                          <Button size="compact-xs" mt="xs" onClick={handleConnect}>
                            Connect
                          </Button>
                        )}
                      </Card>
                    ))}
                  </Stack>
                )}
              </Paper>

              <Paper p="md" withBorder>
                <Title order={5} mb="sm">Controls</Title>
                <Stack gap="md">
                  <Box>
                    <Text size="sm" mb="xs">LED Color</Text>
                    <ColorPicker 
                      value={ledColor} 
                      onChange={handleLedChange}
                      format="hex"
                      fullWidth
                    />
                  </Box>
                  <Box>
                    <Text size="sm" mb="xs">Rumble Left: {rumbleLeft}</Text>
                    <Slider 
                      value={rumbleLeft} 
                      onChange={(v) => handleRumbleChange(v, 'left')} 
                      max={255} 
                    />
                  </Box>
                  <Box>
                    <Text size="sm" mb="xs">Rumble Right: {rumbleRight}</Text>
                    <Slider 
                      value={rumbleRight} 
                      onChange={(v) => handleRumbleChange(v, 'right')} 
                      max={255} 
                    />
                  </Box>
                </Stack>
              </Paper>

              {state && state.battery && (
                <Paper p="md" withBorder>
                  <Title order={5} mb="sm">Battery</Title>
                  <Group justify="space-between" mb="xs">
                    <Text size="sm">{Math.min((state.battery.level || 0) * 10, 100)}%</Text>
                    <Badge color={state.battery.charging ? "orange" : "blue"}>
                      {state.battery.charging ? "Charging" : "Discharging"}
                    </Badge>
                  </Group>
                  <Progress value={Math.min((state.battery.level || 0) * 10, 100)} color="green" />
                </Paper>
              )}
            </Stack>
          </Grid.Col>

          <Grid.Col span={8}>
            <Stack>
              <Paper p="md" withBorder h={400} style={{ position: 'relative', overflow: 'hidden' }}>
                <Group justify="space-between" mb="sm" style={{ position: 'absolute', zIndex: 10, width: 'calc(100% - 32px)' }}>
                  <Title order={5}>Spatial Monitor</Title>
                  <Button size="compact-xs" color="gray" variant="light" onClick={handleResetSpatial}>
                    Reset
                  </Button>
                </Group>
                <Canvas camera={{ position: [0, 2, 5] }}>
                  <ambientLight intensity={0.5} />
                  <pointLight position={[10, 10, 10]} />
                  <ControllerModel spatial={spatial} />
                  <ThreeGrid infiniteGrid />
                  <OrbitControls />
                </Canvas>
              </Paper>

              {state && (
                <Paper p="md" withBorder>
                  <Title order={5} mb="sm">Input Monitor</Title>
                  <Grid>
                    <Grid.Col span={6}>
                      <Text size="sm" fw={500}>Sticks</Text>
                      <Group mt="xs" gap="xl">
                        <Stack gap={0}>
                          <Text size="xs" c="dimmed">Left</Text>
                          <Text size="sm">X: {((state.left_stick.x - 128)/127).toFixed(2)}</Text>
                          <Text size="sm">Y: {((state.left_stick.y - 128)/127).toFixed(2)}</Text>
                        </Stack>
                        <Stack gap={0}>
                          <Text size="xs" c="dimmed">Right</Text>
                          <Text size="sm">X: {((state.right_stick.x - 128)/127).toFixed(2)}</Text>
                          <Text size="sm">Y: {((state.right_stick.y - 128)/127).toFixed(2)}</Text>
                        </Stack>
                      </Group>
                    </Grid.Col>
                    <Grid.Col span={6}>
                      <Text size="sm" fw={500}>Triggers</Text>
                      <Stack mt="xs" gap="xs">
                        <Box>
                          <Text size="xs" c="dimmed">L2: {(state.triggers.l2 / 255).toFixed(2)}</Text>
                          <Progress value={(state.triggers.l2 / 255) * 100} size="sm" />
                        </Box>
                        <Box>
                          <Text size="xs" c="dimmed">R2: {(state.triggers.r2 / 255).toFixed(2)}</Text>
                          <Progress value={(state.triggers.r2 / 255) * 100} size="sm" />
                        </Box>
                      </Stack>
                    </Grid.Col>
                  </Grid>
                </Paper>
              )}
            </Stack>
          </Grid.Col>
        </Grid>
      </AppShell.Main>
    </AppShell>
  );
}

export default App;
