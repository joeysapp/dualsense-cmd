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
	Tabs,
	Select,
	TextInput,
	SegmentedControl,
	Accordion,
	NumberInput,
	Divider,
	Modal,
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
	mode: string;
	position: [number, number, number];
	velocity: [number, number, number];
	linear_accel: [number, number, number];
	angular_velocity: [number, number, number];
	orientation: [number, number, number, number];
}

interface ProfileInfo {
	id: string;
	name: string;
	description: string;
}

interface Profile {
	name: string;
	description: string;
	led_color: { r: number; g: number; b: number };
	lightbar_enabled: boolean;
	l2_trigger: TriggerConfig;
	r2_trigger: TriggerConfig;
	player_leds?: number | { led1: boolean; led2: boolean; led3: boolean; led4: boolean; led5: boolean };
	mute_led?: string;
	rumble_intensity: number;
}

interface TriggerConfig {
	effect_type: string;
	start: number;
	end: number;
	force: number;
	frequency: number;
}

interface FeatureInfo {
	name: string;
	category: string;
	status: string;
	description: string;
}

const SPATIAL_MODES = [
	{ value: "Standard", label: "Standard (Stick X/Y, Triggers Z)" },
	{ value: "Heading", label: "Heading (Gyro Rotate, Triggers Fwd/Back)" },
	{ value: "Accelerometer", label: "Accelerometer (Motion-based position)" },
	{ value: "AxiDraw", label: "AxiDraw (R-Stick X/Y, L-Stick Force, Triggers Z)" },
	{ value: "ThreeD", label: "3D Tool (Common 3D navigation)" },
];

function ControllerModel({ spatial }: { spatial: SpatialState }) {
	const meshRef = useRef<THREE.Mesh>(null);

	useFrame(() => {
		if (meshRef.current) {
			const [w, x, y, z] = spatial.orientation;
			meshRef.current.quaternion.set(x, y, z, w);
			const [px, py, pz] = spatial.position;
			meshRef.current.position.set(px / 100, py / 100, pz / 100);
		}
	});

	return (
		<ThreeBox ref={meshRef} args={[1.6, 0.4, 1.0]}>
			<meshNormalMaterial />
		</ThreeBox>
	);
}

function SpatialInfo({ spatial }: { spatial: SpatialState }) {
	return (
		<Paper p="xs" withBorder bg="dark.7" style={{ opacity: 0.9 }}>
			<Stack gap={4}>
				<Text size="xs" ff="monospace" c="blue.4">
					POS: [{spatial.position.map(v => v.toFixed(2)).join(", ")}]
				</Text>
				<Text size="xs" ff="monospace" c="green.4">
					VEL: [{spatial.velocity.map(v => v.toFixed(2)).join(", ")}]
				</Text>
				<Text size="xs" ff="monospace" c="orange.4">
					ACC: [{spatial.linear_accel.map(v => v.toFixed(2)).join(", ")}]
				</Text>
				<Text size="xs" ff="monospace" c="purple.4">
					GYR: [{spatial.angular_velocity.map(v => v.toFixed(2)).join(", ")}]
				</Text>
				<Text size="xs" ff="monospace" c="dimmed">
					ORI: [{spatial.orientation.map(v => v.toFixed(2)).join(", ")}]
				</Text>
			</Stack>
		</Paper>
	);
}

const TRIGGER_EFFECTS = [
	{ value: "off", label: "Off" },
	{ value: "continuous", label: "Continuous" },
	{ value: "section", label: "Section" },
	{ value: "vibration", label: "Vibration" },
	{ value: "weapon", label: "Weapon" },
	{ value: "bow", label: "Bow" },
];

function rgbToHex(r: number, g: number, b: number): string {
	return "#" + [r, g, b].map(x => x.toString(16).padStart(2, '0')).join('');
}

function hexToRgb(hex: string): { r: number; g: number; b: number } {
	const r = parseInt(hex.slice(1, 3), 16);
	const g = parseInt(hex.slice(3, 5), 16);
	const b = parseInt(hex.slice(5, 7), 16);
	return { r, g, b };
}

