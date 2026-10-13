# src/features/devices/DevicesView.tsx

- Thumb · function · L43-L53 — function Thumb({ seed, label }: { seed: string; label: string })
- fetchArt · function · L60-L73 — function fetchArt(deviceId: string, trackId: string): Promise<string | null>
- PhoneCover · function · L76-L103 — function PhoneCover({ deviceId, track }: { deviceId: string; track: PhoneTrack })
- DevicesView · function · L108-L656 — function DevicesView()
- onSelect · function · L212-L220 — onSelect = (device: PhoneDevice)
- submitPin · function · L222-L245 — submitPin = async ()
- connectByAddress · function · L247-L277 — connectByAddress = async ()
- unpair · function · L279-L287 — unpair = async (device: PhoneDevice)
- startRemotePairing · function · L299-L309 — startRemotePairing = async ()
- stopRemotePairing · function · L311-L314 — stopRemotePairing = async ()
- forgetRemote · function · L316-L325 — forgetRemote = async (id: string)
- back · function · L348-L352 — back = ()
- PinForm · function · L659-L702 — function PinForm({ device, pin, busy, onPin, onSubmit, onCancel, }: { device: PhoneDevice; pin: string; busy: boolean; onPin: (v: string) => void; onSubmit: () => void; onCancel: () => void; })
- DeviceLibrary · function · L705-L800 — function DeviceLibrary({ device, tracks, loading, nowPlaying, onBack, onPlay, banner, }: { device: PhoneDevice; tracks: PhoneTrack[]; loading: boolean; nowPlaying: string | null; onBack: () => void; onPlay: (track: PhoneTrack) => void; banner: React.ReactNode; })
