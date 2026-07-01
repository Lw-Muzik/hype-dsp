; HypeMuzik — NSIS installer hooks for the bundled virtual-audio driver.
;
; Tauri's NSIS template invokes these macros at the matching points of its
; install/uninstall script. The installer runs elevated (per-machine), so
; `pnputil` has the administrator rights it needs to stage and install a driver.
;
; This is the PRIMARY install path (zero user setup at app-install time). The
; in-app "Install audio driver" button (system_audio_install_driver) is the
; runtime fallback/repair, and uses Tauri's resource_dir() so it is always
; layout-correct.
;
; The signed driver package (HypeMuzikAudio.inf + .sys + .cat) is produced and
; signed on Windows per docs/windows-driver.md and dropped into
; src-tauri/drivers/HypeMuzikAudio/ so it is bundled as a resource. Until then the
; folder holds only a README and the POSTINSTALL hook safely no-ops.
;
; VERIFY ON WINDOWS: confirm the bundled-resource path below matches your Tauri
; version's layout (it places `resources` map entries under $INSTDIR). Adjust the
; StrCpy path if a build shows the .inf elsewhere.

!macro NSIS_HOOK_POSTINSTALL
  ; Find the bundled driver .inf by glob (its filename is whatever the signed
  ; upstream package uses — we don't rename it, which would break the .cat).
  FindFirst $R2 $R3 "$INSTDIR\drivers\HypeMuzikAudio\*.inf"
  StrCmp $R3 "" driver_skip
    StrCpy $R0 "$INSTDIR\drivers\HypeMuzikAudio\$R3"
    DetailPrint "Installing the HypeMuzik virtual audio driver ($R3)..."
    nsExec::ExecToLog 'pnputil /add-driver "$R0" /install'
    Pop $R1
    DetailPrint "pnputil /add-driver returned $R1"
    FindClose $R2
    Goto driver_done
  driver_skip:
    FindClose $R2
    DetailPrint "HypeMuzik audio driver not bundled in this build; system-wide EQ can be enabled later from Settings."
  driver_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Best-effort removal: find the published OEM inf by its original name and
  ; delete+uninstall it. VERIFY ON WINDOWS (pnputil output format is locale- and
  ; version-dependent; Get-WindowsDriver requires the DISM PowerShell module).
  DetailPrint "Removing the HypeMuzik virtual audio driver..."
  nsExec::ExecToLog 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Get-WindowsDriver -Online | Where-Object { $$_.OriginalFileName -like \"*HypeMuzikAudio.inf\" } | ForEach-Object { pnputil /delete-driver $$_.Driver /uninstall /force }"'
  Pop $R1
  DetailPrint "driver removal returned $R1"
!macroend