function App() {
	const [controllers, setControllers] = useState<ControllerInfo[]>([]);
	const [connected, setConnected] = useState(false);
	const [isTauri, setIsTauri] = useState(true);

	const [state, setState] = useState<ControllerState | null>(null);
	const [spatial, setSpatial] = useState<SpatialState>({
		mode: "Standard",
		position: [0, 0, 0],
		velocity: [0, 0, 0],
		linear_accel: [0, 0, 0],
		angular_velocity: [0, 0, 0],
		orientation: [1, 0, 0, 0],
	});

	const controlsRef = useRef<any>(null);

	// Current controller settings (editable)
	const [ledColor, setLedColor] = useState("#0080FF");
	const [playerNumber, setPlayerNumber] = useState(1);
	const [l2Effect, setL2Effect] = useState<TriggerConfig>({ effect_type: "off", start: 70, end: 160, force: 200, frequency: 10 });
	const [r2Effect, setR2Effect] = useState<TriggerConfig>({ effect_type: "off", start: 70, end: 160, force: 200, frequency: 10 });

	// Rumble (immediate feedback, not saved to profile)
	const [rumbleLeft, setRumbleLeft] = useState(0);
	const [rumbleRight, setRumbleRight] = useState(0);

	// Profiles
	const [profiles, setProfiles] = useState<ProfileInfo[]>([]);
	const [selectedProfileId, setSelectedProfileId] = useState<string | null>(null);
	const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false);

	// Save profile modal
	const [saveModalOpen, setSaveModalOpen] = useState(false);
	const [newProfileName, setNewProfileName] = useState("");
	const [newProfileDesc, setNewProfileDesc] = useState("");

	// Features
	const [features, setFeatures] = useState<FeatureInfo[]>([]);

	// Active tab
	const [activeTab, setActiveTab] = useState<string | null>("monitor");

	// Sync status
	const [syncing, setSyncing] = useState(false);
	const [lastSyncTime, setLastSyncTime] = useState<Date | null>(null);

	useEffect(() => {
		if (!(window as any).__TAURI__) {
			setIsTauri(false);
			return;
		}

		const fetchControllers = async () => {
			try {
				const list = await invoke<ControllerInfo[]>("list_controllers");
				setControllers(list);
			} catch (e) {
				console.error("Failed to fetch controllers:", e);
			}
		};

		const fetchProfiles = async () => {
			try {
				const list = await invoke<ProfileInfo[]>("list_profiles");
				setProfiles(list);
			} catch (e) {
				console.error("Failed to fetch profiles:", e);
			}
		};

		const fetchFeatures = async () => {
			try {
				const list = await invoke<FeatureInfo[]>("get_features");
				setFeatures(list);
			} catch (e) {
				console.error("Failed to fetch features:", e);
			}
		};

		fetchControllers();
		fetchProfiles();
		fetchFeatures();

		const interval = setInterval(fetchControllers, 5000);

		const unlistenState = listen<ControllerState>("controller-state", (event) => {
			setState(event.payload);
			setConnected(true);
		});

		const unlistenSpatial = listen<SpatialState>("spatial-state", (event) => {
			setSpatial(event.payload);
		});

		const unlistenResetCamera = listen("reset-camera", () => {
			if (controlsRef.current) {
				controlsRef.current.reset();
			}
		});

		const unlistenDisconnected = listen("controller-disconnected", () => {
			setConnected(false);
			setState(null);
		});

		return () => {
			clearInterval(interval);
			unlistenState.then(fn => fn());
			unlistenSpatial.then(fn => fn());
			unlistenResetCamera.then(fn => fn());
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

	const handleResetSpatial = async () => {
		try {
			await invoke("reset_spatial");
		} catch (e) {
			console.error(e);
		}
	};

	const handleSetSpatialMode = async (mode: string | null) => {
		if (!mode) return;
		try {
			await invoke("set_spatial_mode", { mode });
		} catch (e) {
			console.error(e);
		}
	};

	// Load profile values into UI
	const handleSelectProfile = async (profileId: string | null) => {
		if (!profileId) return;

		try {
			const profile = await invoke<Profile>("get_profile", { name: profileId });

			// Load values into UI state
			setLedColor(rgbToHex(profile.led_color.r, profile.led_color.g, profile.led_color.b));
			setL2Effect(profile.l2_trigger);
			setR2Effect(profile.r2_trigger);

			// Handle player LEDs (can be number or custom object)
			if (typeof profile.player_leds === 'number') {
				setPlayerNumber(profile.player_leds);
			} else if (profile.player_leds) {
				// Custom pattern - default to 1 for simplicity
				setPlayerNumber(1);
			}

			setSelectedProfileId(profileId);
			setHasUnsavedChanges(false);
		} catch (e) {
			console.error("Failed to load profile:", e);
		}
	};

	// Mark as having unsaved changes when any setting is modified
	const handleSettingChange = () => {
		setHasUnsavedChanges(true);
	};

	// Apply all current settings to controller
	const handleSyncToController = async () => {
		if (!connected) {
			console.error("No controller connected");
			return;
		}

		setSyncing(true);
		try {
			const rgb = hexToRgb(ledColor);

			// Send LED color
			await invoke("set_led", { r: rgb.r, g: rgb.g, b: rgb.b });

			// Send player LEDs
			await invoke("set_player_leds", { player: playerNumber });

			// Send trigger effects
			await invoke("set_l2_trigger", { config: l2Effect });
			await invoke("set_r2_trigger", { config: r2Effect });

			setLastSyncTime(new Date());
			setHasUnsavedChanges(false);
		} catch (e) {
			console.error("Failed to sync to controller:", e);
		} finally {
			setSyncing(false);
		}
	};

	// Rumble is sent immediately (for testing)
	const handleRumbleChange = (val: number, side: 'left' | 'right') => {
		if (side === 'left') setRumbleLeft(val);
		else setRumbleRight(val);

		invoke("set_rumble", {
			left: side === 'left' ? val : rumbleLeft,
			right: side === 'right' ? val : rumbleRight,
			duration_ms: 100
		});
	};

	// Save current settings as a new profile
	const handleSaveAsProfile = async () => {
		if (!newProfileName.trim()) return;

		try {
			const rgb = hexToRgb(ledColor);
			const profile: Profile = {
				name: newProfileName,
				description: newProfileDesc || "Custom profile",
				led_color: rgb,
				lightbar_enabled: true,
				l2_trigger: l2Effect,
				r2_trigger: r2Effect,
				player_leds: playerNumber,
				rumble_intensity: 255,
			};

			await invoke("save_profile", { profile });

			// Refresh profiles list
			const list = await invoke<ProfileInfo[]>("list_profiles");
			setProfiles(list);

			setSaveModalOpen(false);
			setNewProfileName("");
			setNewProfileDesc("");
			setHasUnsavedChanges(false);
		} catch (e) {
			console.error("Failed to save profile:", e);
		}
	};

	const handleInitProfiles = async () => {
		try {
			await invoke("init_default_profiles");
			const list = await invoke<ProfileInfo[]>("list_profiles");
			setProfiles(list);
		} catch (e) {
			console.error("Failed to init profiles:", e);
		}
	};

	const inputFeatures = features.filter(f => f.category === "input");
	const outputFeatures = features.filter(f => f.category === "output");

	return (
		<AppShell padding="md" header={{ height: 60 }}>
			<AppShell.Header p="md">
				<Group justify="space-between">
					<Title order={3} c="blue">DualSense CMD</Title>
					{!isTauri && <Badge color="red">Running in Browser</Badge>}
					{isTauri && (
						<Group>
							<Badge color={connected ? "green" : "red"} variant="filled">
								{connected ? "Connected" : "Disconnected"}
							</Badge>
							{state?.battery && (
								<Badge color={state.battery.charging ? "orange" : "blue"} variant="outline">
									{Math.min((state.battery.level || 0) * 10, 100)}% {state.battery.charging ? "⚡" : "🔋"}
								</Badge>
							)}
						</Group>
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
							</Stack>
						</Paper>
					</Container>
				)}

				{isTauri && (
					<Tabs value={activeTab} onChange={setActiveTab}>
						<Tabs.List mb="md">
							<Tabs.Tab value="monitor">Monitor</Tabs.Tab>
							<Tabs.Tab value="controls">
								Controls {hasUnsavedChanges && <Badge size="xs" color="orange" ml="xs">•</Badge>}
							</Tabs.Tab>
							<Tabs.Tab value="features">Features</Tabs.Tab>
						</Tabs.List>

						<Tabs.Panel value="monitor">
							<Grid>
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

										{state && (
											<Paper p="md" withBorder>
												<Title order={5} mb="sm">Input Monitoring</Title>
												<Grid mb="md">
													<Grid.Col span={6}>
														<Text size="sm" fw={500}>Sticks</Text>
														<Group mt="xs" gap="xl">
															<Stack gap={0}>
																<Text size="xs" c="dimmed">Left</Text>
																<Text size="sm">X: {((state.left_stick.x - 128) / 127).toFixed(2)}</Text>
																<Text size="sm">Y: {((state.left_stick.y - 128) / 127).toFixed(2)}</Text>
															</Stack>
															<Stack gap={0}>
																<Text size="xs" c="dimmed">Right</Text>
																<Text size="sm">X: {((state.right_stick.x - 128) / 127).toFixed(2)}</Text>
																<Text size="sm">Y: {((state.right_stick.y - 128) / 127).toFixed(2)}</Text>
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
													<Grid.Col span={12}>
														<Divider my="sm" />
														<Text size="sm" fw={500} mb="xs">Touchpad Multi-touch</Text>
														<Group align="flex-start" gap="xl">
															<Box style={{
																width: 200,
																height: 100,
																border: '1px solid #333',
																position: 'relative',
																backgroundColor: 'rgba(0,0,0,0.1)',
																borderRadius: '4px'
															}}>
																{state.touchpad.finger1.active && (
																	<Box style={{
																		position: 'absolute',
																		left: `${(state.touchpad.finger1.x / 1920) * 100}%`,
																		top: `${(state.touchpad.finger1.y / 1080) * 100}%`,
																		width: 12,
																		height: 12,
																		borderRadius: '50%',
																		border: '1px solid black',
																		backgroundColor: 'transparent',
																		transform: 'translate(-50%, -50%)',
																		zIndex: 2
																	}} />
																)}
																{state.touchpad.finger2.active && (
																	<Box style={{
																		position: 'absolute',
																		left: `${(state.touchpad.finger2.x / 1920) * 100}%`,
																		top: `${(state.touchpad.finger2.y / 1080) * 100}%`,
																		width: 12,
																		height: 12,
																		borderRadius: '50%',
																		border: '1px solid black',
																		backgroundColor: 'transparent',
																		transform: 'translate(-50%, -50%)',
																		zIndex: 2
																	}} />
																)}
															</Box>
															<Stack gap={0}>
																<Text size="xs" c="dimmed" fw={700}>Finger 1</Text>
																<Text size="xs">Active: {state.touchpad.finger1.active ? "Yes" : "No"}</Text>
																<Text size="xs">X: {state.touchpad.finger1.x}</Text>
																<Text size="xs">Y: {state.touchpad.finger1.y}</Text>
																<Text size="xs" c="dimmed" fw={700} mt="xs">Finger 2</Text>
																<Text size="xs">Active: {state.touchpad.finger2.active ? "Yes" : "No"}</Text>
																<Text size="xs">X: {state.touchpad.finger2.x}</Text>
																<Text size="xs">Y: {state.touchpad.finger2.y}</Text>
															</Stack>
														</Group>
													</Grid.Col>
												</Grid>

												<Divider my="sm" label="Buttons" labelPosition="center" />
												<Group gap="xs">
													{Object.entries(state.buttons).map(([key, val]) => (
														<Badge
															key={key}
															color={val ? "blue" : "gray"}
															variant={val ? "filled" : "outline"}
															size="sm"
														>
															{key === 'touchpad' ? 'touchpad click' : key}
														</Badge>
													))}
												</Group>
											</Paper>
										)}
									</Stack>

								</Grid.Col>
								<Grid.Col span={8}>
									<Paper p="md" withBorder h={600} style={{ position: 'relative', overflow: 'hidden' }}>
										<Group justify="space-between" mb="sm" style={{ position: 'absolute', zIndex: 10, width: 'calc(100% - 32px)' }}>
											<Title order={5}>Spatial Monitor</Title>
											<Button size="compact-xs" color="gray" variant="light"
												onClick={handleResetSpatial}>
												Reset State
											</Button>
										</Group>

										<Box style={{ position: 'absolute', bottom: 16, right: 16, zIndex: 10, width: 220 }}>
											<SpatialInfo spatial={spatial} />
										</Box>

										<Box style={{ position: 'absolute', bottom: 16, left: 16, zIndex: 10, width: 300 }}>
											<Select
												label="Spatial Mode"
												placeholder="Select mode"
												data={SPATIAL_MODES}
												value={spatial.mode}
												onChange={handleSetSpatialMode}
												size="xs"
											/>
										</Box>

										<Canvas camera={{ position: [0, 2, 5] }}>
											<ambientLight intensity={0.5} />
											<pointLight position={[10, 10, 10]} />
											<ControllerModel spatial={spatial} />
											<ThreeGrid infiniteGrid />
											<OrbitControls ref={controlsRef} />
										</Canvas>
									</Paper>
								</Grid.Col>
							</Grid>
						</Tabs.Panel>

						<Tabs.Panel value="controls">
							<Grid>
								{/* Left column: Profile selector + LED/Player LEDs */}
								<Grid.Col span={4}>
									<Stack>
										{/* Profile Selector */}
										<Paper p="md" withBorder>
											<Group justify="space-between" mb="sm">
												<Title order={5}>Profile</Title>
												{profiles.length === 0 && (
													<Button size="compact-xs" variant="subtle" onClick={handleInitProfiles}>
														Init Defaults
													</Button>
												)}
											</Group>

											{profiles.length === 0 ? (
												<Text size="sm" c="dimmed">No profiles. Click "Init Defaults" to create presets.</Text>
											) : (
												<Select
													placeholder="Select a profile..."
													data={profiles.map(p => ({ value: p.id, label: p.name }))}
													value={selectedProfileId}
													onChange={handleSelectProfile}
													clearable
												/>
											)}

											{selectedProfileId && (
												<Text size="xs" c="dimmed" mt="xs">
													{profiles.find(p => p.id === selectedProfileId)?.description}
												</Text>
											)}
										</Paper>

										{/* LED Color */}
										<Paper p="md" withBorder>
											<Title order={5} mb="sm">Light Bar</Title>
											<ColorPicker
												value={ledColor}
												onChange={(c) => { setLedColor(c); handleSettingChange(); }}
												format="hex"
												fullWidth
											/>
											<Text size="xs" c="dimmed" mt="xs" ta="center">{ledColor}</Text>
										</Paper>

										{/* Player LEDs */}
										<Paper p="md" withBorder>
											<Title order={5} mb="sm">Player LEDs</Title>
											<SegmentedControl
												value={String(playerNumber)}
												onChange={(v) => { setPlayerNumber(Number(v)); handleSettingChange(); }}
												data={["1", "2", "3", "4", "5"]}
												fullWidth
											/>
											<Text size="xs" c="dimmed" mt="xs" ta="center">
												Indicator pattern for player {playerNumber}
											</Text>
										</Paper>

										{/* Rumble Test (immediate, not saved) */}
										<Paper p="md" withBorder>
											<Title order={5} mb="sm">Rumble Test</Title>
											<Text size="xs" c="dimmed" mb="sm">Immediate feedback (not saved to profile)</Text>
											<Stack gap="xs">
												<Box>
													<Text size="xs">Left Motor: {rumbleLeft}</Text>
													<Slider
														value={rumbleLeft}
														onChange={(v) => handleRumbleChange(v, 'left')}
														max={255}
														size="sm"
													/>
												</Box>
												<Box>
													<Text size="xs">Right Motor: {rumbleRight}</Text>
													<Slider
														value={rumbleRight}
														onChange={(v) => handleRumbleChange(v, 'right')}
														max={255}
														size="sm"
													/>
												</Box>
											</Stack>
										</Paper>
									</Stack>
								</Grid.Col>

								{/* Right column: Adaptive Triggers + Actions */}
								<Grid.Col span={8}>
									<Stack>
										{/* Adaptive Triggers */}
										<Paper p="md" withBorder>
											<Title order={5} mb="md">Adaptive Triggers</Title>
											<Grid>
												<Grid.Col span={6}>
													<Text size="sm" fw={500} mb="sm">L2 Trigger</Text>
													<Stack gap="sm">
														<Select
															label="Effect"
															size="sm"
															data={TRIGGER_EFFECTS}
															value={l2Effect.effect_type}
															onChange={(v) => { setL2Effect({ ...l2Effect, effect_type: v || "off" }); handleSettingChange(); }}
														/>
														<NumberInput
															label="Force"
															size="sm"
															value={l2Effect.force}
															onChange={(v) => { setL2Effect({ ...l2Effect, force: Number(v) }); handleSettingChange(); }}
															min={0}
															max={255}
														/>
														<Group grow>
															<NumberInput
																label="Start"
																size="sm"
																value={l2Effect.start}
																onChange={(v) => { setL2Effect({ ...l2Effect, start: Number(v) }); handleSettingChange(); }}
																min={0}
																max={255}
															/>
															<NumberInput
																label="End"
																size="sm"
																value={l2Effect.end}
																onChange={(v) => { setL2Effect({ ...l2Effect, end: Number(v) }); handleSettingChange(); }}
																min={0}
																max={255}
															/>
														</Group>
													</Stack>
												</Grid.Col>
												<Grid.Col span={6}>
													<Text size="sm" fw={500} mb="sm">R2 Trigger</Text>
													<Stack gap="sm">
														<Select
															label="Effect"
															size="sm"
															data={TRIGGER_EFFECTS}
															value={r2Effect.effect_type}
															onChange={(v) => { setR2Effect({ ...r2Effect, effect_type: v || "off" }); handleSettingChange(); }}
														/>
														<NumberInput
															label="Force"
															size="sm"
															value={r2Effect.force}
															onChange={(v) => { setR2Effect({ ...r2Effect, force: Number(v) }); handleSettingChange(); }}
															min={0}
															max={255}
														/>
														<Group grow>
															<NumberInput
																label="Start"
																size="sm"
																value={r2Effect.start}
																onChange={(v) => { setR2Effect({ ...r2Effect, start: Number(v) }); handleSettingChange(); }}
																min={0}
																max={255}
															/>
															<NumberInput
																label="End"
																size="sm"
																value={r2Effect.end}
																onChange={(v) => { setR2Effect({ ...r2Effect, end: Number(v) }); handleSettingChange(); }}
																min={0}
																max={255}
															/>
														</Group>
													</Stack>
												</Grid.Col>
											</Grid>
										</Paper>

										{/* Sync Actions */}
										<Paper p="md" withBorder>
											<Group justify="space-between" align="center">
												<Box>
													<Title order={5}>Apply to Controller</Title>
													<Text size="xs" c="dimmed">
														{lastSyncTime
															? `Last synced: ${lastSyncTime.toLocaleTimeString()}`
															: "Send current settings to controller via Bluetooth/USB"
														}
													</Text>
												</Box>
												<Group>
													<Button
														variant="light"
														onClick={() => setSaveModalOpen(true)}
														disabled={!hasUnsavedChanges}
													>
														Save as Profile
													</Button>
													<Button
														onClick={handleSyncToController}
														loading={syncing}
														disabled={!connected}
														color={hasUnsavedChanges ? "blue" : "gray"}
													>
														{syncing ? "Syncing..." : "Sync to Controller"}
													</Button>
												</Group>
											</Group>

											{!connected && (
												<Text size="xs" c="red" mt="sm">
													Connect a controller first to sync settings.
												</Text>
											)}
										</Paper>

										{/* Info */}
										<Paper p="md" withBorder bg="dark.8">
											<Text size="sm" c="dimmed">
												<strong>How it works:</strong> Settings are sent to the controller as a single HID output report
												containing LED color, player LEDs, and adaptive trigger effects. On Bluetooth, the report includes
												a CRC32 checksum for validation.
											</Text>
											<Text size="xs" c="dimmed" mt="sm">
												Note: Bluetooth output may require "identifying" the controller through macOS System Settings first.
											</Text>
										</Paper>
									</Stack>
								</Grid.Col>
							</Grid>
						</Tabs.Panel>

						<Tabs.Panel value="features">
							<Grid>
								<Grid.Col span={6}>
									<Paper p="md" withBorder>
										<Title order={5} mb="md">Input Features</Title>
										<Stack gap="xs">
											{inputFeatures.map(f => (
												<Group key={f.name} justify="space-between">
													<Box>
														<Text size="sm" fw={500}>{f.name}</Text>
														<Text size="xs" c="dimmed">{f.description}</Text>
													</Box>
													<Badge
														color={f.status === "implemented" ? "green" : f.status === "partial" ? "yellow" : "gray"}
														variant="filled"
														size="sm"
													>
														{f.status === "implemented" ? "✓" : f.status === "partial" ? "◐" : "○"}
													</Badge>
												</Group>
											))}
										</Stack>
									</Paper>
								</Grid.Col>
								<Grid.Col span={6}>
									<Paper p="md" withBorder>
										<Title order={5} mb="md">Output Features</Title>
										<Stack gap="xs">
											{outputFeatures.map(f => (
												<Group key={f.name} justify="space-between">
													<Box>
														<Text size="sm" fw={500}>{f.name}</Text>
														<Text size="xs" c="dimmed">{f.description}</Text>
													</Box>
													<Badge
														color={f.status === "implemented" ? "green" : f.status === "partial" ? "yellow" : "gray"}
														variant="filled"
														size="sm"
													>
														{f.status === "implemented" ? "✓" : f.status === "partial" ? "◐" : "○"}
													</Badge>
												</Group>
											))}
										</Stack>
									</Paper>
								</Grid.Col>
							</Grid>
							<Paper p="md" withBorder mt="md">
								<Title order={5} mb="sm">Legend</Title>
								<Group>
									<Badge color="green" variant="filled" size="sm">✓ Implemented</Badge>
									<Badge color="yellow" variant="filled" size="sm">◐ Partial</Badge>
									<Badge color="gray" variant="filled" size="sm">○ Future/OS-level</Badge>
								</Group>
								<Text size="sm" c="dimmed" mt="md">
									Bluetooth output features (LED, triggers, rumble) may require "identifying" the controller
									through System Settings on macOS before they work properly.
								</Text>
							</Paper>
						</Tabs.Panel>
					</Tabs>
				)}

				{/* Save Profile Modal */}
				<Modal
					opened={saveModalOpen}
					onClose={() => setSaveModalOpen(false)}
					title="Save as Profile"
				>
					<Stack>
						<TextInput
							label="Profile Name"
							placeholder="My Custom Profile"
							value={newProfileName}
							onChange={(e) => setNewProfileName(e.currentTarget.value)}
							required
						/>
						<TextInput
							label="Description"
							placeholder="Optional description"
							value={newProfileDesc}
							onChange={(e) => setNewProfileDesc(e.currentTarget.value)}
						/>
						<Text size="xs" c="dimmed">
							Current settings will be saved: LED {ledColor}, Player {playerNumber},
							L2 {l2Effect.effect_type}, R2 {r2Effect.effect_type}
						</Text>
						<Group justify="flex-end" mt="md">
							<Button variant="subtle" onClick={() => setSaveModalOpen(false)}>Cancel</Button>
							<Button onClick={handleSaveAsProfile} disabled={!newProfileName.trim()}>
								Save Profile
							</Button>
						</Group>
					</Stack>
				</Modal>
			</AppShell.Main>
		</AppShell>
	);
}

export default App;
