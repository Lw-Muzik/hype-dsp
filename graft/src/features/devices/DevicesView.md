# src/features/devices/DevicesView.tsx

- Thumb · function · L44-L54 — function Thumb({ seed, label }: { seed: string; label: string })
- fetchArt · function · L61-L74 — function fetchArt(deviceId: string, trackId: string): Promise<string | null>
- PhoneCover · function · L77-L104 — function PhoneCover({ deviceId, track }: { deviceId: string; track: PhoneTrack })
- DevicesView · function · L109-L659 — function DevicesView()
- onSelect · function · L213-L221 — onSelect = (device: PhoneDevice)
- submitPin · function · L223-L246 — submitPin = async ()
- connectByAddress · function · L248-L278 — connectByAddress = async ()
- unpair · function · L280-L288 — unpair = async (device: PhoneDevice)
- startRemotePairing · function · L300-L310 — startRemotePairing = async ()
- stopRemotePairing · function · L312-L315 — stopRemotePairing = async ()
- forgetRemote · function · L317-L326 — forgetRemote = async (id: string)
- back · function · L349-L353 — back = ()
- PinForm · function · L662-L705 — function PinForm({ device, pin, busy, onPin, onSubmit, onCancel, }: { device: PhoneDevice; pin: string; busy: boolean; onPin: (v: string) => void; onSubmit: () => void; onCancel: () => void; })
- DeviceLibrary · function · L708-L803 — function DeviceLibrary({ device, tracks, loading, nowPlaying, onBack, onPlay, banner, }: { device: PhoneDevice; tracks: PhoneTrack[]; loading: boolean; nowPlaying: string | null; onBack: () => void; onPlay: (track: PhoneTrack) => void; banner: React.ReactNode; })
