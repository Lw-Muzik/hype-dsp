# src/features/devices/DevicesView.tsx

- Thumb · function · L45-L55 — function Thumb({ seed, label }: { seed: string; label: string })
- fetchArt · function · L62-L75 — function fetchArt(deviceId: string, trackId: string): Promise<string | null>
- PhoneCover · function · L78-L105 — function PhoneCover({ deviceId, track }: { deviceId: string; track: PhoneTrack })
- DevicesView · function · L110-L661 — function DevicesView()
- onSelect · function · L214-L222 — onSelect = (device: PhoneDevice)
- submitPin · function · L224-L247 — submitPin = async ()
- connectByAddress · function · L249-L279 — connectByAddress = async ()
- unpair · function · L281-L289 — unpair = async (device: PhoneDevice)
- startRemotePairing · function · L301-L311 — startRemotePairing = async ()
- stopRemotePairing · function · L313-L316 — stopRemotePairing = async ()
- forgetRemote · function · L318-L327 — forgetRemote = async (id: string)
- back · function · L350-L354 — back = ()
- PinForm · function · L664-L707 — function PinForm({ device, pin, busy, onPin, onSubmit, onCancel, }: { device: PhoneDevice; pin: string; busy: boolean; onPin: (v: string) => void; onSubmit: () => void; onCancel: () => void; })
- DeviceLibrary · function · L710-L805 — function DeviceLibrary({ device, tracks, loading, nowPlaying, onBack, onPlay, banner, }: { device: PhoneDevice; tracks: PhoneTrack[]; loading: boolean; nowPlaying: string | null; onBack: () => void; onPlay: (track: PhoneTrack) => void; banner: React.ReactNode; })
